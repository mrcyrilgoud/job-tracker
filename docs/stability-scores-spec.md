# Company stability and employment stability

**Status:** working draft
**Date:** 2026-11-02

Scores today split across two places. Job Tracker stores one integer, `company_stability`, on each job. A separate research export (shape checked against the 2026-11-02 file: top-level `as_of` and `rubric`, per-company `score`, `rationale`, `citations`, `employment_stability`, `employment_rationale`, `employment_citations`, `employment_researched`, `tracker_names`, `signals`) lives outside the app. This spec makes both scores first-class in the app, on the company, with manual edit and a one-shot import. v1 does not research the web.

## Goal

A person triaging the pipeline can see, filter, and sort two different 1–5 scores for the employer:

- **Company stability** — can this employer keep operating (cash, stage, parent, distress)?
- **Employment stability** — how safe is a typical engineering hire there (layoffs, headcount trend, hire-then-cut)?

Each score is one value per company and shows up on every job at that company. Appeal and application ease stay per-job judgments about the posting.

## Non-goals

- Continuous or in-app web research, scraping, or an LLM pass that assigns scores.
- A third “culture / caution” score.
- Per-job overrides. A posting does not get its own company or employment score.
- Creating companies or jobs from the research file.
- Persisting research `signals` (`funding`, `stage`, `headcount_trend`, `notes`) or `employment_researched` as columns. Those stay inputs to the written rationale.
- Changing what `appeal` or `application_ease` mean, or averaging any of these scores into one number.

## Definitions and rubrics

Both scores are integers 1–5 or null (unscored). **5 is best** on both scales. Null is “not scored,” which is different from a low score. Thin evidence defaults to **3**, not to a guess at the extremes.

These rubrics match the 2026-11-02 research scale for company stability. The employment scale is the product scale for the same export’s `employment_stability` field (that file used 1–4 in practice; **5 is still valid**).

### Company stability (research field `score`)

Durability of the employer: funding, revenue, public-company cash flow, parent strength, shutdown risk.

| Score | Meaning |
| --- | --- |
| 5 | Most stable. Public mega-cap, durable cash flow, or a strong parent that can fund the team. |
| 4 | Strong. Late-stage or very well capitalized private (unicorn, Series D+, roughly ≥$100M raised in the last year, or clear revenue). |
| 3 | Typical startup. Series A–C or a large seed with known investors and no red flags. Also the thin-evidence default. |
| 2 | Elevated risk. Small or early seed, stale funding (no raise in several years), or layoffs plus weak fundraising. |
| 1 | Least stable. Distressed: large layoffs without recovery, corroborated shutdown signals, or the company cannot fund operations. |

### Employment stability (research field `employment_stability`)

How secure a typical new engineering hire’s job looks: repeated layoffs, shrinking headcount, cuts soon after hiring, absorption after an acquisition. This can diverge from company stability (a rich company can still run rolling cuts; a modest startup can have a clean hiring record).

| Score | Meaning |
| --- | --- |
| 5 | Steady employment. No meaningful layoff record, stable or growing eng headcount, and no pattern of hiring then cutting. |
| 4 | Secure with small exceptions. Isolated, small, or dated cuts against a large or growing org. Rapidly growing and well funded with no layoff record fits here. |
| 3 | Ordinary risk for the stage. No public layoff record, or old cuts that hiring has since outgrown. Thin-evidence default, including when `employment_researched` is false and the note is a stage default. |
| 2 | Elevated hire risk. Documented layoff rounds, shrinking headcount, post-merger cuts, or a tiny team where a missed round likely means cuts. |
| 1 | Least secure. Repeated rounds, large share of staff cut, or people let go within months of being hired. |

## What exists today (verified)

| Concern | Where it lives | Behavior now |
| --- | --- | --- |
| Company stability | `jobs.company_stability INTEGER`, JSON `companyStability` | Nullable. Check requires an integer storage type and 1–5. No rationale, citations, or timestamp. |
| Appeal | `jobs.appeal`, JSON `appeal` | Per job. 1 = least appealing, 5 = most appealing. |
| Application ease | `jobs.application_ease`, JSON `applicationEase` | Per job. 1 = hardest to apply, 5 = easiest. |
| Companies | `companies` (`id`, `name`, `careers_url`, timestamps) | No score columns. Name match is trim + `COLLATE NOCASE` exact (`find_or_create_company`). |
| CLI write | `jt update --company-stability N` and `--clear-company-stability` | Writes that job only. Omit the flag to leave the value. |
| CLI read | `jt list --min-company-stability N`, `jt get`, `jt list --json` | Minimum filter drops unscored jobs. JSON uses `companyStability`. Plain list/detail label the column “Stability”. |
| Jobs UI | Job detail select, inline select on the jobs list, sort field `companyStability`, filter “Minimum company stability” | Board chips read `Stability N`. Nulls sort last. |
| Companies sidebar | `CompanyRow.stabilityAverage`, `stabilityScoredJobCount`, sort `mostStable` (“Most stable company”) | Average is the mean of pipeline jobs’ `company_stability` (non-archived, not new-from-watch, not dismissed). Annotation text is “Average stability”. |
| CSV mirror | Header `company_stability` on the job CSV | Same three-way rules as `appeal` / `application_ease`: a missing column does not clear; a blank cell clears; an invalid cell rejects; a present integer updates. |

Employment stability is absent from schema, CLI, UI, and CSV.

## Data model

**Canonical store is the company row.** One company stability and one employment stability apply to every job with that `company_id`.

Add nullable columns on `companies`. Use the same integer check as `jobs.company_stability` (SQLite integer storage, null or 1–5), not the looser appeal check.

| Column | Type | Notes |
| --- | --- | --- |
| `company_stability` | INTEGER NULL | Research `score`. |
| `company_stability_rationale` | TEXT NULL | Plain text, max 2000 characters. |
| `company_stability_citations` | TEXT NULL | JSON array of up to 20 `http`/`https` URL strings. Null means “no citation list stored.” |
| `company_stability_scored_at` | TEXT NULL | ISO-8601 timestamp of the last score write. |
| `company_stability_scored_by` | TEXT NULL | `manual` or `import`. |
| `employment_stability` | INTEGER NULL | Research `employment_stability`. |
| `employment_stability_rationale` | TEXT NULL | Same limits as the company rationale. |
| `employment_stability_citations` | TEXT NULL | Same shape as company citations. |
| `employment_stability_scored_at` | TEXT NULL | |
| `employment_stability_scored_by` | TEXT NULL | `manual` or `import`. |

Clearing a score clears its rationale, citations, `scored_at`, and `scored_by` together. A null score must not keep a visible rationale.

Setting a score without sending rationale or citations updates the integer, sets `scored_by` and `scored_at`, and leaves the existing prose and URLs in place. The jobs-list dropdown can stay a score-only control.

JSON names are camelCase, matching `companyStability`: `companyStabilityRationale`, `companyStabilityCitations` (array, not a raw string), `companyStabilityScoredAt`, `companyStabilityScoredBy`, and the `employmentStability*` twins.

### Job column and migration

Keep `jobs.company_stability` as a **write-through cache** of the company value. Existing list SQL, the minimum filter, sort, CSV header, and `companyStability` on job JSON stay valid. Every write to the company score sets `jobs.company_stability` to that same integer (or null) on **every** job with that `company_id`, including archived rows. Inserting a job copies the company’s current cache onto the new row.

Do **not** add `jobs.employment_stability`. Readers join `companies`. A later cleanup may drop the company-stability cache once every reader uses the company row; that cleanup is out of this slice.

Backfill on migrate, per company, from existing job values:

- No non-null job scores: leave the company null.
- Exactly one distinct non-null score: copy it to `companies.company_stability`, set `scored_by` to `manual` and `scored_at` to migration time, leave rationale and citations null. Then rewrite every job row for that company to that score so mixed nulls and the value collapse to one cache.
- Two or more distinct scores: leave the company null and leave job rows untouched. Record the company id and the conflicting values for a one-time report (`jt` stderr on next open is enough). The next explicit company write overwrites the cache on all of that company’s jobs.

There is no employment backfill.

### Why not only job columns

Company stability is a fact about the employer. The current job column already drifts conceptually: two postings at one company can store different integers, and the sidebar then averages them. Employment stability has the same shape in the research file (one value per company, many `job_ids`). A company row is the source of truth. The job column remains only so the shipped company-stability path does not have to be rewritten in the same change.

## Surfaces

### Job Tracker UI

Labels stay distinct. Suggested short labels: **Company**, **Employment**, **Appeal**, **Ease**. Title text carries the scale, the way `Company stability (1–5, 5=most stable)` does today. Board chips that currently say `Stability N` become `Company N`, with a second chip `Employment N` when that score is set. Unscored stays blank on the board (same as today) and “Unscored” in selects.

- **Jobs list:** keep the company-stability column, sort (`companyStability`, nulls last), and “Minimum company stability” filter. Add the employment score beside it, a sort field `employmentStability` (nulls last), and a “Minimum employment stability” filter with the same 1+ … 5 choices. The minimum filter excludes unscored rows. The two minimums AND together. The inline select writes the **company**, then refreshes every visible row for that company. Helper text on the control: the value applies to all jobs at that company.
- **Job detail:** company-stability select stays. Add an employment select with the same unscored / 1–5 pattern (`ScoreSelect`). Both writes update the company and the company-stability cache. Show rationale and citations read-only here (short text plus links), with the edit affordance on the company page so the job form does not become a second editor.
- **Company detail and sidebar:** company detail has no score UI today; add a scores section with both integers, rationale text fields, and citation URL lists, plus scored-at / scored-by as read-only metadata. Sidebar sort `mostStable` sorts by the stored company integer (nulls last), and the annotation shows that integer (for example `Company 4`) instead of a mean. Add a parallel sort for employment stability. `stabilityAverage` and `stabilityScoredJobCount` stop being the source of truth; replace them with `companyStability` and `employmentStability` on `CompanyRow`.

Appeal and application ease controls stay where they are and keep their own copy. A stability select must not favorite the job.

### CLI

Shipped flags keep their names. Their **scope** changes from one job to that job’s company (and the cache fan-out):

- `jt update <id> --company-stability <1-5>`
- `jt update <id> --clear-company-stability`
- `jt list --min-company-stability <1-5>` (unscored excluded)

Add the same shape for employment:

- `jt update <id> --employment-stability <1-5>`
- `jt update <id> --clear-employment-stability`
- `jt list --min-employment-stability <1-5>` (unscored excluded; composes with the company minimum)

Reject values outside 1–5, including floats and padded junk, using the same strictness as `parse_company_stability`. Each flag conflicts with its clear flag. Omitting a flag leaves that score unchanged. `--json` job payloads include `companyStability` and `employmentStability`. Company list/detail JSON includes the rationale, citations, `scoredAt`, and `scoredBy` fields. Plain-text detail prints both scores with their scale hints, and renames the current “Stability” line so it is clearly company stability.

Optional prose flags on `jt update` are in scope if they stay small: `--company-stability-rationale`, `--employment-stability-rationale`, and matching clear flags. Citations are edited in the UI and replaced by import; the CLI does not need citation flags in v1.

`jt get` / `jt list` keep returning the score on the job object so existing scripts that read `companyStability` keep working. Document that the value is shared by every job at the company.

### CSV mirror

The job CSV stays the mirror (no second file in v1).

- Keep header `company_stability`. Cell value is the company score, repeated on each job row.
- Add header `employment_stability` with the same rules already used for `company_stability`: missing column does not clear; blank clears that score for the company; invalid cell fails the row; a legal integer updates the company.
- Before apply, group rows by company. If two rows for one company carry different non-blank values for either score, fail that company, write nothing for it, and report the company name. Do not store a split.
- A successful write updates the company row, fans out `jobs.company_stability`, and rewrites both score cells on every mirrored row for that company on the next sync.
- Rationale and citations are not CSV columns.

### Import

Command: `jt scores import <path>`.

Default is a dry run (no writes, exit 0) that prints the plan. `--yes` applies. `--json` / `-q` follow the existing CLI contract. The payload is a summary: counts applied and unchanged, plus unmatched names and conflicts. It is not a dump of every company.

Input is one JSON object in the research shape: `as_of` (date or timestamp) and `companies[]`. A CSV of the same columns is acceptable if it is easier to share, with one row per company and the same field names; JSON is the format the current research file already has. v1 can ship JSON only and leave CSV import as a follow-up if it slips.

For each company object:

| Research key | Writes |
| --- | --- |
| `score` | `company_stability` |
| `rationale` | `company_stability_rationale` |
| `citations` | `company_stability_citations` |
| `employment_stability` | `employment_stability` |
| `employment_rationale` | `employment_stability_rationale` |
| `employment_citations` | `employment_stability_citations` |

A present key replaces that field. A JSON null score clears that score and its metadata. An omitted key leaves the stored field alone. `scored_by` is `import`. `scored_at` is `as_of` when it parses, otherwise the import time. Then fan out the company-stability cache.

Match to an existing company: case-insensitive exact name, same rule as `find_or_create_company`. Try each `tracker_names` entry first; if none match, try `name`. All matching names must be the same company. Zero matches: report the name, do not insert a company. Two companies: report a conflict, do not write. Ignore `jobs`, `job_ids`, and `signals` for matching and for writes.

Validate before any write of that company: scores integers 1–5 or null, rationale length, citation list shape. A bad company is skipped and reported; siblings in the file still apply.

The importer does not read or write `appeal` or `applicationEase`.

## Display rules

Show company stability and employment stability as two scores wherever a company or its jobs are summarized (jobs list, job detail, company detail, `jt` plain text and JSON).

Appeal answers “do I want this posting?” Application ease answers “how hard is the form?” Neither is a substitute for either stability score, and the UI copy and CLI labels use the four names above. Do not add a combined “stability” figure, and do not color a job by appeal using a stability field.

Empty state is unscored, rendered like the existing selects (blank chip, “Unscored” option, CLI em dash). A 3 that was imported as a thin-evidence default is a real 3; the rationale carries that context.

## Out of scope for v1

- Scheduled or on-demand web research that refreshes scores.
- Culture or “caution list” as another 1–5 score.
- Per-role employment scores (the score is the typical eng hire at the company, not this requisition).
- Fuzzy company-name matching, alias tables beyond `tracker_names` on import, or auto-creating companies.
- Citation editing on the CLI.
- Dropping `jobs.company_stability` entirely.
- Job events for score edits (appeal and ease do not write events today).

## Acceptance criteria

- [ ] Migrate adds the company columns with the integer 1–5-or-null check. Existing databases gain them as null. Re-running migration is a no-op.
- [ ] Backfill copies a company score only when every non-null `jobs.company_stability` for that company is the same value, then rewrites all of that company’s job rows to it. Conflicting companies stay unresolved and are reported. Employment stays null.
- [ ] Setting or clearing company stability from job detail, the jobs-list select, or `jt update` updates the company, all job caches for that company, and the CSV mirror. Other companies stay unchanged. Clearing also clears rationale, citations, and score metadata.
- [ ] The same write path exists for employment stability, without a new job column. `jt list --json` and `jt get --json` include `employmentStability`. `--min-employment-stability` excludes unscored jobs and composes with `--min-company-stability`.
- [ ] Invalid scores (0, 6, non-integers) fail in the UI, CLI, CSV, and import, and leave stored rows unchanged.
- [ ] Jobs list shows, sorts, and minimum-filters both scores. Chips use separate Company and Employment labels. Appeal and application ease behavior is unchanged.
- [ ] Company detail edits both scores, rationales (≤2000 chars), and citation URL lists. Sidebar “Most stable company” uses the stored company integer, and an employment sort exists. The sidebar no longer averages per-job scores.
- [ ] CSV round-trips `company_stability` and `employment_stability`. A missing column does not clear. Disagreeing cells for one company do not write. Rationale is not a CSV column.
- [ ] `jt scores import` dry-run writes nothing. `--yes` updates matched companies only, maps the research keys in the table above, sets `scored_by` to `import`, fans out the cache, and reports unmatched names and conflicts. It does not create companies or touch appeal / application ease.
- [ ] New jobs inherit the company’s company-stability cache. A company with several jobs cannot display two different company-stability integers after a successful write.
