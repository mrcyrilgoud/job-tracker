# Job Tracker

Personal local Mac app for tracking job applications, resumes/cover letters, company ATS watches, and Gmail updates.

The app is a **Tauri 2** desktop shell (`desktop/` UI + `src-tauri/` Rust backend).

## Requirements

- macOS 11+
- Node 20+
- Rust (stable) + Xcode Command Line Tools

```bash
# once
npm install
npm --prefix desktop install
rustup show   # or install from https://rustup.rs
```

## Quick start (Tauri)

```bash
npm run tauri:dev
# alias: npm run dev
```

Packaged `.app` (build only):

```bash
npm run tauri:build
# → src-tauri/target/release/bundle/macos/Job Tracker.app
```

Refresh the packaged app **and** retarget the jobs LaunchAgent:

```bash
npm run app:rebuild
```

Optional one-time setup so a successful `git push` of app code starts a background rebuild:

```bash
npm run hooks:install
```

Day-to-day UI/Rust iteration still uses `npm run tauri:dev`; `app:rebuild` is for keeping the packaged `.app` (and LaunchAgent) current.

## Data directory

| Mode | Path |
| --- | --- |
| Dev | repo `data/` (or `JOB_TRACKER_DATA_DIR`) |
| Release (no env override) | `~/Library/Application Support/com.jobtracker.local/` |

Override explicitly:

```bash
export JOB_TRACKER_DATA_DIR="/absolute/path/to/job-tracker/data"
```

Contents:

- `job-tracker.db` (+ WAL/SHM)
- `documents/`
- `jobs.csv` + `jobs.csv.sync.json`
- `jobs-worker.log`, `jobs-runner.lock`

Release builds may one-time migrate from a legacy repo `data/` tree into Application Support using a WAL checkpoint + copy of `db`/`-wal`/`-shm`, `documents/`, and CSV files (only when the destination DB is missing).

## Features

1. **Jobs** — paste a posting URL, set status/applied date/notes, check whether the posting is still active
2. **Jobs CSV** — `jobs.csv` mirrors editable fields; rewritten after changes and by the jobs runner
3. **Documents** — import PDF/DOCX/TXT; open with the system viewer (`shell.open`), not raw web paths
4. **Companies / watches** — Greenhouse, Lever, Ashby board sync; careers pages produce review items
5. **Gmail** — readonly OAuth with PKCE; refresh token in macOS Keychain (`job-tracker-local` / `gmail-refresh-token`)

### Jobs CSV

- Editable columns: `url`, `title`, `company`, `status`, `applied_at`, `notes`, `location`, `latest_note`
- Conflicts (merge mode): DB wins when both sides changed since last export
- Blank `id` + url/title/company creates a job; missing CSV rows do not delete jobs

## Background worker

Hourly work must run even when the UI is closed. Prefer a LaunchAgent that invokes the **packaged binary**:

```bash
npm run app:rebuild
# or: npm run tauri:build && npm run jobs:install
launchctl unload ~/Library/LaunchAgents/com.jobtracker.local.jobs.plist 2>/dev/null
launchctl load ~/Library/LaunchAgents/com.jobtracker.local.jobs.plist
```

The installer unloads any previous `com.jobtracker.local.jobs` plist, then retargets it to:

```text
…/job-tracker --run-jobs --data-dir <JOB_TRACKER_DATA_DIR or repo data/>
```

One-shot from the repo:

```bash
npm run jobs
# or: cargo run --manifest-path src-tauri/Cargo.toml -- --run-jobs
```

The runner is single-instance (flock on `jobs-runner.lock`) and emits `jobs-runner-progress` events when started from the UI.

## Gmail setup

Default OAuth is **loopback**, not a custom URL scheme:

1. Create a Google Cloud OAuth client (Desktop recommended)
2. Add a redirect URI of the form `http://127.0.0.1:<port>/callback`  
   The app binds an ephemeral port and shows the exact redirect URI after you click Connect — add that URI (or a small port range) in Google Cloud.
3. In the app’s **Gmail** screen, paste client ID/secret, then Connect (opens the system browser)

Keychain service/account: `job-tracker-local` / `gmail-refresh-token`.  
If a notarized/sandboxed build cannot see an older Keychain item, reconnect Gmail once.

Optional env vars:

```bash
GMAIL_CLIENT_ID=...
GMAIL_CLIENT_SECRET=...
# GMAIL_REDIRECT_URI is usually set automatically to the loopback listener
JOB_TRACKER_DATA_DIR=/absolute/path/to/data
```

## Tests

```bash
npm test                 # Rust unit tests (CSV, classify, safe_fetch, ATS parsers, …)
npm run test:desktop     # Desktop Vitest suite
npm run desktop:build    # Vite UI typecheck + build
```

## Scripts

| Command | Purpose |
| --- | --- |
| `npm run tauri:dev` | Run the Mac app (Vite + Rust) |
| `npm run tauri:build` | Produce `Job Tracker.app` |
| `npm run app:rebuild` | Build packaged app, retarget LaunchAgent, notify; quit/relaunch if running |
| `npm run hooks:install` | Install local post-push hook (background rebuild on app-code pushes) |
| `npm run jobs` | One-shot posting / ATS / careers / Gmail / CSV cycle |
| `npm run jobs:install` | Write/retarget the LaunchAgent plist |
| `npm test` | Rust unit tests |
| `npm run test:desktop` | Desktop UI unit tests |

`app:rebuild` flags: `--skip-jobs` (build only), `--background` (used by the post-push hook). Concurrent rebuilds use `data/rebuild.lock` and fail clearly if one is already running. Log: `data/rebuild.log`.

## Repo layout

```text
desktop/          Vite + React + Tailwind SPA
src-tauri/        Rust/Tauri backend (rusqlite, keyring, reqwest)
scripts/          LaunchAgent + rebuild + git hook installers
data/             Local SQLite + documents + CSV (gitignored)
```
