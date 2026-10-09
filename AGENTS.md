# Agent notes

This is a **Tauri 2** desktop app:

- `desktop/` — Vite + React UI
- `src-tauri/` — Rust backend (SQLite, jobs runner, ATS/careers monitoring)
- `scripts/install-launchd.ts` — macOS LaunchAgent installer for the hourly jobs runner

Do not reintroduce a Next.js or Node server stack. Prefer Rust for backend logic and the desktop Vite app for UI.

There is **no** Job Tracker MCP server and **no** in-app LLM/chat. External agents (Cursor, Claude Code, Grok Bot via local Shell, scripts) automate through the **CLI only**.

## Agent ops (CLI contract)

### Binary and install

```bash
npm run cli:install   # symlinks ~/.local/bin/jt and job-tracker → packaged app binary
jt --help
```

Installed `jt` points at the **release** `Job Tracker.app` binary. Prefer global `jt` over `npm run cli` when talking to the same database the GUI uses.

### Which database (critical)

Resolution order (`src-tauri/src/db/paths.rs`):

1. `--data-dir <path>` or `JOB_TRACKER_DATA_DIR` (always wins)
2. Debug/`cargo` builds: repo `data/` when found
3. Release / packaged `jt`: `~/Library/Application Support/com.jobtracker.local/`

The live Mac GUI, packaged `jt`, and the hourly LaunchAgent (`com.jobtracker.local.jobs`, installed by `npm run jobs:install`) all use Application Support. Repo `data/` is dev-only (plus `rebuild.log`/`rebuild.lock`); release `--run-jobs` refuses a data dir inside a git repo unless `--allow-dev-data` is passed. Repo `data/job-tracker.db` is a separate tree and can be stale. **Default for agent work against the real pipeline:**

```bash
# packaged jt already defaults here; only set if you need an override
export JOB_TRACKER_DATA_DIR="$HOME/Library/Application Support/com.jobtracker.local"
jt stats --json
```

Or pass `--data-dir` on every command. Do not assume cwd/`data/` matches the GUI.

### Machine-readable I/O

Always pass **`--json`** for automation. Use **`-q` / `--quiet`** when you only want the payload (less status noise on stderr/stdout).

### Commands agents actually use

| Goal | Example |
| --- | --- |
| Pipeline overview | `jt stats --json` |
| Search / de-dupe | `jt list --search "Stripe" --json` |
| Filter status | `jt list --status wishlist --json` |
| Detail + history | `jt get <id_or_url> --json` |
| Add from URL | `jt add "<url>" --status wishlist --notes "…" --json` |
| Status / applied date | `jt update <id> --status applied --applied-at today --json` |
| Append note | `jt note <id> "Recruiter screen Friday" --json` |
| Watch triage | `jt watches list --new-only --json` |
| Full sync cycle | `jt sync --json` (or `--run-jobs`) |

Do **not** pass `-f` / `--favorite` unless the human explicitly asked to favorite. Prefer leaving favorites for manual UI use.

### What not to build against

- No localhost HTTP API, Unix socket, or MCP for Job Tracker today
- Do not reintroduce a Node/Next server so agents can `fetch` the DB
- GUI path is Tauri `invoke` only (`desktop/src/lib/api.ts`); agents should not try to drive the WebView

Handlers live in `src-tauri/src/cli/` and call the same services as Tauri IPC, so CLI mutations also refresh the CSV mirror.
