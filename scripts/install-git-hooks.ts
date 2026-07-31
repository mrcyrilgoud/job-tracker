import fs from "node:fs";
import path from "node:path";

const projectRoot = process.cwd();
const sourceHook = path.join(projectRoot, "scripts/githooks/post-push");
const gitDir = path.join(projectRoot, ".git");
const hooksDir = path.join(gitDir, "hooks");
const targetHook = path.join(hooksDir, "post-push");

if (!fs.existsSync(gitDir) || !fs.statSync(gitDir).isDirectory()) {
  throw new Error(`No .git directory at ${gitDir}. Run from the repo root.`);
}

if (!fs.existsSync(sourceHook)) {
  throw new Error(`Missing hook template: ${sourceHook}`);
}

fs.mkdirSync(hooksDir, { recursive: true });
fs.copyFileSync(sourceHook, targetHook);
fs.chmodSync(targetHook, 0o755);

console.log(`Installed ${targetHook}`);
console.log("Copied from scripts/githooks/post-push (no git config changes).");
