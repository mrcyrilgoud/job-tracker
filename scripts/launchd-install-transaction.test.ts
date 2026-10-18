import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, test } from "node:test";

import {
  installLaunchAgentTransaction,
  type LaunchctlResult,
} from "./launchd-install-transaction";

const tempDirs: string[] = [];
afterEach(() => {
  for (const dir of tempDirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});

function plistPath(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "job-tracker-launchd-"));
  tempDirs.push(dir);
  return path.join(dir, "LaunchAgents", "com.jobtracker.local.jobs.plist");
}

test("successful LaunchAgent update atomically writes and confirms the loaded service", () => {
  const file = plistPath();
  let loaded = false;
  const calls: string[][] = [];
  const runner = (args: string[]): LaunchctlResult => {
    calls.push(args);
    if (args[0] === "unload") loaded = false;
    if (args[0] === "load") loaded = true;
    return { status: args[0] === "print" && !loaded ? 113 : 0 };
  };

  installLaunchAgentTransaction(file, "com.jobtracker.local.jobs", 501, "new plist", runner);
  assert.equal(fs.readFileSync(file, "utf8"), "new plist");
  assert.equal(loaded, true);
  assert.ok(calls.some((args) => args[0] === "print"));
});

test("failed LaunchAgent load restores and reloads the prior plist", () => {
  const file = plistPath();
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, "old plist");
  let loaded = true;
  const runner = (args: string[]): LaunchctlResult => {
    if (args[0] === "unload") loaded = false;
    if (args[0] === "load") {
      const contents = fs.readFileSync(file, "utf8");
      if (contents === "new plist") return { status: 1, stderr: "load failed" };
      loaded = true;
    }
    if (args[0] === "print") return { status: loaded ? 0 : 113 };
    return { status: 0 };
  };

  assert.throws(
    () => installLaunchAgentTransaction(file, "com.jobtracker.local.jobs", 501, "new plist", runner),
    /could not load/,
  );
  assert.equal(fs.readFileSync(file, "utf8"), "old plist");
  assert.equal(loaded, true);
});

test("failed first LaunchAgent load removes the newly written plist", () => {
  const file = plistPath();
  const runner = (args: string[]): LaunchctlResult => {
    if (args[0] === "load") return { status: 1, stderr: "load failed" };
    return { status: 113 };
  };

  assert.throws(
    () => installLaunchAgentTransaction(file, "com.jobtracker.local.jobs", 501, "new plist", runner),
    /could not load/,
  );
  assert.equal(fs.existsSync(file), false);
});
