import { spawnSync } from "node:child_process";
import fs from "node:fs";


function copyAppBundle(source: string, destination: string): void {
  // ditto preserves macOS bundle metadata better than a plain recursive copy.
  if (process.platform === "darwin") {
    const result = spawnSync("ditto", [source, destination], { encoding: "utf8" });
    if (result.status !== 0) {
      throw new Error(`Could not stage app bundle: ${(result.stderr || result.stdout || "ditto failed").trim()}`);
    }
    return;
  }
  fs.cpSync(source, destination, { recursive: true, errorOnExist: true, force: false });
}

export interface AppPromotionOptions {
  sourceBundle: string;
  installedBundle: string;
  hashExecutable: (bundle: string) => string;
  verifyBundle: (bundle: string) => void;
  beforeReplace?: () => void;
  afterInstall?: () => void;
  onRollback?: () => void;
  log?: (message: string) => void;
}

/** Stage, verify, promote, and retain exactly one prior verified app bundle. */
export function promoteAppBundle(options: AppPromotionOptions): void {
  const stage = `${options.installedBundle}.new`;
  const rollback = `${options.installedBundle}.rollback-${process.pid}`;
  const previous = `${options.installedBundle}.previous`;
  const previousRotation = `${previous}.rotation-${process.pid}`;
  if (fs.existsSync(stage) || fs.existsSync(rollback) || fs.existsSync(previousRotation)) {
    throw new Error("A staging or rollback path already exists; resolve it before rebuilding.");
  }
  if (!fs.existsSync(options.installedBundle) && fs.existsSync(previous)) {
    throw new Error("A previous app backup exists without an active app; resolve the backup before rebuilding.");
  }

  const sourceHash = options.hashExecutable(options.sourceBundle);
  let stageOwned = true;
  let oldAppMoved = false;
  let newAppInstalled = false;
  try {
    copyAppBundle(options.sourceBundle, stage);
    if (options.hashExecutable(stage) !== sourceHash) {
      throw new Error("Source and staged app executable hashes do not match.");
    }
    options.verifyBundle(stage);
    options.beforeReplace?.();

    if (fs.existsSync(options.installedBundle)) {
      fs.renameSync(options.installedBundle, rollback);
      oldAppMoved = true;
    }
    fs.renameSync(stage, options.installedBundle);
    stageOwned = false;
    newAppInstalled = true;
    if (options.hashExecutable(options.installedBundle) !== sourceHash) {
      throw new Error("Installed app executable hash does not match the build.");
    }
    options.verifyBundle(options.installedBundle);
    options.afterInstall?.();

    if (oldAppMoved) {
      if (fs.existsSync(previous)) fs.renameSync(previous, previousRotation);
      try {
        fs.renameSync(rollback, previous);
        oldAppMoved = false;
      } catch (error) {
        if (fs.existsSync(previousRotation)) fs.renameSync(previousRotation, previous);
        throw error;
      }
      try {
        fs.rmSync(previousRotation, { recursive: true, force: true });
      } catch (error) {
        options.log?.(`Could not remove superseded rollback bundle ${previousRotation}: ${String(error)}`);
      }
    }
  } catch (error) {
    const rollbackErrors: string[] = [];
    try {
      options.onRollback?.();
    } catch (rollbackError) {
      rollbackErrors.push(`external rollback failed: ${rollbackError instanceof Error ? rollbackError.message : String(rollbackError)}`);
    }
    try {
      if (oldAppMoved && fs.existsSync(rollback)) {
        fs.rmSync(options.installedBundle, { recursive: true, force: true });
        fs.renameSync(rollback, options.installedBundle);
        oldAppMoved = false;
      } else if (newAppInstalled) {
        fs.rmSync(options.installedBundle, { recursive: true, force: true });
      }
      if (stageOwned) fs.rmSync(stage, { recursive: true, force: true });
      if (fs.existsSync(previousRotation) && !fs.existsSync(previous)) {
        fs.renameSync(previousRotation, previous);
      }
    } catch (rollbackError) {
      rollbackErrors.push(`app rollback failed: ${rollbackError instanceof Error ? rollbackError.message : String(rollbackError)}`);
    }
    const message = error instanceof Error ? error.message : String(error);
    if (rollbackErrors.length) throw new Error(`${message}; ${rollbackErrors.join("; ")}`);
    throw error;
  }
}
