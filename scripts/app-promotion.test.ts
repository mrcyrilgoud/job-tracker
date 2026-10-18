import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, test } from "node:test";

import { promoteAppBundle } from "./app-promotion";

const tempDirs: string[] = [];
afterEach(() => {
  for (const dir of tempDirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});

function bundle(content: string): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "job-tracker-app-"));
  tempDirs.push(dir);
  fs.mkdirSync(path.join(dir, "Contents/MacOS"), { recursive: true });
  fs.writeFileSync(path.join(dir, "Contents/MacOS/job-tracker"), content);
  return dir;
}

function hashExecutable(root: string): string {
  return createHash("sha256").update(fs.readFileSync(path.join(root, "Contents/MacOS/job-tracker"))).digest("hex");
}

function installPaths(): { root: string; installed: string } {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "job-tracker-install-"));
  tempDirs.push(root);
  return { root, installed: path.join(root, "Job Tracker.app") };
}

test("promotion verifies staged and installed apps and rotates one previous bundle", () => {
  const source = bundle("new binary");
  const oldApp = bundle("old binary");
  const oldPrevious = bundle("older binary");
  const { root, installed } = installPaths();
  fs.renameSync(oldApp, installed);
  fs.renameSync(oldPrevious, `${installed}.previous`);
  let verified = 0;

  promoteAppBundle({
    sourceBundle: source,
    installedBundle: installed,
    hashExecutable,
    verifyBundle: () => { verified++; },
  });

  assert.equal(fs.readFileSync(path.join(installed, "Contents/MacOS/job-tracker"), "utf8"), "new binary");
  assert.equal(fs.readFileSync(path.join(`${installed}.previous`, "Contents/MacOS/job-tracker"), "utf8"), "old binary");
  assert.equal(verified, 2);
  assert.equal(fs.existsSync(`${installed}.new`), false);
  assert.equal(fs.existsSync(`${installed}.previous.rotation-${process.pid}`), false);
  assert.equal(fs.readdirSync(root).filter((name) => name.includes("previous")).length, 1);
});

test("verification failure before promotion leaves the installed app untouched", () => {
  const source = bundle("new binary");
  const oldApp = bundle("old binary");
  const { installed } = installPaths();
  fs.renameSync(oldApp, installed);

  assert.throws(() => promoteAppBundle({
    sourceBundle: source,
    installedBundle: installed,
    hashExecutable,
    verifyBundle: () => { throw new Error("signature rejected"); },
  }), /signature rejected/);

  assert.equal(fs.readFileSync(path.join(installed, "Contents/MacOS/job-tracker"), "utf8"), "old binary");
  assert.equal(fs.existsSync(`${installed}.new`), false);
});

test("staged executable hash mismatch prevents promotion", () => {
  const source = bundle("new binary");
  const oldApp = bundle("old binary");
  const { installed } = installPaths();
  fs.renameSync(oldApp, installed);
  const actualHash = hashExecutable;

  assert.throws(() => promoteAppBundle({
    sourceBundle: source,
    installedBundle: installed,
    hashExecutable: (bundlePath) => bundlePath.endsWith(".new") ? "wrong-hash" : actualHash(bundlePath),
    verifyBundle: () => undefined,
  }), /hashes do not match/);

  assert.equal(fs.readFileSync(path.join(installed, "Contents/MacOS/job-tracker"), "utf8"), "old binary");
  assert.equal(fs.existsSync(`${installed}.new`), false);
});

test("post-promotion failure restores the previous app and removes a failed first install", () => {
  const source = bundle("new binary");
  const oldApp = bundle("old binary");
  const { installed } = installPaths();
  fs.renameSync(oldApp, installed);

  assert.throws(() => promoteAppBundle({
    sourceBundle: source,
    installedBundle: installed,
    hashExecutable,
    verifyBundle: () => undefined,
    afterInstall: () => { throw new Error("LaunchAgent rejected"); },
  }), /LaunchAgent rejected/);
  assert.equal(fs.readFileSync(path.join(installed, "Contents/MacOS/job-tracker"), "utf8"), "old binary");
  assert.equal(fs.existsSync(`${installed}.previous`), false);

  const firstInstall = installPaths();
  assert.throws(() => promoteAppBundle({
    sourceBundle: source,
    installedBundle: firstInstall.installed,
    hashExecutable,
    verifyBundle: () => undefined,
    afterInstall: () => { throw new Error("verification failed"); },
  }), /verification failed/);
  assert.equal(fs.existsSync(firstInstall.installed), false);
});
