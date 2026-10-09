import path from "node:path";

/** Pure helpers for the hourly jobs LaunchAgent (no fs / launchctl side effects). */

export const LAUNCHD_LABEL = "com.jobtracker.local.jobs";
export const SYSTEM_APP_BINARY = "/Applications/Job Tracker.app/Contents/MacOS/job-tracker";

export interface LaunchdConfigInput {
  homeDir: string;
  projectRoot: string;
  /** Value of JOB_TRACKER_DATA_DIR, if the user explicitly set it. */
  envDataDir?: string;
  /** Injected existence check so callers/tests control which binaries "exist". */
  exists: (p: string) => boolean;
}

export interface LaunchdConfig {
  label: string;
  binary: string;
  dataDir: string;
  logPath: string;
  workingDirectory: string;
  programArguments: string[];
  dataDirInsideRepo: boolean;
}

export function defaultDataDir(homeDir: string): string {
  return path.join(homeDir, "Library", "Application Support", "com.jobtracker.local");
}

export function candidateBinaries(projectRoot: string): string[] {
  return [
    SYSTEM_APP_BINARY,
    path.join(
      projectRoot,
      "src-tauri/target/release/bundle/macos/Job Tracker.app/Contents/MacOS/job-tracker",
    ),
    path.join(projectRoot, "src-tauri/target/release/job-tracker"),
    path.join(projectRoot, "src-tauri/target/debug/job-tracker"),
  ];
}

export function isInside(child: string, parent: string): boolean {
  const rel = path.relative(path.resolve(parent), path.resolve(child));
  return rel === "" || (!rel.startsWith("..") && !path.isAbsolute(rel));
}

export function resolveLaunchdConfig(input: LaunchdConfigInput): LaunchdConfig {
  const dataDir = input.envDataDir
    ? path.resolve(input.envDataDir)
    : defaultDataDir(input.homeDir);
  const binary = candidateBinaries(input.projectRoot).find((b) => input.exists(b));
  if (!binary) {
    throw new Error(
      "Job Tracker binary not found. Install /Applications/Job Tracker.app (npm run app:rebuild) or run `npm run tauri:build`, then re-run jobs:install.",
    );
  }
  return {
    label: LAUNCHD_LABEL,
    binary,
    dataDir,
    logPath: path.join(dataDir, "jobs-worker.log"),
    // Never run from the repo: keeps cwd-based dev data discovery out of the worker.
    workingDirectory: dataDir,
    programArguments: [binary, "--run-jobs", "--data-dir", dataDir],
    dataDirInsideRepo: isInside(dataDir, input.projectRoot),
  };
}

function xmlEscape(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export function renderPlist(config: LaunchdConfig): string {
  const s = (v: string) => `<string>${xmlEscape(v)}</string>`;
  const args = config.programArguments.map((a) => `      ${s(a)}`).join("\n");
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>Label</key>
    ${s(config.label)}
    <key>ProgramArguments</key>
    <array>
${args}
    </array>
    <key>WorkingDirectory</key>
    ${s(config.workingDirectory)}
    <key>EnvironmentVariables</key>
    <dict>
      <key>JOB_TRACKER_DATA_DIR</key>
      ${s(config.dataDir)}
    </dict>
    <key>StartInterval</key>
    <integer>3600</integer>
    <key>RunAtLoad</key>
    <true/>
    <key>StandardOutPath</key>
    ${s(config.logPath)}
    <key>StandardErrorPath</key>
    ${s(config.logPath)}
  </dict>
</plist>
`;
}
