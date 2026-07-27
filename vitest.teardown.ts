import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export default function teardown() {
  const dir = process.env.JOB_TRACKER_DATA_DIR?.trim();
  if (!dir) {
    return;
  }

  const resolved = path.resolve(dir);
  const expectedPrefix = path.join(os.tmpdir(), "job-tracker-vitest-");
  if (!resolved.startsWith(expectedPrefix)) {
    throw new Error(`Refusing to remove unexpected Vitest data directory: ${resolved}`);
  }

  try {
    fs.rmSync(resolved, { recursive: true, force: true });
  } catch {
    // Best-effort cleanup; OS temp dirs are ephemeral either way.
  }
}
