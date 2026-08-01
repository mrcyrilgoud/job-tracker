import { execSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const projectRoot = process.cwd();
const dataDir = process.env.JOB_TRACKER_DATA_DIR
  ? path.resolve(process.env.JOB_TRACKER_DATA_DIR)
  : path.join(projectRoot, "data");
const logPath = path.join(dataDir, "rebuild.log");
const lockPath = path.join(dataDir, "rebuild.lock");
const appBundle = path.join(
  projectRoot,
  "src-tauri/target/release/bundle/macos/Job Tracker.app",
);
const appName = "Job Tracker";

const flags = new Set(process.argv.slice(2));
const skipJobs = flags.has("--skip-jobs");
const background = flags.has("--background");

function appendLog(message: string): void {
  const line = `[${new Date().toISOString()}] ${message}\n`;
  fs.appendFileSync(logPath, line);
  // When --background (post-commit nohup already redirects stdout into the log),
  // skip mirroring to stdout to avoid duplicate lines.
  if (!background) {
    process.stdout.write(line);
  }
}

function notify(title: string, message: string): void {
  try {
    const escapedTitle = title.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
    const escapedMessage = message.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
    execSync(
      `osascript -e 'display notification "${escapedMessage}" with title "${escapedTitle}"'`,
      { stdio: "ignore" },
    );
  } catch {
    // notification is best-effort
  }
}

function isPidAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function acquireLock(): void {
  fs.mkdirSync(dataDir, { recursive: true });

  if (fs.existsSync(lockPath)) {
    const raw = fs.readFileSync(lockPath, "utf8").trim();
    const existingPid = Number(raw);
    if (Number.isInteger(existingPid) && existingPid > 0 && isPidAlive(existingPid)) {
      const msg = `Rebuild already running (pid ${existingPid}). See ${logPath}`;
      console.error(msg);
      appendLog(msg);
      process.exit(1);
    }
  }

  fs.writeFileSync(lockPath, `${process.pid}\n`);
}

function releaseLock(): void {
  try {
    if (!fs.existsSync(lockPath)) return;
    const existingPid = Number(fs.readFileSync(lockPath, "utf8").trim());
    if (existingPid === process.pid) {
      fs.unlinkSync(lockPath);
    }
  } catch {
    // ignore unlock failures
  }
}

function sleepMs(ms: number): void {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

function isAppRunning(): boolean {
  try {
    const result = execSync(
      `osascript -e 'tell application "System Events" to (name of processes) contains "${appName}"'`,
      { encoding: "utf8" },
    ).trim();
    return result === "true";
  } catch {
    // fall through
  }

  try {
    execSync(`pgrep -x job-tracker`, { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function quitApp(): void {
  try {
    execSync(`osascript -e 'tell application "${appName}" to quit'`, {
      stdio: "ignore",
      timeout: 15_000,
    });
  } catch {
    // best-effort quit
  }

  for (let i = 0; i < 40; i++) {
    if (!isAppRunning()) return;
    sleepMs(250);
  }

  appendLog(`Warning: ${appName} still running after quit attempt`);
}

function openApp(): void {
  if (!fs.existsSync(appBundle)) return;
  try {
    execSync(`open "${appBundle}"`, { stdio: "ignore" });
  } catch {
    // best-effort relaunch
  }
}

function runLogged(command: string, label: string): number {
  appendLog(`Starting: ${label} (${command})`);
  const result = spawnSync(command, {
    shell: true,
    cwd: projectRoot,
    encoding: "utf8",
    env: process.env,
    maxBuffer: 50 * 1024 * 1024,
  });

  if (result.stdout) {
    fs.appendFileSync(logPath, result.stdout);
    if (!background) process.stdout.write(result.stdout);
  }
  if (result.stderr) {
    fs.appendFileSync(logPath, result.stderr);
    if (!background) process.stderr.write(result.stderr);
  }
  if (result.error) {
    const errLine = `Command error: ${result.error.message}\n`;
    fs.appendFileSync(logPath, errLine);
    if (!background) process.stderr.write(errLine);
  }

  const code = result.status ?? 1;
  appendLog(`Finished: ${label} (exit ${code})`);
  return code;
}

function main(): void {
  fs.mkdirSync(dataDir, { recursive: true });
  acquireLock();

  const cleanup = (): void => {
    releaseLock();
  };
  process.on("exit", cleanup);
  process.on("SIGINT", () => {
    cleanup();
    process.exit(130);
  });
  process.on("SIGTERM", () => {
    cleanup();
    process.exit(143);
  });

  appendLog(background ? "Rebuild started (--background)" : "Rebuild started");

  const wasRunning = isAppRunning();
  if (wasRunning) {
    appendLog(`Quitting ${appName} before rebuild`);
    quitApp();
  }

  const buildCode = runLogged("npm run tauri:build", "tauri:build");
  if (buildCode !== 0) {
    notify("Job Tracker rebuild failed", `See ${logPath}`);
    process.exit(buildCode);
  }

  if (!skipJobs) {
    const jobsCode = runLogged("npm run jobs:install", "jobs:install");
    if (jobsCode !== 0) {
      notify("Job Tracker rebuild failed", `jobs:install failed — see ${logPath}`);
      process.exit(jobsCode);
    }
  } else {
    appendLog("Skipping jobs:install (--skip-jobs)");
  }

  if (wasRunning) {
    appendLog(`Relaunching ${appBundle}`);
    openApp();
  }

  appendLog(`Rebuild succeeded: ${appBundle}`);
  notify("Job Tracker rebuild complete", appBundle);
}

main();
