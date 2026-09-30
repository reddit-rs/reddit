#!/usr/bin/env node
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const require = createRequire(import.meta.url);
const platforms = require("../npm/platforms.cjs");
const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
const readJson = (file) => JSON.parse(fs.readFileSync(file, "utf8"));
const sha256 = (file) =>
  createHash("sha256").update(fs.readFileSync(file)).digest("hex");

export function verifyVersion(tag) {
  const cargo = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
  const version = cargo.match(/^version = "([^"]+)"$/m)?.[1];
  assert.match(
    version ?? "",
    /^\d+\.\d+\.\d+$/,
    "Releases require a stable semantic version",
  );
  const manifest = readJson(path.join(root, "npm/package.json"));
  assert.equal(manifest.version, version, "Cargo and npm versions differ");
  const expected = Object.fromEntries(
    Object.keys(platforms).map((key) => [`@rddt/cli-${key}`, version]),
  );
  assert.deepEqual(
    manifest.optionalDependencies,
    expected,
    "Native package versions differ",
  );
  if (tag)
    assert.equal(tag, `v${version}`, "Tag does not match package version");
  return version;
}

function platformFor(target) {
  const entry = Object.entries(platforms).find(([, value]) => value === target);
  assert.ok(entry, `Unsupported target: ${target}`);
  return entry[0];
}

export function assetName(version, target) {
  platformFor(target);
  assert.match(version, /^\d+\.\d+\.\d+$/);
  return `reddit-v${version}-${target}.tar.gz`;
}

export function verifyChecksum(directory, name) {
  assert.equal(path.basename(name), name, "Asset must be a filename");
  const record = fs
    .readFileSync(path.join(directory, `${name}.sha256`), "utf8")
    .trim();
  const digest = sha256(path.join(directory, name));
  assert.equal(record, `${digest}  ${name}`, `Checksum mismatch: ${name}`);
  return digest;
}

function binaryName(target) {
  return target.includes("windows") ? "reddit.exe" : "reddit";
}

function nativeManifest(key, version) {
  const [os, cpu] = key.split("-");
  return {
    name: `@rddt/cli-${key}`,
    version,
    description: `Native reddit binary for ${key}`,
    license: "MIT OR Apache-2.0",
    repository: {
      type: "git",
      url: "git+https://github.com/reddit-rs/reddit.git",
    },
    os: [os],
    cpu: [cpu],
    ...(os === "linux" ? { libc: ["glibc"] } : {}),
    files: ["bin", "README.md", "LICENSE-MIT", "LICENSE-APACHE"],
    publishConfig: { access: "public" },
  };
}

function writeJson(file, value) {
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

function archive(target, binary) {
  const version = verifyVersion();
  const name = assetName(version, target);
  const output = path.join(root, "dist");
  const staging = path.join(output, `archive-${target}`);
  fs.mkdirSync(staging, { recursive: true });
  const source =
    binary ?? path.join(root, "target", target, "release", binaryName(target));
  assert.equal(
    execFileSync(source, ["--version"], { encoding: "utf8" }).trim(),
    `reddit ${version}`,
  );
  fs.copyFileSync(source, path.join(staging, binaryName(target)));
  fs.chmodSync(path.join(staging, binaryName(target)), 0o755);
  const files = [
    binaryName(target),
    "README.md",
    "LICENSE-MIT",
    "LICENSE-APACHE",
  ];
  for (const file of files.slice(1))
    fs.copyFileSync(path.join(root, file), path.join(staging, file));
  for (const file of files) {
    fs.chmodSync(
      path.join(staging, file),
      file === binaryName(target) ? 0o755 : 0o644,
    );
    fs.utimesSync(path.join(staging, file), 0, 0);
  }
  createTarArchive(staging, name, files);
  fs.writeFileSync(
    path.join(output, `${name}.sha256`),
    `${sha256(path.join(output, name))}  ${name}\n`,
  );
  fs.rmSync(staging, { recursive: true });
}

export function createTarArchive(staging, name, files) {
  assert.equal(path.basename(name), name, "Archive must be a filename");
  const bsdTar = /bsdtar|libarchive/i.test(
    execFileSync("tar", ["--version"], { encoding: "utf8" }),
  );
  const ownership = bsdTar
    ? ["--uid=0", "--gid=0", "--uname=", "--gname="]
    : ["--owner=0", "--group=0", "--numeric-owner"];
  // GNU tar interprets a Windows drive prefix in -f as a remote host. Keep
  // every CLI path relative; child_process handles the absolute working dir.
  execFileSync(
    "tar",
    ["--format=ustar", ...ownership, "-czf", `../${name}`, ...files],
    { cwd: staging },
  );
}

export function renderFormula(version, checksums) {
  const stanza = (target, indent) => {
    const name = assetName(version, target);
    assert.match(
      checksums[target] ?? "",
      /^[a-f0-9]{64}$/,
      `Missing checksum: ${target}`,
    );
    return (
      `${indent}url "https://github.com/reddit-rs/reddit/releases/download/v${version}/${name}"\n` +
      `${indent}sha256 "${checksums[target]}"`
    );
  };
  return (
    `# Generated by scripts/release.mjs from verified release assets.\n` +
    `class Reddit < Formula\n` +
    `  desc "Archive Reddit listings as JSON and media with an offline viewer"\n` +
    `  homepage "https://github.com/reddit-rs/reddit"\n` +
    `  version "${version}"\n` +
    `  license any_of: ["MIT", "Apache-2.0"]\n\n` +
    `  on_macos do\n    depends_on macos: :sonoma\n    on_arm do\n${stanza("aarch64-apple-darwin", "      ")}\n    end\n` +
    `    on_intel do\n${stanza("x86_64-apple-darwin", "      ")}\n    end\n  end\n\n` +
    `  on_linux do\n    on_arm do\n${stanza("aarch64-unknown-linux-gnu", "      ")}\n    end\n` +
    `    on_intel do\n${stanza("x86_64-unknown-linux-gnu", "      ")}\n    end\n  end\n\n` +
    `  def install\n    bin.install "reddit"\n  end\n\n` +
    `  test do\n    assert_match "reddit #{version}", shell_output("#{bin}/reddit --version")\n  end\nend\n`
  );
}

function prepare() {
  const version = verifyVersion();
  const directory = path.join(root, "dist");
  const packages = path.join(directory, "npm");
  fs.rmSync(packages, { recursive: true, force: true });
  fs.mkdirSync(packages, { recursive: true });
  const checksums = {};
  for (const [key, target] of Object.entries(platforms)) {
    const name = assetName(version, target);
    checksums[target] = verifyChecksum(directory, name);
    const entries = execFileSync("tar", ["-tzf", name], {
      cwd: directory,
      encoding: "utf8",
    })
      .trim()
      .split(/\r?\n/)
      .sort();
    assert.deepEqual(
      entries,
      [binaryName(target), "README.md", "LICENSE-MIT", "LICENSE-APACHE"].sort(),
      `Unexpected archive contents: ${name}`,
    );
    const extracted = path.join(directory, `extracted-${target}`);
    fs.mkdirSync(extracted, { recursive: true });
    execFileSync("tar", ["-xzf", name, "-C", path.basename(extracted)], {
      cwd: directory,
    });
    const pkg = path.join(packages, `cli-${key}`);
    fs.mkdirSync(path.join(pkg, "bin"), { recursive: true });
    const binary = path.join(pkg, "bin", binaryName(target));
    fs.copyFileSync(path.join(extracted, binaryName(target)), binary);
    assert.ok(fs.statSync(binary).size > 0, `Empty native binary: ${target}`);
    fs.chmodSync(binary, 0o755);
    for (const license of ["LICENSE-MIT", "LICENSE-APACHE"])
      fs.copyFileSync(path.join(extracted, license), path.join(pkg, license));
    fs.writeFileSync(
      path.join(pkg, "README.md"),
      `# @rddt/cli-${key}\n\nNative binary for [@rddt/cli](https://www.npmjs.com/package/@rddt/cli). Install the launcher, not this package directly.\n`,
    );
    writeJson(path.join(pkg, "package.json"), nativeManifest(key, version));
    fs.rmSync(extracted, { recursive: true });
  }
  const launcher = path.join(packages, "cli");
  fs.mkdirSync(launcher, { recursive: true });
  for (const file of [
    "package.json",
    "platforms.cjs",
    "README.md",
    "bin",
    "lib",
  ])
    fs.cpSync(path.join(root, "npm", file), path.join(launcher, file), {
      recursive: true,
    });
  for (const license of ["LICENSE-MIT", "LICENSE-APACHE"])
    fs.copyFileSync(path.join(root, license), path.join(launcher, license));
  fs.chmodSync(path.join(launcher, "bin/rddt.cjs"), 0o755);
  fs.writeFileSync(
    path.join(directory, "SHA256SUMS"),
    Object.entries(checksums)
      .map(([target, hash]) => `${hash}  ${assetName(version, target)}\n`)
      .sort()
      .join(""),
  );
  fs.writeFileSync(
    path.join(directory, "reddit.rb"),
    renderFormula(version, checksums),
  );
}

function smoke(target, binary) {
  const version = verifyVersion();
  const key = platformFor(target);
  assert.equal(
    key,
    `${process.platform}-${process.arch}`,
    "Smoke tests must run on the native platform",
  );
  const pkg = path.join(root, "npm/node_modules/@rddt", `cli-${key}`);
  fs.mkdirSync(path.join(pkg, "bin"), { recursive: true });
  writeJson(path.join(pkg, "package.json"), nativeManifest(key, version));
  fs.copyFileSync(
    binary ?? path.join(root, "target", target, "release", binaryName(target)),
    path.join(pkg, "bin", binaryName(target)),
  );
  fs.chmodSync(path.join(pkg, "bin", binaryName(target)), 0o755);
  try {
    const wrapper = path.join(root, "npm/bin/rddt.cjs");
    assert.equal(
      execFileSync(process.execPath, [wrapper, "--version"], {
        encoding: "utf8",
      }).trim(),
      `reddit ${version}`,
    );
    execFileSync(process.execPath, [wrapper, "--help"], { stdio: "ignore" });
    const invalid = spawnSync(process.execPath, [wrapper, "--not-an-option"], {
      encoding: "utf8",
    });
    assert.equal(
      invalid.status,
      2,
      "Launcher must preserve clap's failure exit status",
    );
  } finally {
    fs.rmSync(pkg, { recursive: true, force: true });
  }
}

function publish() {
  assert.ok(process.env.NODE_AUTH_TOKEN, "NODE_AUTH_TOKEN is required");
  const directory = path.join(root, "dist/npm");
  // Publish native packages before the launcher; reruns verify existing integrity.
  for (const name of [
    ...Object.keys(platforms).map((key) => `cli-${key}`),
    "cli",
  ]) {
    const cwd = path.join(directory, name);
    const manifest = readJson(path.join(cwd, "package.json"));
    const [packed] = JSON.parse(
      execFileSync(npmCommand, ["pack", "--json"], { cwd, encoding: "utf8" }),
    );
    const existing = spawnSync(
      npmCommand,
      [
        "view",
        `${manifest.name}@${manifest.version}`,
        "dist.integrity",
        "--json",
      ],
      { encoding: "utf8" },
    );
    if (existing.status === 0) {
      assert.equal(
        registryIntegrity(JSON.parse(existing.stdout)),
        packed.integrity,
        `Published contents differ: ${manifest.name}`,
      );
      console.log(`Already published: ${manifest.name}@${manifest.version}`);
    } else {
      assert.match(
        existing.stderr,
        /E404/,
        `Cannot query npm: ${existing.stderr}`,
      );
      execFileSync(
        npmCommand,
        ["publish", packed.filename, "--access", "public", "--provenance"],
        { cwd, stdio: "inherit" },
      );
    }
  }
}

export function registryIntegrity(value) {
  if (Array.isArray(value)) {
    assert.equal(
      value.length,
      1,
      "Expected one exact-version npm integrity value",
    );
    [value] = value;
  }
  assert.ok(
    typeof value === "string" && /^sha512-[A-Za-z0-9+/]+={0,2}$/.test(value),
    "Invalid npm integrity value",
  );
  return value;
}

export async function verifyInstalledCli({
  attempts = 12,
  delayMs = 10000,
  run = spawnSync,
  wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
} = {}) {
  assert.ok(
    Number.isInteger(attempts) && attempts > 0 && delayMs >= 0,
    "Invalid npm retry settings",
  );
  const version = verifyVersion();
  const temporary = fs.mkdtempSync(
    path.join(process.env.RUNNER_TEMP ?? os.tmpdir(), "reddit-npm-smoke-"),
  );
  const config = path.join(temporary, "public.npmrc");
  fs.writeFileSync(config, "registry=https://registry.npmjs.org/\n");
  const env = {
    ...process.env,
    NPM_CONFIG_USERCONFIG: config,
    NPM_CONFIG_REGISTRY: "https://registry.npmjs.org/",
    NPM_CONFIG_FETCH_RETRIES: "0",
    NPM_CONFIG_FETCH_TIMEOUT: "15000",
  };
  delete env.NODE_AUTH_TOKEN;
  try {
    for (let attempt = 1; attempt <= attempts; attempt++) {
      // A failed optional dependency install can persist in npx's cache. Each
      // retry gets a fresh cache so it verifies a complete anonymous install.
      const result = run(
        "npx",
        [
          "--yes",
          "--ignore-scripts",
          "--prefer-online",
          "--cache",
          path.join(temporary, String(attempt)),
          `@rddt/cli@${version}`,
          "--version",
        ],
        { encoding: "utf8", env, timeout: 30000 },
      );
      if (result.error && result.error.code !== "ETIMEDOUT") throw result.error;
      if (result.status === 0) {
        assert.equal(
          result.stdout.trim(),
          `reddit ${version}`,
          "Installed binary version differs",
        );
        console.log(result.stdout.trim());
        return;
      }
      const message = `${result.error?.message ?? ""}\n${result.stderr ?? ""}\n${result.stdout ?? ""}`;
      const transient =
        /E404|ETARGET|EAI_AGAIN|ECONNRESET|ETIMEDOUT|E429|E503|Missing @rddt\/cli-/.test(
          message,
        );
      if (!transient || attempt === attempts)
        throw new Error(
          `Public npm installation failed after ${attempt} attempt(s): ${message}`,
        );
      console.warn(
        `npm packages are not yet available (attempt ${attempt}/${attempts}); retrying in ${delayMs / 1000}s`,
      );
      await wait(delayMs);
    }
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const [command, ...args] = process.argv.slice(2);
  switch (command) {
    case "verify":
      console.log(verifyVersion(args[0]));
      break;
    case "archive":
      archive(...args);
      break;
    case "prepare":
      prepare();
      break;
    case "smoke":
      smoke(...args);
      break;
    case "publish":
      publish();
      break;
    case "verify-install":
      await verifyInstalledCli();
      break;
    default:
      throw new Error(
        "Usage: release.mjs verify|archive|prepare|smoke|publish|verify-install [target] [binary]",
      );
  }
}
