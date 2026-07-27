import fs from "node:fs";
import path from "node:path";

/**
 * Guardrail: fail fast if a test run somehow lacks an isolated data dir.
 * The root is injected via `test.env` before modules load. Each worker gets its
 * own database so parallel test files cannot contend for the same SQLite lock.
 */
const dataRoot = process.env.JOB_TRACKER_DATA_DIR?.trim();
const workerId = process.env.VITEST_WORKER_ID?.trim();
if (!dataRoot || !workerId) {
  throw new Error(
    "Vitest data root and worker ID must be set (see vitest.config.ts)",
  );
}

const resolvedRoot = path.resolve(dataRoot);
if (resolvedRoot === path.resolve(process.cwd(), "data")) {
  throw new Error(
    "Vitest refused to use the default repo data/ directory; isolation misconfigured",
  );
}

const resolved = path.join(resolvedRoot, `worker-${workerId}`);
fs.mkdirSync(resolved, { recursive: true });
process.env.JOB_TRACKER_DATA_DIR = resolved;
