import assert from "node:assert/strict";
import type { SpawnSyncReturns } from "node:child_process";
import { test } from "node:test";

import { resignAdhocAndVerify, type SpawnFn } from "./codesign-app";

function ok(): SpawnSyncReturns<string> {
  return {
    status: 0,
    pid: 0,
    output: ["", "", ""],
    stdout: "",
    stderr: "",
    signal: null,
    error: undefined,
  };
}

function fail(stderr: string): SpawnSyncReturns<string> {
  return { ...ok(), status: 1, stderr };
}

test("resignAdhocAndVerify force-signs then verifies", () => {
  const calls: Array<{ command: string; args: readonly string[] }> = [];
  const spawn: SpawnFn = (command, args) => {
    calls.push({ command, args: [...args] });
    return ok();
  };

  resignAdhocAndVerify("/tmp/Job Tracker.app", spawn);

  assert.deepEqual(calls, [
    { command: "codesign", args: ["--force", "--deep", "-s", "-", "/tmp/Job Tracker.app"] },
    { command: "codesign", args: ["--verify", "--deep", "--strict", "/tmp/Job Tracker.app"] },
  ]);
});

test("resignAdhocAndVerify surfaces re-sign failures", () => {
  const spawn: SpawnFn = (_command, args) => {
    if (args.includes("--force")) return fail("unable to sign");
    return ok();
  };
  assert.throws(() => resignAdhocAndVerify("/tmp/App.app", spawn), /Ad-hoc re-sign failed: unable to sign/);
});

test("resignAdhocAndVerify surfaces verify failures after a successful re-sign", () => {
  const spawn: SpawnFn = (_command, args) => {
    if (args.includes("--verify")) return fail("code has no resources but signature indicates they must be present");
    return ok();
  };
  assert.throws(
    () => resignAdhocAndVerify("/tmp/App.app", spawn),
    /Code signature verification failed: code has no resources/,
  );
});
