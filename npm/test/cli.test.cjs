"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");
const { execute, resolveBinary } = require("../lib/cli.cjs");
const platforms = require("../platforms.cjs");
const manifest = require("../package.json");

test("npm exposes only the reddit command and its launcher exists", () => {
  const fs = require("node:fs");
  assert.deepEqual(manifest.bin, { reddit: "bin/reddit.cjs" });
  const launcher = fs.readFileSync(
    path.join(__dirname, "..", manifest.bin.reddit),
    "utf8",
  );
  assert.ok(launcher.startsWith("#!/usr/bin/env node\n"));
  assert.ok(launcher.includes("reddit: ${result.error.message}"));
});

test("every release target has an exact-version optional dependency", () => {
  assert.equal(
    Object.keys(manifest.optionalDependencies).length,
    Object.keys(platforms).length,
  );
  for (const key of Object.keys(platforms)) {
    const [platform, arch] = key.split("-");
    let requested;
    const binary = resolveBinary(platform, arch, (name) => {
      requested = name;
      return path.join("packages", key, "package.json");
    });
    assert.equal(requested, `@rddt/cli-${key}/package.json`);
    assert.equal(
      path.basename(binary),
      platform === "win32" ? "reddit.exe" : "reddit",
    );
    assert.equal(
      manifest.optionalDependencies[`@rddt/cli-${key}`],
      manifest.version,
    );
  }
});

test("unsupported platforms and missing optional packages fail clearly", () => {
  assert.throws(
    () => resolveBinary("linux", "riscv64"),
    /Unsupported platform/,
  );
  assert.throws(
    () =>
      resolveBinary("linux", "x64", () => {
        throw new Error("missing");
      }),
    /omit=optional/,
  );
});

test("arguments are forwarded unchanged without a shell", () => {
  const args = [
    "r/pics",
    "--cookies",
    "path with spaces/cookies.txt",
    "$(echo unsafe);&",
  ];
  const outcome = execute(args, {
    platform: "darwin",
    arch: "arm64",
    resolve: () => path.join("native", "package.json"),
    spawnSync: (binary, actual, options) => {
      assert.equal(binary, path.join("native", "bin", "reddit"));
      assert.deepEqual(actual, args);
      assert.deepEqual(options, { stdio: "inherit", windowsHide: false });
      return { status: 42, signal: null };
    },
  });
  assert.equal(outcome.status, 42);
});

test("spawn errors and termination signals reach the entry point", () => {
  for (const expected of [
    { error: new Error("not executable") },
    { status: null, signal: "SIGTERM" },
  ]) {
    assert.equal(
      execute([], {
        platform: "linux",
        arch: "x64",
        resolve: () => "package.json",
        spawnSync: () => expected,
      }),
      expected,
    );
  }
  assert.match(
    execute([], { platform: "unknown", arch: "x64" }).error.message,
    /Unsupported platform/,
  );
});

test("native subprocess exit codes are not replaced by launcher success", () => {
  const result = execute(["-e", "process.exit(7)"], {
    platform: "linux",
    arch: "x64",
    resolve: () => "package.json",
    spawnSync: (_, args, options) => spawnSync(process.execPath, args, options),
  });
  assert.equal(result.status, 7);
});
