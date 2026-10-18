import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { promoteAppBundle } from "./app-promotion";
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
  type CheckoutSnapshot,
} from "./rebuild-support";

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const dataDir = path.join(projectRoot, "data");
const logPath = path.join(dataDir, "rebuild.log");
const lockPath = path.join(dataDir, "rebuild.lock");
const queueDir = path.join(dataDir, "rebuild-queue");
const appBundle = path.join(projectRoot, "src-tauri/target/release/bundle/macos/Job Tracker.app");
const systemAppBundle = "/Applications/Job Tracker.app";
const appExecutable = path.join("Contents", "MacOS", "job-tracker");
const appName = "Job Tracker";
const launchAgentLabel = "com.jobtracker.local.jobs";
const launchAgentPath = path.join(os.homedir(), "Library", "LaunchAgents", `${launchAgentLabel}.plist`);

const args = process.argv.slice(2);
const skipJobs = args.includes("--skip-jobs");
const background = args.includes("--background");
const requestedCommit = flagValue("--request-commit");
const requestedBranch = flagValue("--request-branch");

interface LaunchAgentSnapshot {
  existed: boolean;
  contents?: Buffer;
  wasLoaded: boolean;
}

function flagValue(flag: string): string | undefined {
  const index = args.indexOf(flag);
  return index >= 0 ? args[index + 1] : undefined;
}

function appendLog(message: string): void {
  fs.mkdirSync(dataDir, { recursive: true });
  const line = `[${new Date().toISOString()}] ${message}\n`;
  fs.appendFileSync(logPath, line);
  if (!background) process.stdout.write(line);
}

function notify(title: string, message: string): void {
  const script = `display notification ${JSON.stringify(message)} with title ${JSON.stringify(title)}`;
  try {
    spawnSync("osascript", ["-e", script], { stdio: "ignore" });
  } catch {
    // Notifications are best-effort.
  }
}

function commandOutput(command: string, commandArgs: string[]): string | null {
  const result = spawnSync(command, commandArgs, { encoding: "utf8" });
  if (result.status !== 0) return null;
  return `${result.stdout ?? ""}${result.stderr ?? ""}`;
}

function launchctlLoaded(): boolean {
  const uid = typeof process.getuid === "function" ? process.getuid() : -1;
  if (uid < 0) return false;
  return commandOutput("launchctl", ["print", `gui/${uid}/${launchAgentLabel}`]) !== null;
}

function captureLaunchAgent(): LaunchAgentSnapshot {
  return {
    existed: fs.existsSync(launchAgentPath),
    contents: fs.existsSync(launchAgentPath) ? fs.readFileSync(launchAgentPath) : undefined,
    wasLoaded: launchctlLoaded(),
  };
}

function restoreLaunchAgent(snapshot: LaunchAgentSnapshot): void {
  spawnSync("launchctl", ["unload", launchAgentPath], { stdio: "ignore" });
  if (snapshot.existed && snapshot.contents) {
    fs.mkdirSync(path.dirname(launchAgentPath), { recursive: true });
    const restorePath = `${launchAgentPath}.restore-${process.pid}`;
    fs.writeFileSync(restorePath, snapshot.contents);
    fs.renameSync(restorePath, launchAgentPath);
    if (snapshot.wasLoaded) {
      const load = spawnSync("launchctl", ["load", "-w", launchAgentPath], { encoding: "utf8" });
      if (load.status !== 0 || !launchctlLoaded()) {
        throw new Error(`Could not restore the previous LaunchAgent: ${load.stderr ?? "launchctl load failed"}`);
      }
    }
  } else {
    fs.rmSync(launchAgentPath, { force: true });
  }
}

function runLogged(command: string, commandArgs: string[], label: string): number {
  appendLog(`Starting: ${label} (${command} ${commandArgs.join(" ")})`);
  const result = spawnSync(command, commandArgs, {
    cwd: projectRoot,
    encoding: "utf8",
    env: process.env,
    maxBuffer: 50 * 1024 * 1024,
  });
  if (result.stdout) fs.appendFileSync(logPath, result.stdout);
  if (result.stderr) fs.appendFileSync(logPath, result.stderr);
  if (result.error) fs.appendFileSync(logPath, `Command error: ${result.error.message}\n`);
  const code = result.status ?? 1;
  appendLog(`Finished: ${label} (exit ${code})`);
  return code;
}

function appRunning(): boolean {
  const byName = spawnSync("osascript", [
    "-e",
    `tell application "System Events" to (name of processes) contains ${JSON.stringify(appName)}`,
  ], { encoding: "utf8" });
  if (byName.status === 0 && byName.stdout.trim() === "true") return true;
  return spawnSync("pgrep", ["-x", "job-tracker"], { stdio: "ignore" }).status === 0;
}

function installedAppRunning(): boolean {
  const processes = spawnSync("pgrep", ["-x", "job-tracker"], { encoding: "utf8" });
  if (processes.status !== 0) return false;
  return processes.stdout.trim().split(/\s+/).some((pid) => {
    const command = spawnSync("ps", ["-p", pid, "-o", "command="], { encoding: "utf8" });
    return command.status === 0 && command.stdout.includes(systemAppBundle);
  });
}

function quitApp(): void {
  spawnSync("osascript", ["-e", `tell application ${JSON.stringify(appName)} to quit`], {
    stdio: "ignore",
    timeout: 15_000,
  });
  for (let i = 0; i < 40; i++) {
    if (!appRunning()) return;
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 250);
  }
  if (appRunning()) throw new Error(`${appName} is still running; deployment stopped before replacing it.`);
}

function openAppAndWait(): void {
  const open = spawnSync("open", [systemAppBundle], { encoding: "utf8" });
  if (open.status !== 0) throw new Error(`Could not relaunch ${systemAppBundle}: ${open.stderr ?? "open failed"}`);
  for (let i = 0; i < 40; i++) {
    if (installedAppRunning()) return;
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 250);
  }
  throw new Error(`${appName} did not relaunch successfully.`);
}

function executableHash(bundle: string): string {
  const executable = path.join(bundle, appExecutable);
  return execFileSync("shasum", ["-a", "256", executable], { encoding: "utf8" }).trim().split(/\s+/)[0];
}

function verifySignature(bundle: string): void {
  const result = spawnSync("codesign", ["--verify", "--deep", "--strict", bundle], { encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(`Code signature verification failed: ${(result.stderr || result.stdout || "codesign failed").trim()}`);
  }
}

function verifyCli(bundle: string): void {
  const result = spawnSync(path.join(bundle, appExecutable), ["--help"], { encoding: "utf8", timeout: 15_000 });
  if (result.status !== 0) {
    throw new Error(`Packaged CLI check failed: ${(result.stderr || result.stdout || "--help failed").trim()}`);
  }
}

function verifyLaunchAgent(): void {
  if (!launchctlLoaded()) throw new Error(`LaunchAgent ${launchAgentLabel} is not loaded.`);
}

function deployOne(snapshot: CheckoutSnapshot): void {
  assertCleanCheckout(projectRoot);
  const npmBin = process.env.JOB_TRACKER_NPM_BIN || "npm";
  const buildCode = runLogged(npmBin, ["run", "tauri:build"], "tauri:build");
  if (buildCode !== 0) throw new Error(`App build failed with exit ${buildCode}.`);
  assertCheckoutUnchanged(projectRoot, snapshot);

  if (!fs.existsSync(appBundle)) throw new Error(`Build output is missing: ${appBundle}`);
  let launchAgentSnapshot: LaunchAgentSnapshot | undefined;
  let launchAgentChanged = false;
  const wasRunning = appRunning();
  try {
    promoteAppBundle({
      sourceBundle: appBundle,
      installedBundle: systemAppBundle,
      hashExecutable: executableHash,
      verifyBundle: (bundle) => {
        verifySignature(bundle);
        verifyCli(bundle);
      },
      beforeReplace: () => {
        assertCheckoutUnchanged(projectRoot, snapshot);
        if (wasRunning) {
          appendLog(`Quitting ${appName} before promotion`);
          quitApp();
        }
      },
      afterInstall: () => {
        assertCheckoutUnchanged(projectRoot, snapshot);
        if (!skipJobs) {
          launchAgentSnapshot = captureLaunchAgent();
          launchAgentChanged = true;
          const jobsCode = runLogged(npmBin, ["run", "jobs:install"], "jobs:install");
          if (jobsCode !== 0) throw new Error(`jobs:install failed with exit ${jobsCode}.`);
          verifyLaunchAgent();
        } else {
          appendLog("Skipping jobs:install (--skip-jobs)");
        }
        if (wasRunning) {
          appendLog(`Relaunching ${systemAppBundle}`);
          openAppAndWait();
        }
      },
      onRollback: () => {
        if (launchAgentChanged && launchAgentSnapshot) {
          restoreLaunchAgent(launchAgentSnapshot);
          appendLog("Previous LaunchAgent configuration restored");
        }
      },
      log: appendLog,
    });
  } catch (error) {
    appendLog(`Deployment failed: ${error instanceof Error ? error.message : String(error)}`);
    if (wasRunning && fs.existsSync(systemAppBundle) && !appRunning()) {
      spawnSync("open", [systemAppBundle], { stdio: "ignore" });
    }
    throw error;
  }
}

function drainQueue(): number {
  for (;;) {
    const requests = pendingRebuildRequests(queueDir);
    if (requests.length === 0) return 0;

    let current: CheckoutSnapshot;
    try {
      assertCleanCheckout(projectRoot);
      current = checkoutSnapshot(projectRoot);
    } catch (error) {
      appendLog(`Rebuild pending: ${error instanceof Error ? error.message : String(error)}`);
      notify("Job Tracker rebuild pending", `Clean the checkout and rerun app:rebuild. See ${logPath}`);
      return 1;
    }

    const eligible = requests.filter((request) => {
      try {
        return readRebuildRequest(request).branch === current.branch;
      } catch {
        return false;
      }
    });
    const stale = requests.filter((request) => !eligible.includes(request));
    clearRebuildRequests(stale);
    if (eligible.length === 0) continue;

    try {
      appendLog(`Building clean checkout ${current.branch}@${current.commit}`);
      deployOne(current);
      clearRebuildRequests(eligible);
      appendLog(`Rebuild succeeded: ${systemAppBundle}`);
      notify("Job Tracker rebuild complete", systemAppBundle);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      appendLog(`Rebuild failed; request remains queued: ${message}`);
      notify("Job Tracker rebuild failed", `See ${logPath}`);
      if (/Checkout changed during rebuild/.test(message)) continue;
      return 1;
    }
  }
}

function main(): number {
  fs.mkdirSync(dataDir, { recursive: true });
  let snapshot: CheckoutSnapshot;
  try {
    snapshot = checkoutSnapshot(projectRoot);
  } catch {
    appendLog("Rebuild skipped: HEAD is detached or Git metadata is unavailable.");
    return 1;
  }
  enqueueRebuild(queueDir, requestedCommit ?? snapshot.commit, requestedBranch ?? snapshot.branch);

  let token = acquireDirectoryLock(lockPath);
  if (!token) {
    appendLog("Rebuild request queued behind the active rebuild.");
    return 0;
  }
  for (;;) {
    let result: number;
    try {
      result = drainQueue();
    } finally {
      releaseDirectoryLock(lockPath, token);
    }
    if (result !== 0 || pendingRebuildRequests(queueDir).length === 0) return result;
    // Close the handoff race where a commit queued just before lock release.
    token = acquireDirectoryLock(lockPath);
    if (!token) return 0;
  }
}

process.exitCode = main();
