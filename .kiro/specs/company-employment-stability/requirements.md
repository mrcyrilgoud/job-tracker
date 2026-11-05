# Requirements Document

## Introduction

Job Tracker stores one employer score today, `jobs.company_stability`, on each job (added by the company-stability-score spec). A separate research export holds per-company `score`, `rationale`, `citations`, `employment_stability`, `employment_rationale`, `employment_citations`, `employment_researched`, `tracker_names`, and `signals`, plus top-level `as_of` and `rubric`. That export lives outside the app.

This feature makes two employer scores first-class and stores them on the company:

- **Company stability**: can this employer keep operating (cash, stage, parent, distress)?
- **Employment stability**: how safe is a typical engineering hire there (layoffs, headcount trend, hire-then-cut)?

Each score is one integer per company (1–5, 5 is best, null is unscored) with optional rationale, citations, scored-at time, and scored-by source. Every job at the company shows the company's scores. `jobs.company_stability` stays as a write-through cache of the company value. Employment stability gets no job column. Users edit scores from the job detail page, the Jobs list, the company detail page, the CLI, and the Jobs CSV mirror, and load them in bulk with `jt scores import <path>`. Appeal and Application_Ease stay per-job scores about the posting and do not change.

Source of truth for scope: the working draft "Company stability and employment stability" dated 2026-11-02.

### Rubrics

Both scores are integers 1–5 or null. 5 is best on both scales. Null means "not scored" and differs from a low score. Thin evidence defaults to 3, and an imported 3 is a real 3.

Company stability (research field `score`):

| Score | Meaning |
| --- | --- |
| 5 | Most stable: public mega-cap, durable cash flow, strong parent |
| 4 | Strong: late-stage or well-capitalized private, unicorn, Series D+, about $100M or more raised in the last year, or clear revenue |
| 3 | Typical startup: Series A–C or large seed with known investors, no red flags; thin-evidence default |
| 2 | Elevated risk: small or early seed, stale funding, or layoffs plus weak fundraising |
| 1 | Least stable: distressed, large layoffs without recovery, corroborated shutdown signals, cannot fund operations |

Employment stability (research field `employment_stability`; the research file used 1–4 in practice, and 5 is still valid):

| Score | Meaning |
| --- | --- |
| 5 | Steady employment: no meaningful layoff record, stable or growing engineering headcount, no hire-then-cut |
| 4 | Secure with small exceptions: isolated, small, or dated cuts against a large or growing org; or rapidly growing and well funded with no layoff record |
| 3 | Ordinary risk for the stage: no public layoff record, or old cuts since outgrown; thin-evidence default, including when `employment_researched` is false |
| 2 | Elevated hire risk: documented layoff rounds, shrinking headcount, post-merger cuts, or a tiny team where a missed round means cuts |
| 1 | Least secure: repeated rounds, a large share cut, people let go within months of hire |

### Verification of "What exists today"

The draft's description of current behavior was checked against `src-tauri/src/db/migrate.rs`, `src-tauri/src/jobs/service.rs`, `src-tauri/src/jobs/csv.rs`, `src-tauri/src/companies.rs`, `src-tauri/src/cli/`, `desktop/src/lib/company-sidebar.ts`, `desktop/src/pages/JobsPage.tsx`, `desktop/src/components/JobDetailClient.tsx`, `desktop/src/components/JobsBoardView.tsx`, and `desktop/src/pages/CompanyDetailPage.tsx`.

Confirmed as described:

- `jobs.company_stability INTEGER` with `CHECK (company_stability IS NULL OR (typeof(company_stability) = 'integer' AND company_stability BETWEEN 1 AND 5))`; `appeal` and `application_ease` use the looser range-only CHECK.
- `companies` has only `id`, `name`, `careers_url`, `created_at`, `updated_at`. `find_or_create_company` matches `name = ?1 COLLATE NOCASE` on the trimmed name.
- `jt update --company-stability N` / `--clear-company-stability` write the one job. `jt list --min-company-stability N` excludes unscored jobs. The `jt list` table header is `STABILITY` and the detail line is `Stability:`.
- The Jobs page has the inline select, the `companyStability` sort field, and the "Minimum company stability" filter (URL parameter `minStability`). Board chips read `Stability N`.
- `CompanyRow` carries `stabilityAverage` and `stabilityScoredJobCount`, computed over Pipeline_Jobs. The `mostStable` sort is labeled "Most stable company" with the annotation label "Average stability".
- The CSV header ends with `company_stability`; a missing column keeps scores, a blank cell clears, an invalid cell skips the row.
- Employment stability does not exist anywhere.

Discrepancies and gaps the draft does not mention:

1. **Leading zeros.** The CLI parser `parse_company_stability` (`src-tauri/src/cli/args.rs`) accepts `"03"` as 3 (its unit test asserts this). It rejects signs, whitespace, decimals, and empty input. These requirements keep that exact behavior for both CLI scores, so "same strictness" means `"03"` is accepted.
2. **No company CLI command.** `jt` has no `companies` list or detail command. Company data reaches the CLI only through the `company` object inside `jt get --json` (`JobDetail.company`). The draft's "company list/detail JSON" is read here as that `company` object plus the Tauri `list_companies` rows. Adding a `jt companies` command is not in these requirements.
3. **No company edit path.** The company detail page renders `CompanyCard` in solo mode with watch and careers actions only. No IPC command updates a company's fields today. The scores editor and its IPC command are new.
4. **No migration version table.** Column presence is the only "applied" marker in `migrate.rs`. The one-time backfill and the one-time conflict report need their own persisted marker; the existing `app_settings` key/value table can hold it.
5. **Job detail save always sends `companyStability`.** `JobDetailClient` sends the current draft value on every save. Once the score lives on the company, an unrelated save would re-write the company score and bump its scored-at time. Requirement 3 makes an unchanged score value a no-op for metadata, and Requirement 8.4 sends a score only when it changed.
6. **CSV per-company disagreement.** The draft says a company fails when two of its rows carry different non-blank values. Read literally, editing one row's cell would always fail, because the company's other rows still show the old value. Requirement 9 compares only the cells that changed since the last export.
7. **Job company reassignment.** `update_job` can move a job to another company through `company_name` (creating the company when needed). The cache must follow the job to its new company (Requirement 2.5).
8. **Sidebar population changes.** The current average uses only Pipeline_Jobs. Sorting by the stored company integer means a company whose jobs are all archived still shows its score.
9. **Empty-company cleanup.** `update_job` and `delete_job` call `delete_untracked_empty_company` on the old company after a move or delete. That function deletes a company with no jobs, watches, careers snapshots, or reviews, which would silently discard a scored company's scores. Requirement 1.12 keeps a company row while either Company_Score is non-null.
10. **CLI and CSV parse score text differently.** The CSV cell parser (`parse_score_cell` in `src-tauri/src/jobs/csv.rs`) uses `i64` parsing on the trimmed cell, so it accepts `+3` and `03`. The CLI (Requirement 6.7) rejects `+3` and accepts `03`. The CSV keeps accepting `+3` (Requirement 9.5) by design, to preserve existing behavior.

Assumptions to confirm:

- **Score_File size cap.** Requirement 10.4 rejects a Score_File with more than 5000 entries in `companies`. The draft sets no limit; 5000 is a new assumption.

### Non-goals

- Continuous or in-app web research, scraping, or an LLM pass that assigns scores.
- A third "culture / caution" score.
- Per-job overrides of either company score.
- Creating companies or jobs from the research file.
- Persisting research `signals` (`funding`, `stage`, `headcount_trend`, `notes`) or `employment_researched` as columns.
- Changing what Appeal or Application_Ease mean, or combining any scores into one number.

### Out of scope for v1

- Scheduled or on-demand web research.
- A culture or caution score.
- Per-role employment scores.
- Fuzzy name matching, alias tables beyond `tracker_names`, and auto-creating companies.
- CLI citation editing.
- CSV input for `jt scores import` (v1 accepts JSON only).
- Dropping `jobs.company_stability`.
- Job events for score edits.
- A `jt companies` command.
- Any HTTP API, MCP server, Node/Next server, or in-app LLM.

## Glossary

- **Job_Tracker**: The Tauri 2 desktop application as a whole: the Rust backend in `src-tauri/`, the desktop UI in `desktop/`, and the `jt` CLI.
- **Database**: The SQLite database resolved by `src-tauri/src/db/paths.rs`.
- **Migrator**: The schema creation and migration code in `src-tauri/src/db/migrate.rs`.
- **Score_Service**: The Rust code that reads and writes Company_Scores on the `companies` table and maintains the Stability_Cache. Shared by Tauri IPC, the CLI, the CSV_Mirror, and the Score_Importer.
- **Job_Service**: The Rust job services in `src-tauri/src/jobs/service.rs` that create, read, list, sort, filter, page, update, and delete jobs.
- **IPC_Layer**: The Tauri commands in `src-tauri/src/commands/` called by the Desktop_UI through `desktop/src/lib/api.ts`.
- **CLI**: The `jt` / `job-tracker` command-line interface in `src-tauri/src/cli/`.
- **CSV_Mirror**: The Jobs CSV export and import code in `src-tauri/src/jobs/csv.rs`, including the sync sidecar baseline.
- **Score_Importer**: The `jt scores import <path>` command and the Rust code behind it.
- **Desktop_UI**: The Vite + React UI in `desktop/`.
- **Jobs_List**: The list view of the Jobs page (`desktop/src/pages/JobsPage.tsx`), including its sort control, filter panel, and per-row controls.
- **Jobs_Board**: The board view of the Jobs page (`desktop/src/components/JobsBoardView.tsx`).
- **Job_Detail_Form**: The job edit form in `desktop/src/components/JobDetailClient.tsx`.
- **Company_Detail_Page**: The company page in `desktop/src/pages/CompanyDetailPage.tsx`.
- **Company_Scores_Editor**: The new scores section on the Company_Detail_Page.
- **Companies_Sidebar**: The Companies sidebar on the Jobs page, with logic in `desktop/src/lib/company-sidebar.ts` and rows from `src-tauri/src/companies.rs`.
- **Appeal**: The existing per-job score, 1 (least appealing) to 5 (most appealing), or null.
- **Application_Ease**: The existing per-job score, 1 (hardest to apply) to 5 (easiest to apply), or null.
- **Valid_Score**: An integer from 1 to 5 inclusive.
- **Company_Stability**: The company-level score in `companies.company_stability`: a Valid_Score (1 = least stable, 5 = most stable) or null. JSON `companyStability`.
- **Employment_Stability**: The company-level score in `companies.employment_stability`: a Valid_Score (1 = least secure, 5 = most secure) or null. JSON `employmentStability`.
- **Company_Score**: Either Company_Stability or Employment_Stability.
- **Score_Metadata**: For one Company_Score, its rationale, citations, scored-at, and scored-by values. Each company has eight Score_Metadata columns (four per Company_Score); with the two score columns, the feature adds ten columns to `companies`.
- **Rationale**: Plain text of 1 to 2000 characters explaining a Company_Score, or null. JSON `companyStabilityRationale` / `employmentStabilityRationale`.
- **Citation_List**: A list of 1 to 20 absolute URLs whose scheme is `http` or `https`, stored as a JSON array string, or null when no list is stored. JSON `companyStabilityCitations` / `employmentStabilityCitations`, always an array or null, never a raw string.
- **Scored_At**: The timestamp of the last write that set a Company_Score, or null. Stored as a UTC RFC 3339 timestamp with millisecond precision and a `Z` suffix, except when taken from a Score_File's `as_of` (Requirement 10.10) or written by the Backfill (Requirement 4.3). JSON `companyStabilityScoredAt` / `employmentStabilityScoredAt`.
- **Scored_By**: `manual` or `import`, naming the source of the last write that set a Company_Score, or null. JSON `companyStabilityScoredBy` / `employmentStabilityScoredBy`.
- **Unscored_Company**: For one Company_Score, a company whose value for that score is null.
- **Stability_Cache**: The column `jobs.company_stability`, which holds a copy of the job's company's Company_Stability.
- **Cache_Fan_Out**: Setting the Stability_Cache on every job whose `company_id` is the company, including archived, closed, new-from-watch, and dismissed jobs, to the company's Company_Stability.
- **Company_Range_Message**: The error text "Company stability must be an integer from 1 to 5 (5 = most stable)".
- **Employment_Range_Message**: The error text "Employment stability must be an integer from 1 to 5 (5 = most secure)".
- **Score_Write**: One request that changes one or both Company_Scores or their Score_Metadata for one company.
- **Unchanged_Score_Write**: A Score_Write whose Company_Score value equals the stored value and whose Rationale and Citation_List are absent or equal to the stored values.
- **Backfill**: The one-time Migrator step that copies existing Stability_Cache values to `companies.company_stability`.
- **Backfill_Conflict**: A company whose jobs hold two or more distinct non-null Stability_Cache values when the Backfill runs.
- **Score_File**: A JSON file with a top-level object holding `as_of` and `companies[]`, in the research export shape.
- **Score_Entry**: One element of `companies[]` in a Score_File.
- **Name_Match**: Comparing a name to `companies.name` after trimming leading and trailing whitespace, case-insensitively and otherwise exactly, as `find_or_create_company` does. SQLite `COLLATE NOCASE` folds only ASCII letters, so non-ASCII letters must match case exactly.
- **Name_Order**: The company ordering used by `compareByNameOrder` in the Desktop_UI: locale-aware, case-insensitive company name, then company `id` by code unit.
- **Dry_Run**: A Score_Importer run without `--yes`, which reports the plan and writes nothing.
- **Pipeline_Job**: A job whose `status` is not `archived`, whose `is_new_from_watch` is 0, and whose `watch_disposition` is not `dismissed`.

## Requirements

### Requirement 1: Store both scores and their metadata on the company

**User Story:** As a job seeker, I want company stability and employment stability stored once per company with their reasoning, so that every job at that employer shows the same scores and I can see why each score was given.

#### Acceptance Criteria

1. WHEN the Migrator creates a new Database, THE Migrator SHALL create the `companies` table with its existing columns (`id`, `name`, `careers_url`, `created_at`, `updated_at`) plus ten nullable columns: `company_stability` and `employment_stability` as INTEGER, and `company_stability_rationale`, `company_stability_citations`, `company_stability_scored_at`, `company_stability_scored_by`, `employment_stability_rationale`, `employment_stability_citations`, `employment_stability_scored_at`, and `employment_stability_scored_by` as TEXT.
2. WHEN the Migrator runs against an existing Database whose `companies` table lacks one or more of the ten columns in criterion 1, THE Migrator SHALL add each missing column with the type and constraint given in criteria 1, 4, and 5, SHALL set the new column to null on every existing company, and SHALL leave every other company column and every job column (including the Stability_Cache) unchanged by the column-adding step.
3. WHEN the Migrator runs a second or later time against a Database that already has the ten columns, THE Migrator SHALL complete without error, SHALL leave exactly one instance of each of the ten columns in `companies`, and SHALL leave every stored company and job value unchanged.
4. THE Database SHALL constrain `companies.company_stability` and `companies.employment_stability` to null or an integer-typed Valid_Score, using the same type-strict CHECK as `jobs.company_stability` (`typeof(...) = 'integer'` and `BETWEEN 1 AND 5`). The constraint SHALL be identical whether the column came from table creation (criterion 1) or was added by migration (criterion 2).
5. THE Database SHALL constrain `companies.company_stability_scored_by` and `companies.employment_stability_scored_by` to null or one of the exact lowercase strings `manual` and `import`, whether the column came from table creation or was added by migration.
6. IF a write tries to store a Company_Score that is neither null nor an integer-typed Valid_Score (for example 0, 6, -1, 2.5, `'2.5'`, `'3'`, or `'high'`), THEN THE Database SHALL reject the whole statement with a constraint error and SHALL leave every column of the targeted company unchanged, including other columns set in the same statement.
7. IF a write tries to store a Scored_By value that is neither null, `manual`, nor `import` (for example `''`, `Manual`, `IMPORT`, or `csv`), THEN THE Database SHALL reject the whole statement with a constraint error and SHALL leave every column of the targeted company unchanged.
8. IF the Migrator fails while adding one of the ten columns, THEN THE Migrator SHALL return an error to its caller identifying the failed migration step, SHALL leave the failed column absent, SHALL keep every column added before the failure along with all previously stored data, and SHALL add the remaining missing columns the next time it runs against that Database.
9. THE Migrator SHALL add no `employment_stability` column, and no other Employment_Stability or Score_Metadata column, to the `jobs` table on new or existing Databases.
10. WHEN a company is created by any path (`find_or_create_company`, a Desktop_UI job add, `jt add`, a watch save, or a CSV_Mirror row), THE Job_Tracker SHALL store the new company with Company_Stability, Employment_Stability, and all eight Score_Metadata values null.
11. WHEN `find_or_create_company` resolves a name to an existing company by Name_Match, THE Job_Tracker SHALL leave that company's Company_Stability, Employment_Stability, and Score_Metadata unchanged.
12. WHEN a job move or job deletion leaves a company with no jobs, THE Job_Service SHALL immediately delete the company when both Company_Stability and Employment_Stability are null, matching the prior behavior exactly, and SHALL retain the company row when either Company_Score is non-null.

### Requirement 2: Keep the Stability_Cache equal to the company value

**User Story:** As a job seeker, I want every job at a company to show the same company stability, so that two postings at one employer never disagree.

#### Acceptance Criteria

1. WHEN the Score_Service commits a Score_Write that carries a Company_Stability value (a Valid_Score or a clear), THE Score_Service SHALL perform Cache_Fan_Out for that company in the same transaction as the company row change, including when the carried value equals the stored Company_Stability.
2. WHEN the Score_Service performs Cache_Fan_Out for a company, THE Score_Service SHALL leave the Stability_Cache of every job whose `company_id` is a different company unchanged.
3. WHEN the Score_Service performs Cache_Fan_Out, THE Score_Service SHALL keep each affected job's Appeal, Application_Ease, favorite flag, status, notes, `updated_at`, and every other column except the Stability_Cache unchanged, and SHALL record no job event.
4. WHEN the Job_Service inserts a job by any path (Desktop_UI add, `jt add`, watch sync save, CSV row), THE Job_Service SHALL set the new job's Stability_Cache, in the same transaction as the insert, to the company's Company_Stability as stored at that moment, which is null when the company is an Unscored_Company or was created by the same insert.
5. WHEN the Job_Service moves a job to a different company by any path (an `update_job` `company_name` change from the Desktop_UI, the CLI, or a CSV `company` cell), THE Job_Service SHALL set the moved job's Stability_Cache to the new company's Company_Stability in the same transaction as the move, and SHALL leave the Stability_Cache of every other job at the old and new companies unchanged, unless the same request also carries a Company_Stability value, in which case criterion 1 and Requirement 5.5 apply to the new company.
6. FOR ALL sequences of successful Score_Writes, job inserts, job company moves, job updates, and job deletions, each remaining job's Stability_Cache SHALL equal its company's Company_Stability afterward, with one exception: jobs that still belong to a Backfill_Conflict company that has had no successful Score_Write carrying a Company_Stability value since the Backfill (invariant property).
7. IF any Cache_Fan_Out statement fails, THEN THE Score_Service SHALL roll back the whole Score_Write, return an error to the caller, and leave the company row and every Stability_Cache value equal to their values before the write.
8. WHEN the Job_Tracker commits a job update that carries no Company_Stability field and does not move the job to a different company, or a Score_Write that carries no Company_Stability value (for example, Employment_Stability or Score_Metadata only), THE Job_Tracker SHALL leave every job's Stability_Cache unchanged.
9. IF setting the Stability_Cache fails during a job insert or a job company move, THEN THE Job_Service SHALL roll back the insert or move, return an error to the caller, and leave the job, its old company, its new company, and every Stability_Cache value as they were before the request.

### Requirement 3: Score_Write semantics and Score_Metadata rules

**User Story:** As a job seeker, I want setting, clearing, and annotating a score to behave the same from every surface, so that a score never shows stale reasoning.

#### Acceptance Criteria

1. WHEN a Score_Write sets a Company_Score to a Valid_Score that differs from the stored value, THE Score_Service SHALL store the score and set that score's Scored_At to the write time. The write time SHALL be a UTC RFC 3339 timestamp with millisecond precision and a `Z` suffix (for example `2026-11-02T08:15:30.123Z`), except where Requirement 10.10 supplies an `as_of` value. THE Score_Service SHALL set Scored_By to `import` for Score_Importer writes and to `manual` for every other surface (Desktop_UI, CLI, CSV_Mirror). THE Score_Service SHALL keep the score's Rationale and Citation_List unchanged unless the same Score_Write supplies them.
2. WHEN a Score_Write clears a Company_Score whose stored value is non-null, THE Score_Service SHALL set that score and its Rationale, Citation_List, Scored_At, and Scored_By to null together. WHEN a Score_Write clears a Company_Score that is already null, THE Score_Service SHALL succeed without error, change no company column, and count the write as unchanged.
3. WHEN a Score_Write supplies a Rationale or Citation_List for a Company_Score that is non-null after the write, THE Score_Service SHALL replace the stored Rationale or Citation_List with the supplied value after applying the normalization in criteria 7, 8, and 10.
4. IF a manual Score_Write supplies a Rationale or Citation_List that is still non-null after normalization (criteria 7, 8, and 10) for a Company_Score that is null after the write, THEN THE Score_Service SHALL reject the whole Score_Write with an error naming the Company_Score and stating that the score must be set first, and SHALL keep the company and every Stability_Cache unchanged.
5. WHEN a Score_Write is an Unchanged_Score_Write for a Company_Score, THE Score_Service SHALL keep that score's value, Rationale, Citation_List, Scored_At, and Scored_By unchanged and count the write as unchanged in any summary that reports counts (CLI output and Score_Importer output). This applies even when the write source differs from the stored Scored_By. Cache_Fan_Out per Requirement 2.1 still applies to an Unchanged_Score_Write that carries a Company_Stability value; it rewrites only job Stability_Cache values and changes no company column.
6. WHEN a Score_Write changes only the Rationale or Citation_List of a non-null Company_Score, THE Score_Service SHALL store the new Rationale or Citation_List and SHALL keep the score's value, Scored_At, and Scored_By unchanged.
7. WHEN the Score_Service receives a Rationale whose text is empty after trimming leading and trailing whitespace, THE Score_Service SHALL store the Rationale as null.
8. WHEN the Score_Service receives a Rationale that is non-empty after trimming, THE Score_Service SHALL store the trimmed text and keep interior whitespace and line breaks unchanged.
9. IF a Rationale is longer than 2000 characters after trimming, counted as Unicode code points, THEN THE Score_Service SHALL reject the whole Score_Write with an error stating the 2000-character limit and SHALL keep the company unchanged. A trimmed Rationale of exactly 2000 code points SHALL be accepted.
10. WHEN the Score_Service receives a Citation_List, THE Score_Service SHALL trim leading and trailing whitespace from each entry and keep the entries in the supplied order, including duplicates. IF the list has zero entries, THEN THE Score_Service SHALL store the Citation_List as null.
11. IF a supplied Citation_List has more than 20 entries, THE Score_Service SHALL reject the whole Score_Write with an error naming the 20-entry limit, even when the list also contains invalid entries. Otherwise, if any entry fails one of these checks after trimming: it is a string, it is 1 to 2048 characters long, and it is an absolute URL with scheme `http` or `https` and a non-empty host, THEN THE Score_Service SHALL reject the whole Score_Write with an error naming the first invalid entry by position and value, and SHALL keep the company unchanged.
12. IF a Score_Write carries a Company_Stability value that is not null and not a Valid_Score, THEN THE Score_Service SHALL reject the whole Score_Write with an error containing the Company_Range_Message and SHALL keep the company and every Stability_Cache unchanged.
13. IF a Score_Write carries an Employment_Stability value that is not null and not a Valid_Score, THEN THE Score_Service SHALL reject the whole Score_Write with an error containing the Employment_Range_Message and SHALL keep the company and every Stability_Cache unchanged.
14. WHEN a Score_Write changes one Company_Score, THE Score_Service SHALL keep the other Company_Score and its Score_Metadata unchanged.
15. WHEN a single Score_Write sets both Company_Scores, THE Score_Service SHALL give both scores the same Scored_At value.
16. THE Score_Service SHALL keep every job's Appeal, Application_Ease, and favorite flag unchanged during any Score_Write.
17. THE Score_Service SHALL record no job event for a Score_Write.
18. IF storing any column of a Score_Write fails, THEN THE Score_Service SHALL roll back the whole Score_Write, so that both Company_Scores, all Score_Metadata, and every Stability_Cache equal their values before the write, and SHALL return an error to the caller indicating that the score was not saved.
19. WHEN two Score_Writes to the same company are submitted concurrently, for example from the Desktop_UI and the CLI, THE Score_Service SHALL apply them one after the other. Afterward, every stored Company_Score and Score_Metadata value SHALL equal the result of applying the two writes fully, one at a time, in some order, with no fields mixed between the two writes.
20. FOR ALL companies with a null Company_Score, the stored Rationale, Citation_List, Scored_At, and Scored_By of that score SHALL be null (invariant property).
21. FOR ALL Valid_Scores and null, Rationales of 1 to 2000 code points with no leading or trailing whitespace, and Citation_Lists of 1 to 20 valid URLs with no leading or trailing whitespace, writing them through the Score_Service and reading the company back SHALL return the written values, with the Citation_List as an array in the same order (round-trip property).

### Requirement 4: Backfill company stability from existing job scores

**User Story:** As an existing user, I want my per-job stability scores moved to the company when they agree, so that I do not re-score companies by hand.

#### Acceptance Criteria

1. WHEN the Migrator runs against a Database that has no Backfill-run marker in `app_settings`, THE Migrator SHALL run the Backfill once, after the ten columns in Requirement 1 exist, and SHALL write the Backfill-run marker in the same transaction as the Backfill changes. This includes a new Database with no companies or jobs, where the Backfill changes nothing and still writes the marker.
2. WHEN the Backfill processes a company whose jobs all hold a null Stability_Cache, or that has no jobs, THE Migrator SHALL leave the company's Company_Stability and its Score_Metadata null.
3. WHEN the Backfill processes a company whose Company_Stability is null and whose jobs hold exactly one distinct non-null Stability_Cache value, THE Migrator SHALL set the company's Company_Stability to that value, Scored_By to `manual`, Scored_At to the Backfill's start time as an ISO-8601 UTC timestamp, and Rationale and Citation_List to null, and SHALL perform Cache_Fan_Out for that company. The Backfill counts the jobs of every status (pipeline, archived, closed, new-from-watch, and dismissed), and all companies scored in one Backfill share the same Scored_At.
4. WHEN the Backfill processes a company whose jobs hold two or more distinct non-null Stability_Cache values, THE Migrator SHALL leave the company's Company_Stability and Score_Metadata null, SHALL leave every Stability_Cache value of that company unchanged, and SHALL persist a Backfill_Conflict holding the company id, the company name, and the distinct values in ascending order. The Backfill_Conflict is persisted in the same transaction as the Backfill-run marker.
5. THE Migrator SHALL leave every company's Employment_Stability and its Score_Metadata null during the Backfill.
6. WHEN the Migrator runs on a Database that holds the Backfill-run marker, THE Migrator SHALL skip the Backfill and SHALL keep every Company_Score, Score_Metadata value, Stability_Cache value, and persisted Backfill_Conflict unchanged.
7. WHEN the CLI runs any command other than `--run-jobs`, without `-q` / `--quiet`, against a Database that holds persisted Backfill_Conflicts not yet marked as reported, THE CLI SHALL print to stderr one line per Backfill_Conflict, in company-name order, naming the company and its distinct values, and SHALL then mark the conflicts as reported so that later CLI runs print nothing. The marker is written only after printing succeeds.
8. WHILE the CLI runs with `--json`, `-q` / `--quiet`, or `--run-jobs`, THE CLI SHALL write nothing about Backfill_Conflicts to stdout, so that the JSON payload stays parseable. When the run uses `-q` / `--quiet` or `--run-jobs`, the CLI SHALL also print no Backfill_Conflict report to stderr and SHALL leave unreported Backfill_Conflicts unreported. The Desktop_UI SHALL not mark Backfill_Conflicts as reported.
9. WHEN a Company_Stability Score_Write succeeds for a Backfill_Conflict company, THE Score_Service SHALL perform Cache_Fan_Out, so that every job of that company holds the written value.
10. IF any Backfill statement fails, including when another process holds the Database write lock past the 5-second busy timeout, THEN THE Migrator SHALL roll back every Backfill change and any Backfill_Conflict rows, SHALL not write the Backfill-run marker, and SHALL return the error to the caller (the Desktop_UI startup, the CLI, or `--run-jobs`), so that the CLI exits with a nonzero status. The Migrator SHALL run the Backfill again on the next Migrator run.
11. WHEN two or more processes (the Desktop_UI, the CLI, and the `--run-jobs` LaunchAgent) run the Migrator against the same unmarked Database at the same time, THE Migrator SHALL apply the Backfill in exactly one process. Each other process SHALL find the marker after acquiring the write lock and SHALL change nothing.
12. FOR ALL Databases of companies and jobs with arbitrary Stability_Cache values and null Company_Stability, after the Backfill each company's Company_Stability SHALL equal the single distinct non-null value of its jobs when exactly one exists and null otherwise; each job's Stability_Cache SHALL equal its pre-Backfill value; the persisted Backfill_Conflicts SHALL be exactly the companies with two or more distinct non-null values; and a second Migrator run SHALL change nothing (model-based property).

### Requirement 5: Set and clear scores through job updates and IPC

**User Story:** As a job seeker, I want to change a company score from any job at that company, so that I can score while triaging postings.

#### Acceptance Criteria

1. THE Job_Service SHALL include both `companyStability` and `employmentStability` in every serialized job that job detail, job list, and job page responses return. Each field SHALL be set to the Valid_Score stored on the job's company, or to JSON null for an Unscored_Company. Each field SHALL be present in both cases.
2. WHEN the IPC_Layer receives an `update_job` request for an existing job and the request's `companyStability` or `employmentStability` field is a Valid_Score or JSON null, THE Job_Service SHALL apply each supplied field to the job's company as one manual Score_Write under the rules of Requirement 3. THE IPC_Layer SHALL then return the updated job detail, and that detail's job fields and `company` object SHALL hold the company's stored values after the write.
3. WHEN an `update_job` request omits `companyStability` or `employmentStability`, THE Job_Service SHALL keep the omitted Company_Score of the job's company and that score's Score_Metadata unchanged, including when the request changes other job fields.
4. WHEN an `update_job` request carries both `companyStability` and `employmentStability`, THE Job_Service SHALL apply both in the same transaction as the job's other field changes. If any part of the request fails, THE Job_Service SHALL write neither score and no job field.
5. WHEN an `update_job` request both moves the job to a different company (through `companyName`) and carries a Company_Score field, THE Job_Service SHALL create the named company as part of the same transaction when Name_Match finds no existing company, SHALL apply the Score_Write to the job's new company, and SHALL keep the previous company's Company_Scores, Score_Metadata, and the Stability_Cache of the previous company's remaining jobs unchanged. IF the request fails, THE Job_Service SHALL roll back the company creation and the entire update.
6. IF an `update_job` request carries a `companyStability` or `employmentStability` value that is neither JSON null nor a Valid_Score, THEN THE Job_Service SHALL reject the whole request and keep every field of the job, its current company, and every other company unchanged. Invalid values include an integer outside 1–5, a non-integer number such as 2.5, a string such as `"3"`, a boolean, an array, and an object. The rejection SHALL use the Company_Range_Message for `companyStability` and the Employment_Range_Message for `employmentStability`. THE Job_Service SHALL create no company, including when the same request names a new `companyName`.
7. IF an `update_job` request with a Company_Score field targets a job id that does not exist, THEN THE Job_Service SHALL reject the request with a "Job not found" error and SHALL change no company or job.
8. THE IPC_Layer SHALL provide a company Score_Write command that applies one manual Score_Write to a company by company id. For each Company_Score, its Rationale, and its Citation_List, the command SHALL keep an omitted field, clear a field sent as JSON null, and set a field sent with a value, under the rules of Requirement 3. On success, the command SHALL return the company with both Company_Scores and all Score_Metadata, with each Citation_List as an array or null.
9. IF the company Score_Write command targets a company id that does not exist, THEN THE IPC_Layer SHALL reject the request with a "Company not found" error and SHALL change no company or job.
10. IF the company Score_Write command receives a Company_Score value that is neither JSON null nor a Valid_Score, a Rationale that is not a string, or a Citation_List that is not an array of strings, THEN THE IPC_Layer SHALL reject the whole request and keep the company and every Stability_Cache unchanged. A bad score value SHALL be rejected with the matching Company_Range_Message or Employment_Range_Message. A bad Rationale or Citation_List SHALL be rejected with an error naming the invalid field.
11. THE IPC_Layer SHALL include both Company_Scores and all Score_Metadata, with each Citation_List as an array or null, in each `list_companies` row and in the `company` object of job detail responses. THE IPC_Layer SHALL remove `stabilityAverage` and `stabilityScoredJobCount` from `list_companies` rows.
12. WHEN an `update_job` request with a Company_Score field or a company Score_Write command succeeds, THE IPC_Layer SHALL mark the CSV_Mirror dirty in the same way as for a job update, so that after the next mirror refresh the CSV rows of every job at that company show the stored values.
13. IF an `update_job` request or a company Score_Write command is rejected, THEN THE IPC_Layer SHALL not mark the CSV_Mirror dirty on account of that request.

### Requirement 6: Company scores in the CLI

**User Story:** As an external agent or script, I want CLI flags and JSON fields for both company scores, so that I can score employers without driving the GUI.

#### Acceptance Criteria

1. WHEN a user runs `jt update <id> --company-stability <n>` with n a Valid_Score, THE CLI SHALL apply a manual Score_Write setting Company_Stability to n on the job's company, so that `jt get` on every job at that company reports `companyStability` n, and SHALL exit with status 0.
2. WHEN a user runs `jt update <id> --employment-stability <n>` with n a Valid_Score, THE CLI SHALL apply a manual Score_Write setting Employment_Stability to n on the job's company, so that `jt get` on every job at that company reports `employmentStability` n, and SHALL exit with status 0.
3. WHEN a user runs `jt update <id> --clear-company-stability` or `--clear-employment-stability`, THE CLI SHALL set that Company_Score and its Rationale, Citation_List, Scored_At, and Scored_By to null on the job's company and exit with status 0, including when the score was already null.
4. WHEN a user runs `jt update <id>` without any Company_Score, clear, or rationale flag, THE CLI SHALL keep both Company_Scores and all Score_Metadata of the job's company unchanged.
5. WHEN a user runs `jt update <id> --company-stability-rationale <text>` or `--employment-stability-rationale <text>` and that Company_Score is non-null after the command, THE CLI SHALL store the text trimmed of leading and trailing whitespace as that score's Rationale on the job's company, SHALL store null when the trimmed text is empty, and SHALL keep that score's Scored_At and Scored_By unchanged unless the same command also changes the score value.
6. WHEN a user runs `jt update <id> --clear-company-stability-rationale` or `--clear-employment-stability-rationale`, THE CLI SHALL set that score's Rationale to null and keep the score, Citation_List, Scored_At, and Scored_By unchanged, and SHALL exit with status 0 including when the Rationale was already null.
7. IF a `--company-stability`, `--employment-stability`, `--min-company-stability`, or `--min-employment-stability` value is not a non-empty base-10 string of ASCII digits whose value is a Valid_Score (accepted: "1" to "5" and leading-zero forms such as "03"; rejected: 0, 6, -1, +3, 2.5, a value with leading or trailing whitespace, non-numeric text, an empty string, or a missing value), THEN THE CLI SHALL exit with a nonzero status before opening the Database and print an argument error containing the Company_Range_Message for the company flags or the Employment_Range_Message for the employment flags. IF both an invalid company-score flag and an invalid employment-score flag are provided, THE CLI MAY stop at the first invalid flag encountered and report only that flag's range error.
8. IF a user passes a Company_Score flag together with its clear flag, a rationale flag together with its clear flag, or a rationale flag together with the clear flag of the same Company_Score (for example `--company-stability-rationale` with `--clear-company-stability`), THEN THE CLI SHALL exit with a nonzero status before opening the Database and print an argument conflict error naming both flags.
9. IF the `<id>` given to `jt update` with any Company_Score, clear, or rationale flag matches no job or matches two or more jobs, THEN THE CLI SHALL exit with a nonzero status, print a not-found error or an ambiguity error listing the matching jobs, and change no company or job.
10. WHEN a user runs `jt list --min-employment-stability <n>`, THE CLI SHALL return only jobs whose company's Employment_Stability is a Valid_Score greater than or equal to n, excluding jobs of Unscored_Companies.
11. WHEN a user runs `jt list --min-company-stability <n>`, THE CLI SHALL return only jobs whose company's Company_Stability is a Valid_Score greater than or equal to n, excluding jobs of Unscored_Companies.
12. WHEN a user passes both `--min-company-stability` and `--min-employment-stability`, THE CLI SHALL return only jobs that satisfy both minimums and every other given filter.
13. WHEN a user runs `jt get <id> --json`, `jt list --json`, or `jt update <id> ... --json`, THE CLI SHALL include the keys `companyStability` and `employmentStability` on every job object, each set to the company's stored integer or JSON null, with both keys present in both cases.
14. WHEN a user runs `jt get <id> --json`, THE CLI SHALL include both Company_Scores and all eight Score_Metadata values in the `company` object, with each Citation_List as a JSON array of strings or JSON null and never a JSON-encoded string.
15. WHEN a user runs `jt list` without `--json`, THE CLI SHALL show two table columns headed `CO. STABILITY` and `EMP. STABILITY`, in that order and in the position the `STABILITY` column held, each cell holding the company's integer or "—", and SHALL show no column headed `STABILITY` alone.
16. WHEN a user runs `jt get <id>` or `jt update <id>` without `--json`, THE CLI SHALL print a "Company stability" line with the hint "(1-5, 5 = most stable)" and an "Employment stability" line with the hint "(1-5, 5 = most secure)", each showing the integer or "—", and SHALL print no line labeled "Stability" alone.
17. THE CLI SHALL list in `jt update --help` the flags `--company-stability`, `--clear-company-stability`, `--employment-stability`, `--clear-employment-stability`, `--company-stability-rationale`, `--clear-company-stability-rationale`, `--employment-stability-rationale`, and `--clear-employment-stability-rationale`, and in `jt list --help` the flags `--min-company-stability` and `--min-employment-stability`, with each score and minimum flag's help text stating the range 1–5, that 5 is most stable (company) or most secure (employment), and that the value applies to every job at the job's company, and each minimum flag's help text stating that unscored companies are excluded.
18. WHEN the CLI applies any Company_Score, clear, or rationale flag, THE CLI SHALL keep the favorite flag, Appeal, and Application_Ease of every job unchanged.
19. WHEN a CLI Score_Write succeeds, THE CLI SHALL refresh the CSV_Mirror before exiting, so that the `company_stability` and `employment_stability` cells of every job at that company hold the stored values.
20. THE CLI SHALL offer no flag that edits a Citation_List.
21. WHEN one `jt update` command carries flags for both Company_Scores, or a score flag together with a rationale flag, THE CLI SHALL apply them as one Score_Write through the company Score_Write path of Requirement 5.8, and SHALL apply that Score_Write and any job field changes from the same command in one transaction, so that either every requested change is stored or none is.
22. IF the Score_Service rejects a Score_Write from `jt update` (a Rationale longer than 2000 characters after trimming, or a rationale flag for a Company_Score that is null after the command), THEN THE CLI SHALL exit with a nonzero status, print the Score_Service error, and keep the company, every Stability_Cache, and every field of the job unchanged, including fields set by other flags in the same command such as `--status` or `--notes`.

### Requirement 7: Jobs_List and Jobs_Board

**User Story:** As a job seeker, I want both company scores visible, sortable, and filterable on the Jobs page, so that I can focus on stable employers.

#### Acceptance Criteria

1. THE Jobs_List SHALL show on each row, after the Appeal and Ease selects and in that order, a compact Company_Stability select labeled "Company" (replacing the current "Stability" label) and a compact Employment_Stability select labeled "Employment", each preselected to the job's company's stored value and offering exactly six options: "Unscored" (null) and the values 1, 2, 3, 4, and 5.
2. THE Jobs_List SHALL label the end options of the Company select "1 — least stable" and "5 — most stable" and of the Employment select "1 — least secure" and "5 — most secure" wherever the select shows full option labels, and SHALL show "1" to "5" in the compact row selects.
3. THE Jobs_List SHALL give the Company select and its label the title "Company stability (1–5, 5=most stable)" and the Employment select and its label the title "Employment stability (1–5, 5=most secure)", and SHALL show helper text on the Jobs page stating that both values apply to all jobs at that company.
4. WHEN a user changes a row's Company or Employment select, THE Jobs_List SHALL immediately show the selected value on every loaded row whose company is the edited job's company, SHALL send one `update_job` request for the edited job carrying only the changed field, and on success SHALL set that score on every loaded row of that company to the value returned in the response.
5. IF the `update_job` request from a row's Company or Employment select fails, THEN THE Jobs_List SHALL restore every row changed in criterion 4 to the value it held before the change, SHALL keep the other Company_Score of those rows unchanged, and SHALL show the error message, or "Failed to update company stability" / "Failed to update employment stability" when the error has no message.
6. WHEN a row-level Score_Write succeeds, THE Jobs_List SHALL refetch the Companies_Sidebar rows without reloading the jobs list, and SHALL keep every loaded row in its current position, including rows that no longer satisfy an active minimum filter or sort order, until the next list reload.
7. WHEN a user clicks, focuses, or changes a row's Company or Employment select, THE Jobs_List SHALL keep the job's favorite flag unchanged and SHALL not navigate to the job detail page.
8. THE Jobs_List sort control SHALL offer on every tab, including the Applied tab, the options "Company stability" (`companyStability`) and "Employment stability" (`employmentStability`), each defaulting to descending order when first selected, and SHALL keep the existing sort options.
9. WHEN the `companyStability` or `employmentStability` sort is active, THE Job_Service SHALL order jobs whose company has a non-null value for that score by that value in the chosen direction, SHALL place jobs of Unscored_Companies for that score after all scored jobs in both directions, SHALL break ties by job `id` ascending, and SHALL page with keyset cursors so that walking every page returns each matching job exactly once.
10. THE Jobs_List filter panel SHALL offer a "Minimum company stability" control and a "Minimum employment stability" control, each with exactly the choices "Any", "1+", "2+", "3+", "4+", and "5", where "n+" and "5" mean a threshold of n.
11. WHEN a user applies the filter panel with "Minimum employment stability" set to n, THE Jobs_List SHALL write `minEmploymentStability=<n>` to the URL; WHEN the applied value is "Any", THE Jobs_List SHALL remove `minEmploymentStability` from the URL; in both cases THE Jobs_List SHALL keep writing `minStability` for "Minimum company stability" as it does today.
12. WHILE a minimum employment stability n is applied, THE Job_Service SHALL return only jobs whose company's Employment_Stability is a Valid_Score greater than or equal to n, excluding jobs of Unscored_Companies for that score.
13. WHILE both minimums are applied, THE Job_Service SHALL return only jobs that satisfy both minimums together with every other active filter, search, sort, and tab, and SHALL report a total count equal to the number of those jobs.
14. WHEN the Jobs_List loads a URL whose `minEmploymentStability` value is not exactly one of the strings "1" to "5" (for example "0", "6", "3.0", " 3", or empty), THE Jobs_List SHALL treat the filter as "Any", SHALL send no minimum employment stability to the IPC_Layer, and SHALL show no error or dropped-filter notice.
15. IF the IPC_Layer receives a list request whose minimum employment stability is not null and not a Valid_Score, THEN THE IPC_Layer SHALL reject the request with the Employment_Range_Message and return no jobs.
16. WHILE a minimum employment stability is applied, THE Jobs_List SHALL add exactly 1 to the active filter count, in addition to the 1 already added for an applied minimum company stability.
17. WHEN the user clears all filters, THE Jobs_List SHALL reset both "Minimum company stability" and "Minimum employment stability" to "Any" and remove `minStability` and `minEmploymentStability` from the URL.
18. WHILE a job's company has a non-null Company_Stability n, THE Jobs_Board stage card SHALL show the chip text "Company n" after the Ease chip, in place of "Stability n", with the title "Company stability (1–5, 5=most stable)".
19. WHILE a job's company has a non-null Employment_Stability n, THE Jobs_Board stage card SHALL show the chip text "Employment n" immediately after the Company chip position, with the title "Employment stability (1–5, 5=most secure)".
20. WHILE a job's company is an Unscored_Company for a Company_Score, THE Jobs_Board SHALL omit that score's chip on the stage card and that score's segment, including its " · " separator, on the summary line.
21. WHILE the Jobs_Board shows a job in the "Closed or archived favorites" section, THE Jobs_Board SHALL show " · Company n" in place of " · Stability n" and then " · Employment n" after the Ease segment of that job's summary line, with the same titles as criteria 18 and 19.
22. IF the Companies_Sidebar refetch in criterion 6 fails, THEN THE Jobs_List SHALL keep the existing Companies_Sidebar rows, SHALL keep the saved score on the job rows, and SHALL show no error for the refetch.
23. THE Jobs_List and Jobs_Board SHALL keep the existing Appeal and Ease controls, labels, chips, sort options, and behavior unchanged.
24. FOR ALL sets of jobs and thresholds m, n in 1–5, the result with both minimums applied SHALL equal the unfiltered result restricted to jobs whose company has Company_Stability ≥ m and Employment_Stability ≥ n, in the same relative order (metamorphic property).

### Requirement 8: Job_Detail_Form and Company_Scores_Editor

**User Story:** As a job seeker, I want to edit company scores with their reasoning on the company page and see them on each job, so that the evidence stays next to the score.

#### Acceptance Criteria

1. WHEN the Job_Detail_Form loads a job, THE Job_Detail_Form SHALL show a Company_Stability select labeled "Company stability (1–5, 5=most stable)" with the options "Unscored", "1 — least stable", "2", "3", "4", and "5 — most stable", and an Employment_Stability select labeled "Employment stability (1–5, 5=most secure)" with the options "Unscored", "1 — least secure", "2", "3", "4", and "5 — most secure", each preselected to the stored value of the job's company, or "Unscored" when that value is null.
2. THE Job_Detail_Form SHALL show helper text stating that both scores apply to every job at the company, with a link that opens the Company_Detail_Page of the job's stored company inside the Desktop_UI.
3. WHEN a user changes a Company_Score select in the Job_Detail_Form to a value different from the last saved value, THE Job_Detail_Form SHALL show "Unsaved changes" and enable Save; WHEN the user changes it back to the last saved value and no other field differs, THE Job_Detail_Form SHALL clear the unsaved state.
4. WHEN a user saves the Job_Detail_Form, THE Desktop_UI SHALL include `companyStability` or `employmentStability` in the `update_job` request only when that select differs from the last saved value, and on success SHALL set both selects and the last saved values to the values returned in the response.
5. IF a Job_Detail_Form save fails, THEN THE Job_Detail_Form SHALL keep the selected Company_Score values, keep the form marked as having unsaved changes, keep Save enabled for a retry, and show the error message or "Save failed" when the error has no message.
6. WHILE a Company_Score of the job's company is non-null, THE Job_Detail_Form SHALL show that score's stored Rationale as read-only text when it is non-null and each stored Citation_List URL as a link that opens in the system browser, and SHALL show no Rationale text or citation links for that score when the stored value is null.
7. WHEN the Company_Detail_Page loads a company, THE Company_Detail_Page SHALL show a Company_Scores_Editor with, for each Company_Score, a select with the same options as criterion 1, a Rationale text field, a Citation_List editor that adds and removes URLs, and read-only Scored_At (as a local date and time) and Scored_By (`manual` or `import`) values, each initialized to the company's stored values and showing "—" for a null Scored_At or Scored_By.
8. WHILE a Company_Score select is "Unscored" in the Company_Scores_Editor, THE Company_Scores_Editor SHALL disable that score's Rationale field and Citation_List editor and SHALL send no Rationale or Citation_List for that score on save.
9. WHILE a Rationale field holds more than 2000 characters after trimming leading and trailing whitespace, THE Company_Scores_Editor SHALL show the trimmed character count and an error stating the 2000-character limit, and SHALL disable Save.
10. WHILE the Citation_List editor holds an entry that, after trimming leading and trailing whitespace, is empty or is not an absolute `http` or `https` URL, or holds more than 20 entries, THE Company_Scores_Editor SHALL mark each invalid entry or show an error stating the 20-entry limit, and SHALL disable Save.
11. WHEN a user saves the Company_Scores_Editor, THE Desktop_UI SHALL send one manual Score_Write containing only the fields that differ from the stored values, and on success SHALL replace every editor field, Scored_At, and Scored_By with the values returned by the IPC_Layer.
12. IF a Company_Scores_Editor save fails, THEN THE Company_Scores_Editor SHALL keep every user edit, keep Save enabled for a retry, keep the stored Scored_At and Scored_By displayed, and show the error message or "Failed to save company scores" when the error has no message.
13. WHEN a user changes a Company_Score select in the Job_Detail_Form or Company_Scores_Editor, THE Desktop_UI SHALL keep the job's favorite flag unchanged.
14. WHILE every Company_Scores_Editor field equals the company's stored value, THE Company_Scores_Editor SHALL disable Save and show no unsaved-changes indicator.
15. WHEN a user changes a Company_Score select in the Company_Scores_Editor from a stored non-null value to "Unscored", THE Company_Scores_Editor SHALL show a notice that saving clears that score's Rationale and Citation_List, and on a successful save SHALL show that score's Rationale, Citation_List, Scored_At, and Scored_By as empty or "—".
16. IF the user tries to leave or reload the Company_Detail_Page while the Company_Scores_Editor has unsaved changes, THEN THE Desktop_UI SHALL show the same leave-page warning the Job_Detail_Form shows for unsaved changes.

### Requirement 9: CSV_Mirror columns for both scores

**User Story:** As a job seeker who edits jobs in a spreadsheet, I want both company scores in the Jobs CSV, so that I can bulk-score employers and keep the CSV and the app in sync.

#### Acceptance Criteria

1. WHEN the CSV_Mirror exports jobs, THE CSV_Mirror SHALL write a header with `company_stability` immediately followed by `employment_stability`, both after `application_ease`, SHALL keep every other column in its current order, and SHALL write exactly as many cells in each data row as there are columns in the header.
2. WHEN the CSV_Mirror exports a job, THE CSV_Mirror SHALL write the job's company's stored Company_Stability and Employment_Stability, each as one digit from `1` to `5` or as an empty cell when the company is an Unscored_Company for that score, so that every row of one company holds the same two cell values.
3. THE CSV_Mirror SHALL write no Rationale, Citation_List, Scored_At, or Scored_By column.
4. WHEN the CSV_Mirror imports a file whose header (matched after trimming whitespace, case-insensitively) lacks `company_stability` or `employment_stability`, THE CSV_Mirror SHALL keep the matching Company_Score and its Score_Metadata of every company unchanged.
5. IF an imported row's `company_stability` or `employment_stability` cell, after trimming leading and trailing whitespace, is not empty and does not parse as a base-10 integer from 1 to 5 (an optional leading `+` and leading zeros are accepted, as the existing `company_stability` cell parser does; a `-` sign, a decimal point, or other characters are rejected), THEN THE CSV_Mirror SHALL skip the whole row, including its non-score fields, SHALL increase the skipped count by 1, SHALL report an error containing the row number, "Invalid company stability" or "Invalid employment stability", and the trimmed cell, and SHALL continue with the remaining rows.
6. WHILE the CSV_Mirror imports in merge mode, THE CSV_Mirror SHALL treat a score cell as changed only when the parsed cell value differs from that job's sync-sidecar baseline value for the same score, or, for a row with no sidecar baseline (a new job or a job missing from the sidecar), only when the cell is non-empty; a blank cell in a row with no baseline SHALL leave the company's score unchanged.
7. WHEN the changed score cells for one Company_Score across all non-skipped rows of one company hold one common value (a Valid_Score, or blank meaning clear), THE CSV_Mirror SHALL apply that value to the company as one manual Score_Write with Cache_Fan_Out under the rules of Requirement 3, so that an equal value is an Unchanged_Score_Write that keeps Scored_At and Scored_By unchanged.
8. IF, in merge mode, the company's stored value for that Company_Score differs from the baseline value of any changed row of that company, THEN THE CSV_Mirror SHALL keep the stored value and its Score_Metadata unchanged and SHALL report a conflict naming the job key and the score field (`companyStability` or `employmentStability`), together with the other fields' outcomes of that row.
9. IF the changed cells of one Company_Score across the non-skipped rows of one company hold two or more different values, including a blank and a Valid_Score, THEN THE CSV_Mirror SHALL write neither Company_Score for that company, SHALL report one error naming the company and the row numbers holding the disagreeing cells, and SHALL still import the non-score fields of those rows and all rows of other companies.
10. WHILE the CSV_Mirror imports in overwrite-editable mode, THE CSV_Mirror SHALL treat every score cell in a present column as changed (a blank cell meaning clear), SHALL apply criteria 7 and 9, and SHALL report no score conflict under criterion 8.
11. WHEN the CSV_Mirror creates a job from a row, THE CSV_Mirror SHALL resolve or create the row's company first, SHALL set the new job's Stability_Cache from that company per Requirement 2.4, and SHALL apply the row's non-empty score cells at the company level through criteria 6, 7, and 9.
12. WHEN an imported row changes a job's company and also carries changed score cells, THE CSV_Mirror SHALL count those cells toward the job's new company in criteria 7 to 9 and SHALL leave the previous company's Company_Scores unchanged.
13. WHEN an import that is not a dry run writes a Company_Score, THE CSV_Mirror SHALL export afterward so that both score cells of every row of that company equal the company's stored values, and SHALL record those values as the new sidecar baseline of each row.
14. WHILE the CSV_Mirror imports as a dry run, THE CSV_Mirror SHALL report the same score writes, errors, and conflicts a real import would produce and SHALL write no Company_Score, Score_Metadata, Stability_Cache, CSV file, or sidecar.
15. WHEN the CSV_Mirror imports a file or reads a sidecar written before this feature (with no `employment_stability` column or field), THE CSV_Mirror SHALL complete the import without errors caused by the missing column or field and SHALL keep every company's Employment_Stability and its Score_Metadata unchanged.
16. FOR ALL Databases with any Company_Scores, exporting the jobs and importing the unedited file in merge mode SHALL leave every Company_Score, Score_Metadata value, Stability_Cache, Appeal, Application_Ease, and favorite flag unchanged and SHALL report no errors or conflicts (round-trip property).
17. FOR ALL score cells that parse to a Valid_Score or null, parsing the cell, exporting the value, and parsing the exported cell SHALL produce the same value, and the exported cell SHALL be a single digit or empty (round-trip property).

### Requirement 10: Import scores from a research file

**User Story:** As a job seeker with an external research export, I want to load its scores in one command, so that I can score many employers without typing each one.

#### Acceptance Criteria

1. WHEN a user runs `jt scores import <path>` without `--yes`, THE Score_Importer SHALL read and validate the Score_File, print the plan (companies to apply, unchanged, unmatched, conflicting, invalid, and warnings), make no change to any company, job, or the CSV_Mirror, and exit with status 0 when the file passes the file-level checks in criterion 4.
2. WHEN a user runs `jt scores import <path> --yes`, THE Score_Importer SHALL apply each valid, matched, non-conflicting Score_Entry that is not unchanged as an import Score_Write, and SHALL print a summary with the count and company names of applied and unchanged companies, plus the lists of unmatched names, conflicts, invalid entries, warnings, and failed writes.
3. WHEN a user passes `--json`, THE Score_Importer SHALL print the plan or summary as exactly one JSON object on stdout with the keys `dryRun` (boolean), `applied`, `unchanged`, `unmatched`, `conflicts`, `invalid`, `warnings`, and `failed`. Each key except `dryRun` SHALL hold an array, present and empty when nothing falls in that group, and each element SHALL identify its Score_Entry by zero-based position in `companies[]` and by the entry's `name`, plus the matched company id and name where one exists and a reason for `conflicts`, `invalid`, `warnings`, and `failed`.
4. IF the Score_File cannot be read, is not valid JSON, has a top level that is not a JSON object, lacks a `companies` key, has a `companies` value that is not an array, or has more than 5000 elements in `companies`, THEN THE Score_Importer SHALL exit with a nonzero status, print an error naming the problem, and change no company, job, or CSV_Mirror.
5. THE Score_Importer SHALL map Score_Entry keys as follows: `score` to Company_Stability, `rationale` to its Rationale, `citations` to its Citation_List, `employment_stability` to Employment_Stability, `employment_rationale` to its Rationale, and `employment_citations` to its Citation_List.
6. WHEN a mapped key is present with a non-null value, THE Score_Importer SHALL replace the stored field with that value, normalized under the Rationale and Citation_List rules of Requirement 3 (a trimmed Rationale, an empty or whitespace-only Rationale stored as null, and an empty Citation_List stored as null).
7. WHEN a Score_Entry's `score` or `employment_stability` is JSON null, THE Score_Importer SHALL clear that Company_Score and its Score_Metadata. WHEN a mapped key is omitted, THE Score_Importer SHALL keep that stored field unchanged.
8. WHEN a Score_Entry's `rationale`, `citations`, `employment_rationale`, or `employment_citations` is JSON null for a score that stays non-null, THE Score_Importer SHALL clear that Rationale or Citation_List and keep the score, Scored_At, and Scored_By unchanged.
9. WHEN a Score_Entry supplies a non-null Rationale or Citation_List for a Company_Score that is null after the entry is applied, THE Score_Importer SHALL discard that Rationale and Citation_List, apply the rest of the entry, and list one warning for that company and score in the output.
10. WHEN an import Score_Write sets a Company_Score to a value that differs from the stored value, THE Score_Importer SHALL set Scored_By to `import` and Scored_At to the Score_File's `as_of` normalized to ISO-8601 when `as_of` is a string that parses as a calendar date (`YYYY-MM-DD`) or an RFC 3339 timestamp. Otherwise, including when `as_of` is missing, null, or not a string, THE Score_Importer SHALL set Scored_At to the import time and list one file-level warning that `as_of` was not used.
11. WHEN the Score_Importer matches a Score_Entry, THE Score_Importer SHALL Name_Match each string in the entry's `tracker_names` array, SHALL treat the entry as matched when every matching name resolves to the same single company, and SHALL Name_Match `name` only when `tracker_names` is absent, an empty array, or matches no company. A name that is empty after trimming SHALL match no company.
12. IF the names used to match one Score_Entry resolve to two or more different companies, THEN THE Score_Importer SHALL report a conflict listing the entry and the matched company names, and SHALL write nothing for that entry. IF matching-name ambiguity and duplicate-entry matching affect the same company or entries, THE Score_Importer SHALL emit one merged conflict entry for that company aggregating every relevant entry and reason, and SHALL write nothing for those entries.
13. IF two or more Score_Entries match the same company, THEN THE Score_Importer SHALL report one conflict for that company listing every matching entry, and SHALL write nothing for any of those entries.
14. IF a Score_Entry matches no company, THEN THE Score_Importer SHALL list the entry's `name` and `tracker_names` as unmatched and SHALL create no company or job.
15. IF a Score_Entry is not a JSON object; has neither a non-empty string `name` nor a `tracker_names` array of strings; has a `tracker_names` value that is not an array of strings; holds a `score` or `employment_stability` that is not null and not a JSON integer from 1 to 5 (so `0`, `6`, `2.5`, `3.0`, `"3"`, and `true` are invalid); holds a Rationale that is not a string or exceeds 2000 characters after trimming; or holds a Citation_List that is not an array of at most 20 strings, each an absolute `http` or `https` URL, THEN THE Score_Importer SHALL skip the whole entry, list it as invalid with the first failing key and reason, and keep processing the other entries.
16. THE Score_Importer SHALL complete the checks in criteria 4 and 11 to 15 for every Score_Entry before the first write.
17. THE Score_Importer SHALL ignore the `jobs`, `job_ids`, `signals`, `employment_researched`, and `rubric` keys and every other unmapped key, and an unmapped key SHALL never make an entry invalid.
18. WHEN the Score_Importer applies a Score_Entry, THE Score_Importer SHALL write that company and its Cache_Fan_Out in one transaction. IF that transaction fails, THEN THE Score_Importer SHALL roll back that company and its jobs to their values before the entry, list the entry under failed with the error, and keep applying the remaining entries.
19. THE Score_Importer SHALL keep every job's Appeal, Application_Ease, and favorite flag unchanged.
20. WHEN `jt scores import --yes` writes one or more companies, THE Score_Importer SHALL refresh the CSV_Mirror before exiting. IF that refresh fails, THEN THE Score_Importer SHALL keep the committed Database writes, print an error indicating the mirror refresh failed, and exit with a nonzero status.
21. FOR ALL Score_Files, a Dry_Run followed by a `--yes` run on the same unchanged Database SHALL report the same applied, unchanged, unmatched, conflict, invalid, and warning sets, provided no write fails (model-based property).
22. FOR ALL Score_Files, running `jt scores import --yes` twice in a row SHALL report every company applied in the first run as unchanged in the second run, SHALL report no applied companies in the second run, and SHALL leave every stored value, including Scored_At and Scored_By, equal after both runs (idempotence property).
23. WHEN a user passes `-q` or `--quiet`, THE Score_Importer SHALL print only the plan or summary payload, without progress or status lines.
24. WHEN a matched, valid Score_Entry would make no Company_Score, Rationale, or Citation_List of its company differ from the stored value after the normalization in criterion 6 (an Unchanged_Score_Write for both scores, including clearing a score that is already null), THE Score_Importer SHALL list the company as unchanged, SHALL write no company column for it, and SHALL keep its Scored_At and Scored_By unchanged even when `as_of` differs from the stored Scored_At.
25. WHEN a `jt scores import --yes` run finishes without a failed write and without a CSV_Mirror refresh failure, THE Score_Importer SHALL exit with status 0, including when entries are unmatched, conflicting, invalid, or carry warnings. IF one or more entries are listed under failed, THEN THE Score_Importer SHALL print the full summary and exit with a nonzero status.

### Requirement 11: Companies_Sidebar sorts by stored scores

**User Story:** As a job seeker, I want to sort companies by their stored company and employment scores, so that I see the employers I rated highest first.

#### Acceptance Criteria

1. THE Companies_Sidebar SHALL read `companyStability` and `employmentStability` from each `CompanyRow` in place of `stabilityAverage` and `stabilityScoredJobCount`, and SHALL use no job-level Stability_Cache value or per-job average for sorting or annotation.
2. WHILE the "Most stable company" sort (`mostStable`) is active, THE Companies_Sidebar SHALL order companies with a non-null Company_Stability from 5 down to 1, SHALL order companies with equal Company_Stability by Name_Order, and SHALL place every Unscored_Company after all scored companies, ordered among themselves by Name_Order.
3. THE Companies_Sidebar sort menu SHALL list a "Most stable employment" option (`mostStableEmployment`) immediately after "Most stable company" as the last entry of the same menu group, with no separator between the two, and with the same selection, keyboard, focus, and ARIA behavior as the other sort options.
4. WHILE the "Most stable employment" sort is active, THE Companies_Sidebar SHALL order companies with a non-null Employment_Stability from 5 down to 1, SHALL order companies with equal Employment_Stability by Name_Order, and SHALL place every Unscored_Company for Employment_Stability after all scored companies, ordered among themselves by Name_Order.
5. WHILE the `mostStable` sort is active and a company has a non-null Company_Stability n, THE Companies_Sidebar SHALL show the annotation text "Company n" (n as a whole number with no decimal point) with the accessible label "Company stability n", and SHALL no longer show the "Average stability" annotation.
6. WHILE the `mostStableEmployment` sort is active and a company has a non-null Employment_Stability n, THE Companies_Sidebar SHALL show the annotation text "Employment n" (n as a whole number with no decimal point) with the accessible label "Employment stability n".
7. WHILE a stability sort is active and the company is an Unscored_Company for the active sidebar sort's score, THE Companies_Sidebar SHALL show no annotation at all for that company, regardless of the other Company_Score's value.
8. IF a row's `companyStability` or `employmentStability` is missing, null, not a JSON number, a numeric string (for example `"4"`), non-finite, a non-integer number (for example 4.5), or outside 1–5, THEN THE Companies_Sidebar SHALL treat that value as null for sorting and annotation, SHALL still list the company, and SHALL show no error and throw no exception.
9. WHEN the Companies_Sidebar loads a view state saved before this feature, including one whose sort is `mostStable`, THE Companies_Sidebar SHALL restore the saved sort and filters unchanged, with `mostStable` now ordering by stored Company_Stability per criterion 2.
10. WHEN a user selects "Most stable employment", THE Companies_Sidebar SHALL persist the sort as `mostStableEmployment` in the saved view state with the active filters unchanged, and SHALL restore that sort and those filters the next time the Companies_Sidebar loads.
11. WHILE a stability sort is active and one or more company filters are active, THE Companies_Sidebar SHALL list only the companies that pass every active filter, in the order defined by criterion 2 or 4.
12. WHEN the IPC_Layer returns `list_companies` rows, THE IPC_Layer SHALL set each row's `companyStability` and `employmentStability` to the company's stored values whatever the status of its jobs, so that a company whose jobs are all archived, closed, new-from-watch, or dismissed, or that has no jobs, still shows and sorts by its stored scores.
13. FOR ALL lists of company rows with valid and malformed score values, and any subset of active company filters, the order under each stability sort SHALL equal a reference sort over the filtered rows on (unscored last, score descending, Name_Order), with malformed values sanitized to null per criterion 8 (model-based property).

### Requirement 12: Display rules for the four scores

**User Story:** As a job seeker, I want the four scores named consistently and never merged, so that I do not confuse a posting judgment with an employer judgment.

#### Acceptance Criteria

1. THE Desktop_UI SHALL use the compact labels "Company", "Employment", "Appeal", and "Ease" for the four scores in Jobs_List row select labels, Jobs_Board chip text, and Companies_Sidebar annotations, and SHALL use no other compact label (including a bare "Stability") for Company_Stability or Employment_Stability.
2. THE Job_Tracker SHALL use the full labels "Company stability", "Employment stability", "Appeal", and "Application ease" in Job_Detail_Form field labels, Company_Scores_Editor section labels, select and chip titles, Jobs_List sort options and filter labels, CLI plain-text detail lines, and CLI help text, except for the `jt list` table headers `CO. STABILITY` and `EMP. STABILITY` defined in Requirement 6.15.
3. WHEN the Desktop_UI or the CLI shows more than one of the four scores for one job, THE Job_Tracker SHALL show them in the order Appeal, Application_Ease, Company_Stability, Employment_Stability.
4. THE Job_Tracker SHALL show both Company_Scores in each of these places: every Jobs_List row, every Jobs_Board stage card, every Jobs_Board "Closed or archived favorites" row, the Job_Detail_Form, the Company_Scores_Editor, `jt get` and `jt update` plain-text output, the `jt list` table, and the job objects of `jt get`, `jt list`, and `jt update` JSON output.
5. THE Job_Tracker SHALL show, sort by, filter by, export, or return no value computed from two or more of Company_Stability, Employment_Stability, Appeal, and Application_Ease (such as a sum, average, minimum, maximum, or weighted score), in any Desktop_UI view, CLI output, CSV_Mirror column, or JSON field.
6. WHEN a Score_Write changes a Company_Score, THE Desktop_UI SHALL keep the Appeal value, label, title, styling, and Appeal sort position of every job display unchanged.
7. WHILE a Company_Score is null, THE Job_Tracker SHALL omit that score's Jobs_Board chip and its " · " separator, select the "Unscored" option in every select for that score, print "—" for that score in CLI plain-text output, and return JSON null for that score in CLI and IPC_Layer output.

### Requirement 13: Documentation

**User Story:** As an external agent or a new user, I want the company-scoped scores and the importer documented, so that I use them correctly.

#### Acceptance Criteria

1. THE Job_Tracker SHALL replace the company stability paragraph of `AGENTS.md` with text stating all of the following: Company_Stability is stored on the company, not on the job; `jt update <id> --company-stability <1..5>` and `--clear-company-stability` write the job's company and update every job at that company, including archived and closed jobs; only the integers 1 to 5 are accepted (1 = least stable, 5 = most stable), and any other value is rejected; `companyStability` on a job object holds the company's value, is shared by every job at that company, and is null when the company is unscored; the CLI prints a one-time stderr report of Backfill_Conflict companies, and that report never appears on stdout. After the change, no sentence in `AGENTS.md` SHALL describe Company_Stability as a per-job score.
2. THE Job_Tracker SHALL document in `AGENTS.md`: the `jt update <id> --employment-stability <1..5>` flag; the `--clear-employment-stability` flag; the `jt list --min-employment-stability <1..5>` flag; the scale, 1 = least secure and 5 = most secure, with only the integers 1 to 5 accepted; that the value applies to every job at the job's company; that the minimum filter excludes jobs at Unscored_Companies and, when combined with `--min-company-stability`, returns only jobs that meet both minimums; and the `employmentStability` JSON field on job objects, null when unscored.
3. THE Job_Tracker SHALL document in `AGENTS.md`: the `--company-stability-rationale <text>` and `--employment-stability-rationale <text>` flags and their `--clear-company-stability-rationale` and `--clear-employment-stability-rationale` counterparts; the 2000-character Rationale limit after trimming; that a rationale write is rejected while its score is null; that clearing a score also clears its Rationale, Citation_List, Scored_At, and Scored_By; and that the CLI has no flag that edits citations.
4. THE Job_Tracker SHALL document `jt scores import <path>` in `AGENTS.md`, stating: without `--yes` the command is a Dry_Run that prints the plan, writes nothing, and exits 0, and `--yes` applies the plan; `--json` prints one object with the keys `dryRun`, `applied`, `unchanged`, `unmatched`, `conflicts`, `invalid`, `warnings`, and `failed`, and `-q` / `--quiet` prints only the payload; only JSON input is accepted; the six key mappings (`score`, `rationale`, `citations`, `employment_stability`, `employment_rationale`, `employment_citations`), that an omitted key keeps the stored value and a JSON null clears it; names are matched case-insensitively after trimming, through `tracker_names` first and then `name`; an entry that matches more than one company, and a company matched by more than one entry, are reported as conflicts and not written; invalid entries are skipped and the remaining entries still apply; unmatched companies are reported and no company or job is created; applied scores record Scored_By `import` and take Scored_At from `as_of` when it parses; the exit status rules of Requirement 10.25; and Appeal, Application_Ease, and favorite flags are left untouched.
5. THE Job_Tracker SHALL change the "Commands agents actually use" table in `AGENTS.md`: change the existing company stability row so its goal says the score applies to the job's company and every job at it; add one row for setting Employment_Stability with the example `jt update <id> --employment-stability 4 --json` and the scale "1–5, 5 = most secure; null if unset"; add one row for applying a Score_File with the example `jt scores import <path> --yes --json`; and keep every other row unchanged.
6. THE Job_Tracker SHALL document in `AGENTS.md` that setting or clearing either Company_Score, or either Rationale, does not favorite the job and does not change its Appeal or Application_Ease.
7. THE Job_Tracker SHALL make these changes to the "Jobs CSV" section of `README.md`: add `employment_stability` immediately after `company_stability` in the editable columns list; replace the per-job `company_stability` bullet with a company-level description (1 to 5, 5 = most stable); add an `employment_stability` bullet (1 to 5, 5 = most secure); and state that each score cell repeats the company's value on every row of that company, that a blank cell means unscored, and that the CSV has no rationale, citation, Scored_At, or Scored_By columns.
8. THE Job_Tracker SHALL document these CSV import rules for the two score columns in the "Jobs CSV" section of `README.md`: a cell counts as changed only when it differs from the last export; a blank changed cell clears the company score; a file without the column keeps the stored scores; a cell that is not blank and not an integer from 1 to 5 skips its row; changed cells that disagree for one company are rejected for that company, while other companies and the rows' non-score fields still import; and when the company's stored score changed in the app since the last export, the stored value wins and a conflict is reported.
9. THE Job_Tracker SHALL document in `AGENTS.md` that the `company` object in `jt get <id> --json` carries both Company_Scores and, for each, its Rationale, Citation_List, Scored_At, and Scored_By, that Scored_By is `manual` or `import`, and that each Citation_List is an array of URLs or null.
10. THE Job_Tracker SHALL spell every flag, subcommand, JSON field, and CSV column named in `AGENTS.md` and `README.md` for this feature exactly as it appears in `jt update --help`, `jt list --help`, `jt scores import --help`, CLI JSON output, and the exported CSV header.
