import fs from "node:fs";
import path from "node:path";

export interface LaunchctlResult {
  status: number | null;
  stdout?: string | null;
  stderr?: string | null;
}

export type LaunchctlRunner = (args: string[]) => LaunchctlResult;

function atomicallyWrite(filePath: string, contents: Buffer | string): void {
  const temporaryPath = `${filePath}.tmp-${process.pid}-${Date.now()}`;
  fs.writeFileSync(temporaryPath, contents, { flag: "wx" });
  fs.renameSync(temporaryPath, filePath);
}

function isLoaded(label: string, uid: number, run: LaunchctlRunner): boolean {
  return run(["print", `gui/${uid}/${label}`]).status === 0;
}

/** Replace a LaunchAgent plist and load it, restoring the previous state on failure. */
export function installLaunchAgentTransaction(
  plistPath: string,
  label: string,
  uid: number,
  contents: string,
  run: LaunchctlRunner,
): void {
  const existed = fs.existsSync(plistPath);
  const oldContents = existed ? fs.readFileSync(plistPath) : undefined;
  const wasLoaded = isLoaded(label, uid, run);

  fs.mkdirSync(path.dirname(plistPath), { recursive: true });
  run(["unload", plistPath]);
  atomicallyWrite(plistPath, contents);

  const load = run(["load", "-w", plistPath]);
  const output = `${load.stdout ?? ""}${load.stderr ?? ""}`;
  if (load.status === 0 && !/error|failed/i.test(output) && isLoaded(label, uid, run)) return;

  const failure = `launchctl could not load ${label} (exit ${load.status ?? "?"}${output.trim() ? `: ${output.trim()}` : ""})`;
  const restoreErrors: string[] = [];
  const unload = run(["unload", plistPath]);
  if (unload.status !== 0 && unload.status !== null) {
    // An unloaded or never-loaded service commonly returns nonzero here.
  }
  try {
    if (existed && oldContents) {
      atomicallyWrite(plistPath, oldContents);
      if (wasLoaded) {
        const restoreLoad = run(["load", "-w", plistPath]);
        if (restoreLoad.status !== 0 || !isLoaded(label, uid, run)) {
          restoreErrors.push(`previous LaunchAgent could not be reloaded: ${restoreLoad.stderr ?? "launchctl load failed"}`);
        }
      }
    } else {
      fs.rmSync(plistPath, { force: true });
    }
  } catch (error) {
    restoreErrors.push(`previous plist could not be restored: ${error instanceof Error ? error.message : String(error)}`);
  }
  throw new Error(restoreErrors.length ? `${failure}; ${restoreErrors.join("; ")}` : failure);
}
