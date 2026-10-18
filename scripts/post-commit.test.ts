import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { afterEach, test } from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const hookTemplate = path.join(repoRoot, "scripts/githooks/post-commit");
const tempDirs: string[] = [];
afterEach(() => {
  for (const dir of tempDirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});

function fixture(): { dir: string; npmLog: string; fakeBin: string } {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "job-tracker-post-commit-"));
  tempDirs.push(dir);
  const npmLog = path.join(dir, "npm.log");
  const fakeBin = path.join(dir, "bin");
  fs.mkdirSync(fakeBin);
  fs.mkdirSync(path.join(dir, "scripts/githooks"), { recursive: true });
  fs.copyFileSync(hookTemplate, path.join(dir, "scripts/githooks/post-commit"));
  fs.mkdirSync(path.join(dir, "desktop"), { recursive: true });
  execFileSync("git", ["init", "-q"], { cwd: dir });
  execFileSync("git", ["config", "user.email", "test@example.invalid"], { cwd: dir });
  execFileSync("git", ["config", "user.name", "Test"], { cwd: dir });
  const fakeNpm = path.join(fakeBin, "npm");
  fs.writeFileSync(fakeNpm, "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$TEST_NPM_LOG\"\n");
  fs.chmodSync(fakeNpm, 0o755);
  execFileSync("git", ["add", "scripts/githooks/post-commit", "bin/npm"], { cwd: dir });
  execFileSync("git", ["commit", "-q", "-m", "fixture"], { cwd: dir });
  return { dir, npmLog, fakeBin };
}

function commit(dir: string, file: string, content: string, message: string): void {
  const target = path.join(dir, file);
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(target, content);
  execFileSync("git", ["add", file], { cwd: dir });
  execFileSync("git", ["commit", "-q", "-m", message], { cwd: dir });
}

function runHook(dir: string, fakeBin: string, npmLog: string) {
  return spawnSync("bash", [path.join(dir, "scripts/githooks/post-commit")], {
    cwd: dir,
    encoding: "utf8",
    env: { ...process.env, PATH: `${fakeBin}:${process.env.PATH}`, TEST_NPM_LOG: npmLog },
  });
}

test("post-commit launches one background rebuild for an app-path commit", async () => {
  const { dir, fakeBin, npmLog } = fixture();
  commit(dir, "desktop/app.tsx", "export {};\n", "app change");
  const result = runHook(dir, fakeBin, npmLog);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /rebuild queued/);
  for (let i = 0; i < 30 && !fs.existsSync(npmLog); i++) await delay(25);
  assert.match(fs.readFileSync(npmLog, "utf8"), /run app:rebuild -- --background --request-commit/);
});

test("post-commit skips unrelated, dirty, and detached checkouts", () => {
  const unrelated = fixture();
  commit(unrelated.dir, "docs.md", "docs\n", "docs");
  assert.equal(runHook(unrelated.dir, unrelated.fakeBin, unrelated.npmLog).status, 0);
  assert.equal(fs.existsSync(unrelated.npmLog), false);

  const dirty = fixture();
  commit(dirty.dir, "desktop/app.tsx", "export {};\n", "app change");
  fs.writeFileSync(path.join(dirty.dir, "local.txt"), "uncommitted\n");
  const dirtyResult = runHook(dirty.dir, dirty.fakeBin, dirty.npmLog);
  assert.equal(dirtyResult.status, 0);
  assert.match(fs.readFileSync(path.join(dirty.dir, "data/rebuild.log"), "utf8"), /checkout is dirty/);
  assert.equal(fs.existsSync(dirty.npmLog), false);

  const detached = fixture();
  commit(detached.dir, "desktop/app.tsx", "export {};\n", "app change");
  execFileSync("git", ["checkout", "--detach", "-q", "HEAD"], { cwd: detached.dir });
  assert.equal(runHook(detached.dir, detached.fakeBin, detached.npmLog).status, 0);
  assert.match(fs.readFileSync(path.join(detached.dir, "data/rebuild.log"), "utf8"), /HEAD is detached/);
  assert.equal(fs.existsSync(detached.npmLog), false);
});

test("post-commit logs a clear skip when npm is unavailable", () => {
  const { dir, npmLog } = fixture();
  commit(dir, "desktop/app.tsx", "export {};\n", "app change");
  const minimalPath = fs.mkdtempSync(path.join(os.tmpdir(), "job-tracker-no-npm-"));
  tempDirs.push(minimalPath);
  const gitBin = execFileSync("which", ["git"], { encoding: "utf8" }).trim();
  fs.symlinkSync(gitBin, path.join(minimalPath, "git"));
  const result = spawnSync("/bin/bash", [path.join(dir, "scripts/githooks/post-commit")], {
    cwd: dir,
    encoding: "utf8",
    env: { ...process.env, PATH: `${minimalPath}:/bin:/usr/bin:/usr/sbin:/sbin`, TEST_NPM_LOG: npmLog },
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(fs.readFileSync(path.join(dir, "data/rebuild.log"), "utf8"), /npm was not found/);
  assert.equal(fs.existsSync(npmLog), false);
});
