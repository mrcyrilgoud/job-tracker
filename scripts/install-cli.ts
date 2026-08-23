import { execSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

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

function installSymlink(targetBin: string, aliasName: string, installDir: string) {
  const destPath = path.join(installDir, aliasName);
  try {
    if (fs.existsSync(destPath) || fs.lstatSync(destPath).isSymbolicLink()) {
      fs.unlinkSync(destPath);
    }
  } catch {
    // ignore
  }

  fs.symlinkSync(targetBin, destPath);
  console.log(`✓ Symlinked ${aliasName} -> ${targetBin}`);
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

main();
