import { execSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const label = "com.jobtracker.local.jobs";
const plistPath = path.join(os.homedir(), "Library", "LaunchAgents", `${label}.plist`);
const projectRoot = process.cwd();

/**
 * Prefer the packaged Tauri binary with `--run-jobs`. Fall back to the debug
 * cargo target.
 */
function resolveRunner(): {
  programArguments: string[];
  workingDirectory: string;
  logPath: string;
  dataDir: string;
} {
  const dataDir = process.env.JOB_TRACKER_DATA_DIR
    ? path.resolve(process.env.JOB_TRACKER_DATA_DIR)
    : path.join(projectRoot, "data");
  const logPath = path.join(dataDir, "jobs-worker.log");

  const releaseApp = path.join(
    projectRoot,
    "src-tauri/target/release/bundle/macos/Job Tracker.app/Contents/MacOS/job-tracker",
  );
  const releaseBin = path.join(projectRoot, "src-tauri/target/release/job-tracker");
  const debugBin = path.join(projectRoot, "src-tauri/target/debug/job-tracker");

  for (const binary of [releaseApp, releaseBin, debugBin]) {
    if (fs.existsSync(binary)) {
      return {
        programArguments: [binary, "--run-jobs", "--data-dir", dataDir],
        workingDirectory: projectRoot,
        logPath,
        dataDir,
      };
    }
  }

  throw new Error(
    "Tauri binary not found. Run `npm run tauri:build` (or `npm run tauri:dev` once) then re-run jobs:install.",
  );
}

const { programArguments, workingDirectory, logPath, dataDir } = resolveRunner();

const programArgsXml = programArguments
  .map((arg) => `      <string>${arg}</string>`)
  .join("\n");

const plist = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>Label</key>
    <string>${label}</string>
    <key>ProgramArguments</key>
    <array>
${programArgsXml}
    </array>
    <key>WorkingDirectory</key>
    <string>${workingDirectory}</string>
    <key>EnvironmentVariables</key>
    <dict>
      <key>JOB_TRACKER_DATA_DIR</key>
      <string>${dataDir}</string>
    </dict>
    <key>StartInterval</key>
    <integer>3600</integer>
    <key>RunAtLoad</key>
    <true/>
    <key>StandardOutPath</key>
    <string>${logPath}</string>
    <key>StandardErrorPath</key>
    <string>${logPath}</string>
  </dict>
</plist>
`;

fs.mkdirSync(path.dirname(plistPath), { recursive: true });
fs.mkdirSync(path.dirname(logPath), { recursive: true });

// Unload any previous agent before rewriting so it cannot keep writing an old tree.
try {
  execSync(`launchctl unload "${plistPath}"`, { stdio: "ignore" });
} catch {
  // not loaded
}

fs.writeFileSync(plistPath, plist);

console.log(`Wrote ${plistPath}`);
console.log(`Program: ${programArguments.join(" ")}`);
console.log("Load it with:");
console.log(`  launchctl unload ${plistPath} 2>/dev/null; launchctl load ${plistPath}`);
console.log(`The worker runs once per hour and logs to ${logPath}`);
