import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { renderPlist, resolveLaunchdConfig } from "./launchd-plist";
import { installLaunchAgentTransaction } from "./launchd-install-transaction";

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/**
 * Install + load the hourly jobs LaunchAgent.
 * Binary preference: /Applications/Job Tracker.app, then repo release bundle,
 * release bin, debug bin. Data dir: Application Support (same DB as the GUI and
 * jt) unless JOB_TRACKER_DATA_DIR is explicitly set.
 */
const config = resolveLaunchdConfig({
  homeDir: os.homedir(),
  projectRoot,
  envDataDir: process.env.JOB_TRACKER_DATA_DIR || undefined,
  exists: (p) => fs.existsSync(p),
});
const plistPath = path.join(os.homedir(), "Library", "LaunchAgents", `${config.label}.plist`);

if (config.dataDirInsideRepo) {
  console.warn(
    `WARNING: data dir ${config.dataDir} is inside the repo. The repo data/ folder is dev-only; ` +
      "the GUI and jt use ~/Library/Application Support/com.jobtracker.local. Unset JOB_TRACKER_DATA_DIR unless this is intentional.",
  );
}

fs.mkdirSync(path.dirname(plistPath), { recursive: true });
fs.mkdirSync(config.dataDir, { recursive: true });

const uid = typeof process.getuid === "function" ? process.getuid() : -1;
if (uid < 0) throw new Error("Could not determine the macOS GUI user for the LaunchAgent.");

try {
  installLaunchAgentTransaction(
    plistPath,
    config.label,
    uid,
    renderPlist(config),
    (args) => {
      const result = spawnSync("launchctl", args, { encoding: "utf8" });
      return {
        status: result.status,
        stdout: result.stdout,
        stderr: result.error?.message ?? result.stderr,
      };
    },
  );
  console.log(`Wrote and loaded ${plistPath}`);
  console.log(`Program: ${config.programArguments.join(" ")}`);
  console.log(`Data dir: ${config.dataDir}`);
  console.log(`Loaded ${config.label}. It runs at load and hourly; logs: ${config.logPath}`);
} catch (error) {
  console.error(`LaunchAgent installation failed: ${error instanceof Error ? error.message : String(error)}`);
  console.error(`Review ${plistPath}; retry with npm run jobs:install after resolving the cause.`);
  process.exitCode = 1;
}
