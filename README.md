# Job Tracker

Personal local Mac app for tracking job applications, resumes/cover letters, and company ATS watches.

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

Optional one-time setup so a `git commit` that touches app code starts a background rebuild:

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

1. **Jobs** — paste a posting URL, set status/applied date/notes, rate overall appeal, check whether the posting is still active
2. **Jobs CSV** — `jobs.csv` mirrors editable fields; rewritten after changes and by the jobs runner
3. **Documents** — import PDF/DOCX/TXT; open with the system viewer (`shell.open`), not raw web paths
4. **Companies / watches** — Greenhouse, Lever, Ashby board sync; careers pages produce review items
5. **Settings** — choose a CSV mirror path and tune watch/location preferences

### Jobs CSV

- Editable columns: `url`, `title`, `company`, `status`, `applied_at`, `notes`, `location`, `latest_note`, `appeal`
- `appeal` is one overall score from 1 to 5 (**5 = most appealing**, 1 = least), covering salary, career growth, industry, role fit, and anything else that matters. Leave the cell blank for an unscored job. There is no default score, and a CSV that omits the column does not clear a score already stored in the app.
- Conflicts (merge mode): DB wins when both sides changed since last export
- Blank `id` + url/title/company creates a job; missing CSV rows do not delete jobs
- By default, the mirror is `<data directory>/jobs.csv`. In **Settings**, choose another local
  `.csv` file to keep that mirror elsewhere; the app and background worker use the same saved
  location. Its `.sync.json` and `.lock` companion files live beside the selected CSV.

## Background worker

Hourly work must run even when the UI is closed. Prefer a LaunchAgent that invokes the **packaged binary**:

```bash
npm run app:rebuild
# or: npm run tauri:build && npm run jobs:install
launchctl list | grep com.jobtracker.local.jobs   # verify it is loaded
```

The installer unloads any previous `com.jobtracker.local.jobs` plist, rewrites it, and loads it (`launchctl load -w`; a load failure is reported with the manual command). Binary preference: `/Applications/Job Tracker.app/Contents/MacOS/job-tracker`, then the repo release bundle, release bin, debug bin:

```text
…/job-tracker --run-jobs --data-dir ~/Library/Application Support/com.jobtracker.local
```

The LaunchAgent, the GUI and packaged `jt` all use the **same Application Support DB**. Repo `data/` is dev-only. `JOB_TRACKER_DATA_DIR` overrides the installer default (it warns if that path is inside the repo), and release builds refuse `--run-jobs` against a data dir inside a git repo unless `--allow-dev-data` / `JOB_TRACKER_ALLOW_DEV_DATA=1` is given. `app:rebuild` keeps `rebuild.log`/`rebuild.lock` in repo `data/` but does not pass that path to `jobs:install`.

One-shot from the repo:

```bash
npm run jobs
# or: cargo run --manifest-path src-tauri/Cargo.toml -- --run-jobs
```

The runner is single-instance (flock on `jobs-runner.lock`) and emits `jobs-runner-progress` events when started from the UI.

Optional env vars:

```bash
JOB_TRACKER_DATA_DIR=/absolute/path/to/data
```

## Tests

```bash
npm test                 # Rust unit tests (CSV, safe_fetch, ATS parsers, …)
npm run test:desktop     # Desktop Vitest suite
npm run desktop:build    # Vite UI typecheck + build
```

## Scripts

| Command | Purpose |
| --- | --- |
| `npm run tauri:dev` | Run the Mac app (Vite + Rust) |
| `npm run tauri:build` | Produce `Job Tracker.app` |
| `npm run app:rebuild` | Build packaged app, retarget LaunchAgent, notify; quit/relaunch if running |
| `npm run hooks:install` | Install local post-commit hook (background rebuild on app-code commits) |
| `npm run cli -- <cmd>` | Run Job Tracker CLI subcommands (e.g. `npm run cli -- list`, `stats`) |
| `npm run cli:install` | Symlink `jt` and `job-tracker` into `~/.local/bin/` for global terminal access |
| `npm run jobs` | One-shot posting / ATS / careers / CSV cycle |
| `npm run jobs:install` | Write/retarget the LaunchAgent plist |
| `npm test` | Rust unit tests |
| `npm run test:desktop` | Desktop UI unit tests |

## CLI / Terminal Usage

Job Tracker’s agent surface is the **CLI only** (no MCP server, no in-app chat). Terminal users and AI agents (Cursor, Claude Code, Grok Bot via local Shell, scripts) share the same `jt` binary as the packaged Mac app. Longer agent notes live in [`AGENTS.md`](AGENTS.md).

```bash
# Install global jt / job-tracker → packaged Job Tracker.app binary
npm run cli:install

# List & search (agents: always add --json)
jt list --status interviewing
jt list --search "Staff" --favorites --json
jt list --search "Remote" --json                 # Also searches location, careers URL, and job history
jt watches list --search "distributed systems" --new-only --json

# Pipeline overview
jt stats --json

# View job details and event history
jt get <job_id_or_url> --json

# Add job by URL (auto-scrapes title, company, location)
jt add "https://boards.greenhouse.io/stripe/jobs/12345" --status wishlist --notes "Referred by Alex" --json

# Update status or log notes
jt update <job_id> --status applied --applied-at today --json
jt note <job_id> "Recruiter phone screen scheduled for Friday" --json

# Overall appeal: 1 = least appealing, 5 = most. Blank means unscored.
jt update <job_id> --appeal 5 --json
jt update <job_id> --clear-appeal --json

# ATS Watch triage
jt watches list --new-only --json
jt watches save <job_id>
jt watches dismiss <job_id>
```

### Data directory for CLI vs GUI

Packaged `jt` defaults to the **same** release data dir as the GUI:

`~/Library/Application Support/com.jobtracker.local/`

Override with `--data-dir` or `JOB_TRACKER_DATA_DIR`. Debug/`cargo run` builds prefer the repo `data/` folder when present — that is **not** the live Application Support DB. Agents targeting the real pipeline should use packaged `jt` (or set `JOB_TRACKER_DATA_DIR` explicitly).

```bash
export JOB_TRACKER_DATA_DIR="$HOME/Library/Application Support/com.jobtracker.local"
jt list --status wishlist --json
```

`app:rebuild` flags: `--skip-jobs` (build only), `--background` (used by the post-commit hook). Concurrent rebuilds use `data/rebuild.lock` and fail clearly if one is already running. Log: `data/rebuild.log`.

## Repo layout

```text
desktop/          Vite + React + Tailwind SPA
src-tauri/        Rust/Tauri backend (rusqlite, reqwest)
scripts/          LaunchAgent + rebuild + git hook installers
data/             Local SQLite + documents + CSV (gitignored)
```

## Code walkthrough

For a line-oriented explanation of the Tauri startup, React pages, Rust command bridge,
SQLite schema, integrations, CSV synchronization, CLI, and hourly worker, see
[`docs/CODE_WALKTHROUGH.md`](docs/CODE_WALKTHROUGH.md).
