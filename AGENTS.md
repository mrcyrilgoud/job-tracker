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
| Application ease (1–5, 5 = easiest; null if unset) | `jt update <id> --application-ease 4 --json` |
| Company stability (company-wide, 1–5, 5 = most stable; null if unscored) | `jt update <id> --company-stability 4 --json` |
| Employment stability (company-wide, 1–5, 5 = most secure; null if unscored) | `jt update <id> --employment-stability 4 --json` |
| Apply a Score_File | `jt scores import <path> --yes --json` |
| Permanently delete | `jt delete <id> [<id>...] --yes --json` |
| Append note | `jt note <id> "Recruiter screen Friday" --json` |
| Watch triage | `jt watches list --new-only --json` |
| Full sync cycle | `jt sync --json` (or `--run-jobs`) |

`closed` is a normal persisted status: setting it (GUI, `jt update --status closed`, or a CSV row) keeps the job, records a `status_changed` event, preserves history, round-trips through CSV, and is counted in stats / `jt list --status closed`. It no longer deletes the job. Archiving (`archived`) is separate. Permanent deletion is `jt delete <id> --yes` (or the explicit Delete action in the app). Pass `--yes` so the command does not prompt; a missing or ambiguous id deletes nothing and exits nonzero.

Do **not** pass `-f` / `--favorite` unless the human explicitly asked to favorite. Prefer leaving favorites for manual UI use.

Application ease is a per-job score for how easy that posting is to apply to: `jt update <id> --application-ease 1..5 --json`. `5` is easiest. Omit the flag to leave the stored score unchanged. `--clear-application-ease` sets it back to null. `jt get` and `jt list --json` return it as `applicationEase` (`null` when unscored). It does not favorite the job.

Company stability is a company-wide score for whether the employer can keep operating: `jt update <id> --company-stability 1..5 --json`. `1` is least stable and `5` is most stable; any other value is rejected. The value applies to every job at that company, including archived and closed jobs. Omit the flag to leave it unchanged. `--clear-company-stability` clears the company score and all of its metadata. `--company-stability-rationale <text>` sets trimmed rationale; `--clear-company-stability-rationale` clears only that rationale while keeping the score. `jt list --min-company-stability <1..5> --json` returns only jobs whose company score is at least the threshold; unscored companies are excluded while the filter is set.

Employment stability is a company-wide score for how secure a typical engineering hire is: `jt update <id> --employment-stability 1..5 --json`. `1` is least secure and `5` is most secure; only integers 1 through 5 are accepted and any other value is rejected. The value applies to every job at that company, and there is no employment score column on `jobs`. Omit the flag to leave it unchanged. `--clear-employment-stability` clears the employment score and all of its metadata. `--employment-stability-rationale <text>` sets trimmed rationale; `--clear-employment-stability-rationale` clears only that rationale. `jt list --min-employment-stability <1..5> --json` excludes jobs at companies below the threshold and unscored companies; combined with `--min-company-stability`, it returns only jobs meeting both minimums. Both score parsers accept digit-only values from 1 through 5, including `03`; signs, whitespace, decimals, empty values, and out-of-range values are rejected with the scale-specific error.

`jt get --json` and `jt list --json` return both job keys, `companyStability` and `employmentStability`, as integers or `null`; the value is shared by every job at that company. The `company` object in `jt get <id> --json` carries `companyStability`, `companyStabilityRationale`, `companyStabilityCitations`, `companyStabilityScoredAt`, `companyStabilityScoredBy`, `employmentStability`, `employmentStabilityRationale`, `employmentStabilityCitations`, `employmentStabilityScoredAt`, and `employmentStabilityScoredBy`; citations are arrays of URLs or `null`, and source is `manual`, `import`, or `null`. The CLI has no citation-editing flag. CLI score writes and successful score-file imports refresh the Jobs CSV mirror. Setting or clearing either score or either rationale does not favorite jobs or alter the per-job `Appeal` or `Application_Ease` scores.

`jt scores import <path> --json` validates a JSON score file before opening SQLite and prints a dry-run plan. Without `--yes` it writes nothing and exits 0; add `--yes` to apply valid matched entries. Only JSON input is accepted. The supported entry mappings are `score` → company stability, `rationale` → company stability rationale, `citations` → company stability citations, `employment_stability` → employment stability, `employment_rationale` → employment stability rationale, and `employment_citations` → employment stability citations. An omitted key preserves the stored value; JSON `null` clears it. Names are matched case-insensitively after trimming, through `tracker_names` first and then `name`. An entry matching more than one company, or a company matched by more than one entry, is reported as a conflict and none of those entries are written. Invalid entries are skipped while remaining entries continue; unmatched companies are reported and never create a company or job. Applied scores use `Scored_By` `import` and take `Scored_At` from `as_of` when it parses; an invalid or absent `as_of` uses import time and adds a warning. `signals`, `employment_researched`, `jobs`, `job_ids`, `rubric`, and unknown fields are ignored. The file must contain a top-level object with `companies`; more than 5,000 entries is rejected before database writes.

Rationale flags trim surrounding whitespace, allow at most 2,000 characters, and are rejected when their score is null. Clearing a score also clears its Rationale, Citation_List, Scored_At, and Scored_By. The CLI has no flag that edits citations. Importer JSON/quiet output has exactly `dryRun`, `applied`, `unchanged`, `unmatched`, `conflicts`, `invalid`, `warnings`, and `failed` keys; `-q` / `--quiet` prints only that payload. With `--yes`, each entry is transactional, later entries continue after a failed entry, and a successful write refreshes the CSV mirror. Malformed file-level input exits nonzero before database access. After a valid file is planned/applied, unmatched, conflict, invalid, and warning groups exit successfully; failed writes or CSV mirror refresh failures exit nonzero. Score operations leave `Appeal`, `Application_Ease`, and favorite flags untouched.

On ordinary non-JSON, non-quiet commands, each unreported `Backfill_Conflict` is printed once to stderr as `Backfill_Conflict: <company> (<id>) has conflicting company stability values: <values>`, then marked reported only after the output succeeds. JSON, `--quiet`, and `--run-jobs` suppress this report.

### What not to build against

- No localhost HTTP API, Unix socket, or MCP for Job Tracker today
- Do not reintroduce a Node/Next server so agents can `fetch` the DB
- GUI path is Tauri `invoke` only (`desktop/src/lib/api.ts`); agents should not try to drive the WebView

Handlers live in `src-tauri/src/cli/` and call the same services as Tauri IPC, so CLI mutations also refresh the CSV mirror.
