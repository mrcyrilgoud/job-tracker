import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const label = "com.jobtracker.local.jobs";
const plistPath = path.join(os.homedir(), "Library", "LaunchAgents", `${label}.plist`);
const projectRoot = process.cwd();
const nodePath = process.execPath;
const scriptPath = path.join(projectRoot, "scripts", "run-jobs.ts");
const tsxPath = path.join(projectRoot, "node_modules", "tsx", "dist", "cli.mjs");
const logPath = path.join(projectRoot, "data", "jobs-worker.log");

const plist = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>Label</key>
    <string>${label}</string>
    <key>ProgramArguments</key>
    <array>
      <string>${nodePath}</string>
      <string>${tsxPath}</string>
      <string>${scriptPath}</string>
    </array>
    <key>WorkingDirectory</key>
    <string>${projectRoot}</string>
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
fs.writeFileSync(plistPath, plist);

console.log(`Wrote ${plistPath}`);
console.log("Load it with:");
console.log(`  launchctl unload ${plistPath} 2>/dev/null; launchctl load ${plistPath}`);
console.log("The worker runs once per hour and logs to data/jobs-worker.log");
