import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import {
  createTarArchive,
  registryIntegrity,
  verifyInstalledCli,
  assetName,
  renderFormula,
  verifyChecksum,
  verifyVersion,
} from "../release.mjs";

test("npm integrity supports scalar and singleton-array output without ambiguity", () => {
  const integrity = `sha512-${createHash("sha512").update("fixture").digest("base64")}`;
  assert.equal(registryIntegrity(integrity), integrity);
  assert.equal(registryIntegrity([integrity]), integrity);
  for (const value of [[], [integrity, integrity], {}, null, "invalid"]) {
    assert.throws(() => registryIntegrity(value));
  }
});

test("npm smoke test retries propagation with fresh, anonymous install caches", async () => {
  let calls = 0;
  let waits = 0;
  const caches = [];
  await verifyInstalledCli({
    attempts: 3,
    delayMs: 1,
    wait: async () => {
      waits++;
    },
    run: (command, args, options) => {
      assert.equal(command, "npx");
      assert.equal(options.env.NODE_AUTH_TOKEN, undefined);
      assert.equal(
        options.env.NPM_CONFIG_REGISTRY,
        "https://registry.npmjs.org/",
      );
      caches.push(args[args.indexOf("--cache") + 1]);
      calls++;
      if (calls === 1) return { status: 1, stderr: "npm error E404" };
      if (calls === 2)
        return { status: 1, stderr: "reddit: Missing @rddt/cli-linux-x64" };
      return { status: 0, stdout: `reddit ${verifyVersion()}\n` };
    },
  });
  assert.equal(waits, 2);
  assert.equal(new Set(caches).size, 3);
  assert.ok(caches.every((cache) => !fs.existsSync(path.dirname(cache))));
});

test("npm smoke test is bounded and does not retry real authorization errors", async () => {
  let calls = 0;
  await assert.rejects(
    verifyInstalledCli({
      attempts: 2,
      delayMs: 0,
      wait: async () => {},
      run: () => {
        calls++;
        return { status: 1, stderr: "E404" };
      },
    }),
    /after 2 attempt/,
  );
  assert.equal(calls, 2);
  await assert.rejects(
    verifyInstalledCli({
      run: () => ({ status: 1, stderr: "E403" }),
      wait: async () => {
        assert.fail("Must not retry authorization failures");
      },
    }),
    /after 1 attempt/,
  );
});

test("tar writes archives outside an absolute staging path, including Windows drives", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "reddit-tar-"));
  try {
    const staging = path.join(directory, "staging with spaces");
    fs.mkdirSync(staging);
    fs.writeFileSync(path.join(staging, "reddit"), "binary fixture");
    createTarArchive(staging, "archive.tar.gz", ["reddit"]);
    assert.equal(
      execFileSync("tar", ["-tzf", "archive.tar.gz"], {
        cwd: directory,
        encoding: "utf8",
      }).trim(),
      "reddit",
    );
    const extracted = path.join(directory, "extracted");
    fs.mkdirSync(extracted);
    execFileSync("tar", ["-xzf", "archive.tar.gz", "-C", "extracted"], {
      cwd: directory,
    });
    assert.equal(
      fs.readFileSync(path.join(extracted, "reddit"), "utf8"),
      "binary fixture",
    );
  } finally {
    fs.rmSync(directory, { recursive: true });
  }
});

test("release tags and package versions must agree", () => {
  const version = verifyVersion();
  assert.equal(verifyVersion(`v${version}`), version);
  assert.throws(() => verifyVersion("v99.0.0"), /Tag does not match/);
});

test("asset paths are restricted to known targets and stable versions", () => {
  assert.equal(
    assetName("0.5.0", "aarch64-apple-darwin"),
    "reddit-v0.5.0-aarch64-apple-darwin.tar.gz",
  );
  assert.throws(() => assetName("../escape", "aarch64-apple-darwin"));
  assert.throws(() => assetName("0.5.0", "../../escape"));
});

test("checksum records authenticate bytes and exact filenames", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "reddit-release-"));
  try {
    const name = "archive.tar.gz";
    fs.writeFileSync(path.join(directory, name), "archive bytes");
    const digest = createHash("sha256").update("archive bytes").digest("hex");
    fs.writeFileSync(
      path.join(directory, `${name}.sha256`),
      `${digest}  ${name}\n`,
    );
    assert.equal(verifyChecksum(directory, name), digest);
    fs.appendFileSync(path.join(directory, name), "changed");
    assert.throws(() => verifyChecksum(directory, name), /Checksum mismatch/);
    assert.throws(() => verifyChecksum(directory, "../escape"), /filename/);
  } finally {
    fs.rmSync(directory, { recursive: true });
  }
});

test("Homebrew formula pins four platform-specific binary checksums", () => {
  const targets = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
  ];
  const hashes = Object.fromEntries(
    targets.map((target, index) => [target, String(index).repeat(64)]),
  );
  const formula = renderFormula("0.5.0", hashes);
  assert.equal((formula.match(/sha256 "/g) ?? []).length, 4);
  for (const target of targets)
    assert.ok(formula.includes(assetName("0.5.0", target)));
  assert.ok(formula.includes('bin.install "reddit"'));
  assert.throws(() => renderFormula("0.5.0", {}), /Missing checksum/);
});
