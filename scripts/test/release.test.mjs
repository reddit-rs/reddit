import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import {
  createTarArchive,
  assetName,
  renderFormula,
  verifyChecksum,
  verifyVersion,
} from "../release.mjs";

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
