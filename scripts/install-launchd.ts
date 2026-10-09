import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { renderPlist, resolveLaunchdConfig } from "./launchd-plist";

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

// Unload any previous agent before rewriting so it cannot keep writing an old tree.
spawnSync("launchctl", ["unload", plistPath], { stdio: "ignore" });

fs.writeFileSync(plistPath, renderPlist(config));
console.log(`Wrote ${plistPath}`);
console.log(`Program: ${config.programArguments.join(" ")}`);
console.log(`Data dir: ${config.dataDir}`);

const load = spawnSync("launchctl", ["load", "-w", plistPath], { encoding: "utf8" });
const loadOutput = `${load.stdout ?? ""}${load.stderr ?? ""}`.trim();
if (load.status === 0 && !/error|failed/i.test(loadOutput)) {
  console.log(`Loaded ${config.label}. It runs at load and hourly; logs: ${config.logPath}`);
} else {
  console.warn(
    `WARNING: plist written but launchctl load failed (exit ${load.status ?? "?"}${loadOutput ? `: ${loadOutput}` : ""}).`,
  );
  console.warn(`Load it manually: launchctl unload "${plistPath}" 2>/dev/null; launchctl load -w "${plistPath}"`);
}
