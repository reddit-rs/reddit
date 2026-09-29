"use strict";

const path = require("node:path");
const { spawnSync } = require("node:child_process");
const platforms = require("../platforms.cjs");

function resolveBinary(platform, arch, resolve = require.resolve) {
  const key = `${platform}-${arch}`;
  if (!Object.hasOwn(platforms, key)) {
    throw new Error(
      `Unsupported platform: ${key}. See https://github.com/reddit-rs/reddit#installation`,
    );
  }
  const packageName = `@rddt/cli-${key}`;
  let manifest;
  try {
    manifest = resolve(`${packageName}/package.json`);
  } catch (cause) {
    throw new Error(
      `Missing ${packageName}. Reinstall without --omit=optional. Linux requires glibc 2.35 or newer.`,
      { cause },
    );
  }
  return path.join(
    path.dirname(manifest),
    "bin",
    platform === "win32" ? "reddit.exe" : "reddit",
  );
}

function execute(args, options = {}) {
  try {
    const binary = resolveBinary(
      options.platform ?? process.platform,
      options.arch ?? process.arch,
      options.resolve,
    );
    return (options.spawnSync ?? spawnSync)(binary, args, {
      stdio: "inherit",
      windowsHide: false,
    });
  } catch (error) {
    return { error };
  }
}

module.exports = { resolveBinary, execute };
