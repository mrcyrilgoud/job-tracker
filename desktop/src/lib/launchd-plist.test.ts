import { describe, expect, it } from "vitest";

import {
  SYSTEM_APP_BINARY,
  renderPlist,
  resolveLaunchdConfig,
} from "../../../scripts/launchd-plist";

const home = "/Users/test";
const repo = "/Users/test/repos/Job-Tracker";
const appSupport = "/Users/test/Library/Application Support/com.jobtracker.local";
const repoBundle = `${repo}/src-tauri/target/release/bundle/macos/Job Tracker.app/Contents/MacOS/job-tracker`;
const debugBin = `${repo}/src-tauri/target/debug/job-tracker`;

describe("launchd plist", () => {
  it("defaults to Application Support data dir", () => {
    const cfg = resolveLaunchdConfig({ homeDir: home, projectRoot: repo, exists: (p) => p === repoBundle });
    expect(cfg.dataDir).toBe(appSupport);
    expect(cfg.programArguments).toEqual([repoBundle, "--run-jobs", "--data-dir", appSupport]);
    expect(cfg.logPath).toBe(`${appSupport}/jobs-worker.log`);
    expect(cfg.dataDirInsideRepo).toBe(false);
    const xml = renderPlist(cfg);
    expect(xml).toContain(`<string>${appSupport}</string>`);
    expect(xml).not.toContain(`${repo}/data`);
  });

  it("honors JOB_TRACKER_DATA_DIR override and flags repo paths", () => {
    const cfg = resolveLaunchdConfig({
      homeDir: home,
      projectRoot: repo,
      envDataDir: `${repo}/data`,
      exists: (p) => p === debugBin,
    });
    expect(cfg.dataDir).toBe(`${repo}/data`);
    expect(cfg.binary).toBe(debugBin);
    expect(cfg.dataDirInsideRepo).toBe(true);
  });

  it("prefers the /Applications binary", () => {
    const cfg = resolveLaunchdConfig({ homeDir: home, projectRoot: repo, exists: () => true });
    expect(cfg.binary).toBe(SYSTEM_APP_BINARY);
    expect(renderPlist(cfg)).toContain(`<string>${SYSTEM_APP_BINARY}</string>`);
  });

  it("throws when no binary exists", () => {
    expect(() => resolveLaunchdConfig({ homeDir: home, projectRoot: repo, exists: () => false })).toThrow(
      /binary not found/,
    );
  });
});
