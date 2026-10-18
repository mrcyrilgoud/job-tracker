import { randomUUID } from "node:crypto";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

export interface CheckoutSnapshot {
  branch: string;
  commit: string;
}

export function checkoutSnapshot(repoRoot: string): CheckoutSnapshot {
  const branch = execFileSync("git", ["symbolic-ref", "--quiet", "--short", "HEAD"], {
    cwd: repoRoot,
    encoding: "utf8",
  }).trim();
  const commit = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: repoRoot,
    encoding: "utf8",
  }).trim();
  return { branch, commit };
}

export function assertCleanCheckout(repoRoot: string): void {
  const status = execFileSync("git", ["status", "--porcelain", "--untracked-files=all"], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  if (status.trim()) {
    throw new Error("Checkout is dirty; commit or stash changes, then rerun app:rebuild.");
  }
}

export function assertCheckoutUnchanged(repoRoot: string, expected: CheckoutSnapshot): void {
  assertCleanCheckout(repoRoot);
  const actual = checkoutSnapshot(repoRoot);
  if (actual.branch !== expected.branch || actual.commit !== expected.commit) {
    throw new Error("Checkout changed during rebuild; the pending rebuild was left queued.");
  }
}

export function enqueueRebuild(queueDir: string, commit: string, branch: string): string {
  fs.mkdirSync(queueDir, { recursive: true });
  const requestPath = path.join(queueDir, `${Date.now()}-${randomUUID()}.json`);
  fs.writeFileSync(requestPath, JSON.stringify({ commit, branch, requestedAt: new Date().toISOString() }), {
    flag: "wx",
  });
  return requestPath;
}

export function readRebuildRequest(requestPath: string): { commit?: string; branch?: string; requestedAt?: string } {
  return JSON.parse(fs.readFileSync(requestPath, "utf8")) as {
    commit?: string;
    branch?: string;
    requestedAt?: string;
  };
}

export function pendingRebuildRequests(queueDir: string): string[] {
  if (!fs.existsSync(queueDir)) return [];
  return fs.readdirSync(queueDir)
    .filter((entry) => entry.endsWith(".json"))
    .sort()
    .map((entry) => path.join(queueDir, entry));
}

export function clearRebuildRequests(requests: string[]): void {
  for (const request of requests) {
    try {
      fs.unlinkSync(request);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
  }
}

export function acquireDirectoryLock(lockDir: string): string | null {
  fs.mkdirSync(path.dirname(lockDir), { recursive: true });
  const token = randomUUID();
  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      fs.mkdirSync(lockDir);
      fs.writeFileSync(path.join(lockDir, "owner.json"), JSON.stringify({ pid: process.pid, token }), {
        flag: "wx",
      });
      return token;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
      let owner: { pid?: number } | undefined;
      try {
        owner = JSON.parse(fs.readFileSync(path.join(lockDir, "owner.json"), "utf8")) as { pid?: number };
      } catch {
        let ageMs: number;
        try {
          ageMs = Date.now() - fs.statSync(lockDir).mtimeMs;
        } catch (statError) {
          if ((statError as NodeJS.ErrnoException).code === "ENOENT") continue;
          throw statError;
        }
        if (ageMs < 5_000) return null;
      }
      if (owner?.pid && isPidAlive(owner.pid)) return null;
      const staleDir = `${lockDir}.stale-${randomUUID()}`;
      try {
        fs.renameSync(lockDir, staleDir);
        fs.rmSync(staleDir, { recursive: true, force: true });
      } catch (renameError) {
        if ((renameError as NodeJS.ErrnoException).code !== "ENOENT") throw renameError;
      }
    }
  }
  try {
    fs.mkdirSync(lockDir);
    fs.writeFileSync(path.join(lockDir, "owner.json"), JSON.stringify({ pid: process.pid, token }), {
      flag: "wx",
    });
    return token;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "EEXIST") return null;
    throw error;
  }
}

export function releaseDirectoryLock(lockDir: string, token: string): void {
  try {
    const owner = JSON.parse(fs.readFileSync(path.join(lockDir, "owner.json"), "utf8")) as { token?: string };
    if (owner.token !== token) return;
    const releasingDir = `${lockDir}.release-${token}`;
    fs.renameSync(lockDir, releasingDir);
    fs.rmSync(releasingDir, { recursive: true, force: true });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
}

function isPidAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}
