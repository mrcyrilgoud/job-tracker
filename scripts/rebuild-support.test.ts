import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, test } from "node:test";

import {
  acquireDirectoryLock,
  assertCheckoutUnchanged,
  assertCleanCheckout,
  checkoutSnapshot,
  clearRebuildRequests,
  enqueueRebuild,
  pendingRebuildRequests,
  readRebuildRequest,
  releaseDirectoryLock,
} from "./rebuild-support";

const tempDirs: string[] = [];
afterEach(() => {
  for (const dir of tempDirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});

function tempDir(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "job-tracker-rebuild-"));
  tempDirs.push(dir);
  return dir;
}

test("rebuild directory lock is exclusive and released by its owner token", () => {
  const lock = path.join(tempDir(), "data", "rebuild.lock");
  const owner = acquireDirectoryLock(lock);
  assert.ok(owner);
  assert.equal(acquireDirectoryLock(lock), null);
  releaseDirectoryLock(lock, "wrong-token");
  assert.equal(acquireDirectoryLock(lock), null);
  releaseDirectoryLock(lock, owner);
  const nextOwner = acquireDirectoryLock(lock);
  assert.ok(nextOwner);
  releaseDirectoryLock(lock, nextOwner);
});

test("rebuild queue retains distinct requests and clears only consumed requests", () => {
  const queue = path.join(tempDir(), "queue");
  const first = enqueueRebuild(queue, "abc123", "main");
  const second = enqueueRebuild(queue, "def456", "feature/example");
  assert.deepEqual(pendingRebuildRequests(queue), [first, second].sort());
  const request = readRebuildRequest(first);
  assert.equal(request.commit, "abc123");
  assert.equal(request.branch, "main");
  assert.ok(Number.isFinite(Date.parse(request.requestedAt ?? "")));
  clearRebuildRequests([first]);
  assert.deepEqual(pendingRebuildRequests(queue), [second]);
});

test("checkout guard detects dirty files and a changed HEAD", () => {
  const repo = tempDir();
  const git = (args: string[]) => execFileSync("git", args, { cwd: repo, stdio: "ignore" });
  git(["init", "-q"]);
  git(["config", "user.email", "test@example.invalid"]);
  git(["config", "user.name", "Test"]);
  fs.writeFileSync(path.join(repo, "tracked.txt"), "one\n");
  git(["add", "tracked.txt"]);
  git(["commit", "-q", "-m", "initial"]);
  const snapshot = checkoutSnapshot(repo);
  assertCleanCheckout(repo);
  fs.writeFileSync(path.join(repo, "tracked.txt"), "two\n");
  assert.throws(() => assertCleanCheckout(repo), /Checkout is dirty/);
  git(["checkout", "--", "tracked.txt"]);
  fs.writeFileSync(path.join(repo, "second.txt"), "two\n");
  git(["add", "second.txt"]);
  git(["commit", "-q", "-m", "second"]);
  assert.throws(() => assertCheckoutUnchanged(repo, snapshot), /Checkout changed during rebuild/);
});
