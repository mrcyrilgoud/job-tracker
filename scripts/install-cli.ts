import { execSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";

const projectRoot = process.cwd();

function resolveBinary(): string {
  const releaseApp = path.join(
    projectRoot,
    "src-tauri/target/release/bundle/macos/Job Tracker.app/Contents/MacOS/job-tracker",
  );
  const releaseBin = path.join(projectRoot, "src-tauri/target/release/job-tracker");
  const debugBin = path.join(projectRoot, "src-tauri/target/debug/job-tracker");

  for (const bin of [releaseApp, releaseBin, debugBin]) {
    if (fs.existsSync(bin)) {
      return bin;
    }
  }

  console.log("No compiled binary found. Building debug binary...");
  execSync("cargo build --manifest-path src-tauri/Cargo.toml", { stdio: "inherit" });
  return debugBin;
}

export function installSymlink(
  targetBin: string,
  aliasName: string,
  installDir: string,
  projectRoot = process.cwd(),
) {
  const destPath = path.join(installDir, aliasName);
  const target = path.resolve(targetBin);
  const managedTargets = [
    "/Applications/Job Tracker.app/Contents/MacOS/job-tracker",
    path.join(
      projectRoot,
      "src-tauri/target/release/bundle/macos/Job Tracker.app/Contents/MacOS/job-tracker",
    ),
    path.join(projectRoot, "src-tauri/target/release/job-tracker"),
    path.join(projectRoot, "src-tauri/target/debug/job-tracker"),
  ].map((candidate) => path.resolve(candidate));

  try {
    const existing = fs.lstatSync(destPath);
    if (!existing.isSymbolicLink()) {
      throw new Error(`Refusing to replace existing non-symlink: ${destPath}`);
    }
    const existingTarget = path.resolve(installDir, fs.readlinkSync(destPath));
    if (existingTarget !== target) {
      if (!managedTargets.includes(existingTarget)) {
        throw new Error(
          `Refusing to replace unrelated symlink: ${destPath} -> ${existingTarget}`,
        );
      }
      fs.unlinkSync(destPath);
    } else {
      console.log(`✓ ${aliasName} already points to ${target}`);
      return;
    }
  } catch (error) {
    if (!(error instanceof Error) || !("code" in error) || error.code !== "ENOENT") {
      throw error;
    }
  }

  fs.symlinkSync(target, destPath);
  console.log(`✓ Symlinked ${aliasName} -> ${target}`);
  console.log(`  Installed to ${destPath}`);
}

function main() {
  const targetBin = resolveBinary();
  const userLocalBin = path.join(os.homedir(), ".local", "bin");
  fs.mkdirSync(userLocalBin, { recursive: true });

  installSymlink(targetBin, "jt", userLocalBin);
  installSymlink(targetBin, "job-tracker", userLocalBin);

  const envPath = process.env.PATH || "";
  const isLocalBinInPath = envPath.split(path.delimiter).some((p) => path.resolve(p) === userLocalBin);

  console.log("\n==========================================");
  console.log("Job Tracker CLI installed successfully!");
  console.log("==========================================");
  if (!isLocalBinInPath) {
    console.log(`\nNote: ${userLocalBin} is not in your current PATH.`);
    console.log("Add it to your shell config (~/.zshrc or ~/.bashrc):");
    console.log(`  export PATH="$HOME/.local/bin:$PATH"\n`);
  }
  console.log("Try running:");
  console.log("  jt list");
  console.log("  jt stats");
  console.log("  jt --help\n");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
