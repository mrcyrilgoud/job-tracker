import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { installSymlink } from "./install-cli";

test("installs the requested CLI alias when the destination is missing", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "job-tracker-cli-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const installDir = path.join(root, "bin");
  fs.mkdirSync(installDir);
  const target = path.join(root, "target/job-tracker");

  installSymlink(target, "jt", installDir, root);

  assert.equal(fs.readlinkSync(path.join(installDir, "jt")), target);
});

test("replaces a previous Job Tracker symlink", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "job-tracker-cli-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const installDir = path.join(root, "bin");
  fs.mkdirSync(installDir);
  const oldTarget = path.join(root, "src-tauri/target/debug/job-tracker");
  const newTarget = path.join(root, "src-tauri/target/release/job-tracker");
  fs.symlinkSync(oldTarget, path.join(installDir, "jt"));

  installSymlink(newTarget, "jt", installDir, root);

  assert.equal(fs.readlinkSync(path.join(installDir, "jt")), newTarget);
});

test("preserves an unrelated symlink", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "job-tracker-cli-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const installDir = path.join(root, "bin");
  fs.mkdirSync(installDir);
  const existingPath = path.join(installDir, "jt");
  fs.symlinkSync("/usr/bin/true", existingPath);

  assert.throws(() => installSymlink("/tmp/job-tracker", "jt", installDir, root), /unrelated symlink/);
  assert.equal(fs.readlinkSync(existingPath), "/usr/bin/true");
});

test("preserves an existing regular file", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "job-tracker-cli-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const installDir = path.join(root, "bin");
  fs.mkdirSync(installDir);
  const existingPath = path.join(installDir, "jt");
  fs.writeFileSync(existingPath, "leave this command alone");

  assert.throws(() => installSymlink("/tmp/job-tracker", "jt", installDir, root), /non-symlink/);
  assert.equal(fs.readFileSync(existingPath, "utf8"), "leave this command alone");
});
