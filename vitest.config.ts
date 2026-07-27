import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { defineConfig } from "vitest/config";

const testDataRoot = fs.mkdtempSync(path.join(os.tmpdir(), "job-tracker-vitest-"));
process.env.JOB_TRACKER_DATA_DIR = testDataRoot;

export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
    env: {
      JOB_TRACKER_DATA_DIR: testDataRoot,
    },
    setupFiles: ["./vitest.setup.ts"],
    globalTeardown: "./vitest.teardown.ts",
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
});
