import { spawnSync, type SpawnSyncReturns } from "node:child_process";

export type SpawnFn = (
  command: string,
  args: readonly string[],
  options?: { encoding?: BufferEncoding },
) => SpawnSyncReturns<string>;

function spawnFailureMessage(result: SpawnSyncReturns<string>, fallback: string): string {
  return (result.stderr || result.stdout || result.error?.message || fallback).trim();
}

/**
 * Tauri's macOS ad-hoc signature sometimes claims sealed resources without
 * writing Contents/_CodeSignature. Re-sign ad-hoc, then verify deeply.
 */
export function resignAdhocAndVerify(bundle: string, spawn: SpawnFn = spawnSync): void {
  const resign = spawn("codesign", ["--force", "--deep", "-s", "-", bundle], { encoding: "utf8" });
  if (resign.status !== 0) {
    throw new Error(`Ad-hoc re-sign failed: ${spawnFailureMessage(resign, "codesign --force failed")}`);
  }
  const verify = spawn("codesign", ["--verify", "--deep", "--strict", bundle], { encoding: "utf8" });
  if (verify.status !== 0) {
    throw new Error(
      `Code signature verification failed: ${spawnFailureMessage(verify, "codesign --verify failed")}`,
    );
  }
}
