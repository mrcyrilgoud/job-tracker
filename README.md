# Job Tracker

Personal local app for tracking job applications, resumes/cover letters, company ATS watches, and Gmail updates.

## Quick start

```bash
npm install
npm run dev
```

Open [http://localhost:3000](http://localhost:3000).

Data is stored in a local SQLite database at `data/job-tracker.db`. Imported documents live in `data/documents/`. Both are gitignored.

## Features

1. **Jobs** — paste a posting URL, set status/applied date/notes, check whether the posting is still active
2. **Jobs CSV** — `data/jobs.csv` mirrors editable job fields; the app rewrites it after changes / `npm run jobs`, and you can edit it manually then import
3. **Documents** — import PDF/DOCX/TXT resumes and cover letters, attach them to applications
4. **Companies / watches** — validate and sync Greenhouse, Lever, and Ashby boards; careers pages produce review items on change
5. **Gmail** — readonly OAuth with PKCE; refresh token in macOS Keychain; ambiguous matches go to triage

### Jobs CSV

- File: `data/jobs.csv` (sidecar state in `data/jobs.csv.sync.json`)
- Export: `GET /api/jobs/csv` (add `?download=1` for attachment)
- Import: `POST /api/jobs/csv/import` with `{ "mode": "merge" }` or `"overwrite_editable"`; optional `dryRun: true`
- Status: `GET /api/jobs/csv/status`
- Editable columns: `url`, `title`, `company`, `status`, `applied_at`, `notes`, `location`, `latest_note`
- Conflicts (both app and CSV changed the same field since last export): DB wins; reported in the import response
- Blank `id` + url/title/company creates a new job; missing CSV rows do not delete jobs

## Background worker

Run once:

```bash
npm run jobs
```

Install the hourly macOS LaunchAgent:

```bash
npm run jobs:install
launchctl unload ~/Library/LaunchAgents/com.jobtracker.local.jobs.plist 2>/dev/null
launchctl load ~/Library/LaunchAgents/com.jobtracker.local.jobs.plist
```

## Gmail setup

1. Create a Google Cloud OAuth client (Desktop / Web) with redirect URI `http://localhost:3000/api/gmail/callback`
2. Add yourself as a test user while the app is in testing mode
3. Open **Gmail** in the app, paste the client ID/secret, then Connect

Optional env vars:

```bash
GMAIL_CLIENT_ID=...
GMAIL_CLIENT_SECRET=...
GMAIL_REDIRECT_URI=http://localhost:3000/api/gmail/callback
```

## Tests

```bash
npm test
```

## Scripts

| Command | Purpose |
| --- | --- |
| `npm run dev` | Start the local web app |
| `npm run jobs` | One-shot posting checks, ATS sync, careers checks, Gmail poll |
| `npm run jobs:install` | Write the LaunchAgent plist |
| `npm test` | Run unit/integration tests |
