# Implementation Plan: Company Stability Score

## Overview

Company_Stability is a third optional per-job score (1–5, null = unscored) that follows the Appeal and Application_Ease implementation at every layer. Work goes Rust backend first (schema, Job_Service, sort/filter/paging, IPC, CLI, CSV, company aggregates), then desktop libraries, then React components, then `AGENTS.md` and `README.md`. All backend logic stays in Rust; the UI uses Tauri `invoke` only. See design.md for SQL, signatures, and option lists.

Property tests use `proptest` (Rust) and `fast-check` (desktop), run at least 100 cases, and carry `// Feature: company-stability-score, Property {number}: {property_text}` with the design.md property number. Commands: `cargo test` in `src-tauri/`, `npx vitest --run` in `desktop/`.

## Tasks

- [x] 1. Add the Company_Stability column and model types
  - [x] 1.1 Add the `company_stability` column in `src-tauri/src/db/migrate.rs`
    - Add it to `CREATE TABLE jobs` and as a `PRAGMA table_info`-guarded `ALTER TABLE ... ADD COLUMN` after the `application_ease` block, both with the type-strict CHECK from the design
    - The CHECK must include `typeof(company_stability) = 'integer'` (the Appeal CHECK accepts `2.5`); no default, no backfill; propagate the `ALTER` error with `?`
    - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 1.7_

  - [x] 1.2 Add Company_Stability model types in `src-tauri/src/models.rs`
    - `company_stability: Option<i64>` on `Job` and `JobSummary` (no `skip_serializing_if`, so `null` is emitted), the `CompanyStability` sort variant, `total_count: i64` on `JobListPage`, `COMPANY_STABILITY_RANGE_MESSAGE`, and `checked_company_stability`
    - Update every full struct literal of `Job`, `JobSummary`, and `JobListPage` across the crate and its tests so it compiles
    - _Requirements: 2.1, 2.5, 6.1, 7.13_

  - [x]* 1.3 Write migration unit tests in `src-tauri/src/db/migrate.rs`
    - Fresh DB has the column; a legacy DB with a few rows gains it, rows read back NULL and other columns match a snapshot; running `migrate` again changes nothing, including stability values written in between
    - Table test of invalid direct writes (`0`, `6`, `-1`, `2.5`, `'2.5'`, `'high'`) fails and leaves the row unchanged
    - A failing `ALTER` (read-only connection) leaves the column absent
    - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 1.7_

- [x] 2. Set and clear Company_Stability in the Job_Service
  - [x] 2.1 Implement the update path in `src-tauri/src/jobs/service.rs`
    - Append `company_stability` as the last column of `JOB_COLS` and update `map_job` and `JOB_COL_COUNT` together
    - Add the tri-state `UpdateJobInput.company_stability` with the custom deserializer from the design, so non-integer JSON fails with the Stability_Range_Message; range-check in `update_job` before the single `UPDATE`
    - `UpdateJobInput` derives `Default`; add `company_stability: None` only to the full literals that do not use `..Default::default()` (for example in `cli/handlers.rs` and `jobs/csv.rs`)
    - _Requirements: 1.6, 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 2.8, 2.10, 2.11_

  - [x]* 2.2 Write Job_Service unit tests in `src-tauri/src/jobs/service.rs`
    - Table over `{null, 1..5}`: `update_job`, `get_job_detail`, `list_jobs`, and `list_jobs_page` return the value, and the JSON always has the `companyStability` key
    - Independence cases: other-field update, archive, unarchive, and `closed` keep Company_Stability; a stability-only update keeps Appeal, Application_Ease, favorite, and status
    - Table of bad JSON payloads (`0`, `6`, `3.5`, `3.0`, `"3"`, `true`, `[]`, `{}`), alone and with another valid change: error contains the Stability_Range_Message and the job, including `updated_at`, is unchanged
    - Unknown id returns "Job not found"; `create_job_from_url` and a watch save produce an Unscored_Job
    - _Requirements: 1.6, 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 2.8, 2.10, 2.11, 3.7_

- [x] 3. Sort, filter, and page by Company_Stability in the Job_Service
  - [x] 3.1 Add the Minimum_Stability_Filter in `src-tauri/src/jobs/service.rs`
    - Add `min_company_stability` to `JobFilters` and one fragment in `append_job_filters` with `n` as a bound parameter; unscored jobs must be excluded
    - _Requirements: 6.14, 7.2, 7.3, 7.4, 7.10, 8.1, 8.2, 8.5_

  - [x] 3.2 Add the Stability_Sort_Field ordering and cursor in `src-tauri/src/jobs/service.rs`
    - Null-last ordering with `j.id ASC` tie-break, allowed on every tab including applied
    - Factor the Appeal cursor into `append_nullable_score_cursor` (static column name only) and use it for both columns; set `next_cursor.sort_value` for the new field
    - The existing Appeal sort and paging tests (`jobs_page_supports_posting_application_appeal_and_company_sorts` and the large paging test) must keep passing unchanged
    - _Requirements: 6.3, 6.4, 6.5, 6.6, 6.10_

  - [x] 3.3 Add sized paging and `total_count` in `src-tauri/src/jobs/service.rs`
    - Add `list_jobs_page_sized`; `list_jobs_page` calls it with `JOB_PAGE_SIZE`
    - Compute `total_count` with the filter WHERE clause but without the cursor predicate, so it is the same on every page; add `j.company_stability` to the page projection
    - _Requirements: 2.1, 6.11, 7.13_

  - [x]* 3.4 Write property test for stability sort order and lossless keyset paging
    - **Property 10: Stability sort matches the reference order** and **Property 11: Keyset paging is lossless and counts are stable**, in one test
    - 0–500 jobs, both directions, page sizes 1–100 via `list_jobs_page_sized`, Minimum_Stability_Filter Any or 1–5: the concatenated pages equal the reference order `(isUnscored, desc ? -cs : cs, id)` with no repeats or gaps, and `total_count` on every page equals the length of the full unpaged filtered list
    - **Validates: Requirements 6.3, 6.4, 6.5, 6.6, 6.10, 6.11, 6.12, 7.13**

  - [x]* 3.5 Write property test for the minimum filter as a pure restriction
    - **Property 12: Minimum filter is a pure restriction**
    - Check both `list_jobs_page` (all pages) and `list_jobs`
    - **Validates: Requirements 6.14, 7.2, 7.3, 7.4, 7.10, 7.11, 7.12, 8.1, 8.5**

- [x] 4. Validate the minimum stability filter in the IPC_Layer
  - [x] 4.1 Add `min_company_stability` to `ListJobsArgs` in `src-tauri/src/commands/mod.rs`
    - Accept `serde_json::Value`; treat `Null` as absent; reject anything but an integer 1–5 with the Stability_Range_Message in `into_filters`, before any query
    - _Requirements: 2.9, 7.7, 7.10_

- [x] 5. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, including the existing Appeal sort and paging tests, ask the user if questions arise.

- [x] 6. Add Company_Stability to the CLI
  - [x] 6.1 Add the flags and value parser in `src-tauri/src/cli/args.rs`
    - `parse_company_stability` returns the Stability_Range_Message on any input other than base-10 `1`–`5` (do not use `value_parser!(i64).range`, its message lacks "5 = most stable")
    - `UpdateArgs`: `--company-stability` and `--clear-company-stability` with `conflicts_with`; `ListArgs`: `--min-company-stability`; help text per the design
    - _Requirements: 3.4, 3.5, 3.10, 8.4, 8.6_

  - [x] 6.2 Wire the flags in `src-tauri/src/cli/handlers.rs`
    - `handle_update` maps clear to `Some(None)`, a value to `Some(Some(n))`, otherwise `None`; `handle_list` passes the minimum into `JobFilters`
    - _Requirements: 3.1, 3.2, 3.3, 3.6, 3.7, 3.11, 3.12, 8.1, 8.2, 8.3, 8.5_

  - [x] 6.3 Show Company_Stability in `src-tauri/src/cli/output.rs`
    - `STABILITY` table column after `EASE` and a `Stability:` detail line after `Ease:`, `—` when unscored
    - _Requirements: 3.8, 3.9_

  - [x]* 6.4 Write CLI unit tests
    - Table test of `parse_company_stability` (`"1"`–`"5"` accepted; `"0"`, `"6"`, `"3.5"`, `"03"`, `" 3"`, `"x"`, `""` rejected with the message) and of `ListJobsArgs::into_filters` with valid and invalid JSON values
    - `handle_update` set, clear, and keep; both flags together fail naming both; missing id fails nonzero; table column and detail line; `--help` substrings; `jt list` without the flag is unchanged; `--min-company-stability n --json` with no matches prints `[]`
    - CSV mirror shows the new value after a CLI update (temp data dir)
    - _Requirements: 2.9, 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.8, 3.9, 3.10, 3.12, 7.7, 8.2, 8.3, 8.4, 8.6_

- [x] 7. Round-trip Company_Stability through the CSV_Mirror
  - [x] 7.1 Add the `company_stability` column in `src-tauri/src/jobs/csv.rs`
    - Append it after `application_ease` in `CSV_HEADERS`; add `company_stability` (`#[serde(default)]`) and `company_stability_in_file` (`#[serde(skip)]`) to `EditableFields`, `"companyStability"` to `EDITABLE_KEYS`, and `field_norm`
    - Parse cells with `parse_score_cell(raw, "company stability", "5 = most stable")`; an invalid cell skips the row and records the error
    - The merge loop must skip the field when the column is not in the file; include the field in the post-create `update_job` condition
    - _Requirements: 1.6, 4.1, 4.2, 4.3, 4.4, 4.5, 4.6, 4.7, 4.8_

  - [x]* 7.2 Write property test for the CSV export/import round trip
    - **Property 8: CSV export/import round trip**
    - **Validates: Requirements 4.1, 4.2, 4.3, 4.9, 4.10**

  - [x]* 7.3 Write CSV unit tests in `src-tauri/src/jobs/csv.rs`
    - Exhaustive loop over baseline, CSV, and DB values in `{null, 1..5}` checking the three-way merge result and conflict report
    - Header order and cell counts (update the existing test that expects `application_ease` last); blank cell clears and creates unscored; file without the column keeps stored scores; one invalid row is skipped with "Invalid company stability '<v>'" and later rows import; legacy sidecar JSON loads
    - _Requirements: 1.6, 4.1, 4.4, 4.5, 4.6, 4.7, 4.8_

- [x] 8. Compute Company_Stability_Average for the Companies_Sidebar
  - [x] 8.1 Add the stability aggregate in `src-tauri/src/companies.rs`
    - Guarded sum and count over all Pipeline_Jobs (not only wishlist) in `load_pipeline_aggregates`; emit `stabilityAverage` (unrounded or null) and `stabilityScoredJobCount`
    - _Requirements: 9.1, 9.2, 9.11_

  - [x]* 8.2 Write property test for the company stability average
    - **Property 16: Company stability average matches its definition**
    - Reuse the existing job-spec generator with `company_stability` added
    - **Validates: Requirements 9.1, 9.2, 9.11**

- [x] 9. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 10. Add Company_Stability to the desktop libraries
  - [x] 10.1 Extend desktop types and existing fixtures
    - Add the fields from the design to `desktop/src/lib/schema.ts` (`Job.companyStability`, `JobSummary` pick, sort field, `JobListPage.totalCount`, `CompanyRow.stabilityAverage` and `stabilityScoredJobCount`) and `desktop/src/lib/api.ts` (`minCompanyStability`, `companyStability` on `UpdateJobInput`)
    - The new `CompanyRow` and `Job` fields are required, so update test fixtures that build these objects in full, including `components/CompaniesSidebar.test.tsx`, `lib/company-sidebar.test.ts`, `pages/JobsPage.companies.test.tsx`, `lib/__fixtures__/company-row-arbitraries.ts`, and `lib/company-sidebar.sorting.test.ts`; grep `wishlistEaseAverage` and `applicationEase` in `desktop/src` test files for the rest. Leave `desktop/src/dev/fixtures.ts` unchanged
    - _Requirements: 2.1, 6.1, 7.2, 7.13, 9.1_

  - [x] 10.2 Add the Stability_Sort_Field to `desktop/src/lib/job-list-sort.ts`
    - Add `"companyStability"` to `SORT_FIELDS`, available on all tabs, default direction descending
    - _Requirements: 6.1, 6.2, 6.7, 6.8_

  - [x] 10.3 Add Minimum_Stability_Filter helpers to `desktop/src/lib/job-filters.ts`
    - `parseMinStabilityFromUrl` (exactly `"1"`–`"5"`, else `undefined`), `encodeMinStabilityToUrl`, and `minStability` on `JobFilterDraft` / `filterDraftFromUrl`
    - Move the inline active filter count out of `desktop/src/pages/JobsPage.tsx` into an exported helper (for example `countActiveJobFilters`) that also counts the minimum stability value, and make `JobsPage` call it so the logic exists once
    - _Requirements: 7.5, 7.6, 7.8_

  - [x]* 10.4 Write example tests for sort and filter URL helpers
    - In `job-list-sort.test.ts`: `sortBy=companyStability` with `ascending`, `descending`, absent, and junk `sortOrder` on applied and non-applied tabs
    - In `job-filters.test.ts`: `minStability` parse table (`"1"`–`"5"`, absent, `""`, `"0"`, `"6"`, `"3.5"`, `"03"`, `"x"`), encode/parse round trip for `undefined` and 1–5, and the active filter count rising by exactly 1 when the minimum is set
    - _Requirements: 6.2, 6.7, 6.8, 7.5, 7.6, 7.8_

  - [x] 10.5 Add the Stability_Company_Sort to `desktop/src/lib/company-sidebar.ts`
    - Insert `mostStable` right after `easiestWishlist` in `COMPANY_SORT_OPTIONS`; add `stabilityOf` / `stabilityCountOf` reusing `sanitizeEase` / `sanitizeCount`; add the `sortKeyVector` and `sortAnnotationFor` cases from the design
    - Leave the view-state codec and storage key unchanged
    - _Requirements: 9.3, 9.4, 9.5, 9.6, 9.7, 9.8, 9.9, 9.10, 9.12_

  - [x]* 10.6 Write property test for most-stable company ordering
    - **Property 17: Most-stable company ordering**
    - Extend `lib/__fixtures__/company-row-arbitraries.ts` with valid and invalid `stabilityAverage` and `stabilityScoredJobCount` values; test in `company-sidebar.sorting.test.ts`
    - Add annotation examples alongside (null average gives no annotation; `4.25` → `"4.3"`, `3` → `"3.0"`, label `"Average stability 4.3"`)
    - **Validates: Requirements 9.4, 9.5, 9.6, 9.7, 9.8, 9.12**

- [x] 11. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 12. Add Company_Stability to the desktop components
  - [x] 12.1 Extract `ScoreSelect` and add `CompanyStabilitySelect`
    - Create `desktop/src/components/ScoreSelect.tsx` from the shared body of `AppealSelect` and `ApplicationEaseSelect`, taking `scaleLabel` and `options`; turn both into thin wrappers with unchanged exports, props, and DOM so their tests pass unchanged
    - Create `desktop/src/components/CompanyStabilitySelect.tsx` as a third wrapper exporting `COMPANY_STABILITY_SCALE_LABEL` and `COMPANY_STABILITY_OPTIONS`
    - _Requirements: 5.1, 5.7, 5.11_

  - [x] 12.2 Add the field to `desktop/src/components/JobDetailClient.tsx`
    - Add `companyStability` to the draft, initial value, and `draftsEqual`; render the select (`id="job-company-stability"`) after Application_Ease; send it on save and take the saved baseline from the response
    - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.5, 5.6_

  - [x] 12.3 Show "Stability n" in `desktop/src/components/JobsBoardView.tsx`
    - Card and compact views, only when scored, with no extra separator when unscored
    - _Requirements: 5.12, 5.13_

  - [x] 12.4 Add the row control and update handler to `desktop/src/pages/JobsPage.tsx`
    - Compact `CompanyStabilitySelect` (`id={`stability-${job.id}`}`) with a "Stability" `<label htmlFor>` after the Ease select
    - `handleSetCompanyStability`: optimistic row update, `api.updateJob(jobId, { companyStability: next })`, set the row from the response; then `api.listCompanies()` → `setCompanyRows`, ignoring refetch failures
    - Do not call `load({ quiet: true })` and do not re-sort `jobs`; on update failure restore the previous value, show the message or "Failed to update company stability", and skip the refetch
    - _Requirements: 5.7, 5.8, 5.9, 5.10, 5.11, 6.13, 9.13, 9.14, 9.15_

  - [x] 12.5 Add the sort option, minimum filter, and count to `desktop/src/pages/JobsPage.tsx`
    - "Company stability" sort option after "Appeal" on every tab
    - `jobs-min-stability` select in the filter panel; pass `minCompanyStability` in `loadJobs`, `toggleAllVisibleJobs`, the request key, and `filtersKey` so a change loads page 0 with no cursor
    - Show "{totalCount} matching job(s)" from the first page while any filter or search is active
    - _Requirements: 6.1, 6.7, 6.9, 7.1, 7.5, 7.6, 7.9, 7.13, 7.14_

  - [x]* 12.6 Write unit tests for `CompanyStabilitySelect`, `JobDetailClient`, and `JobsBoardView`
    - Cover only new behavior: stability options and labels; the detail form's stability preselect, dirty flag, and save payload; the "Stability n" label with tooltip shown only when scored
    - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.12, 5.13_

  - [x]* 12.7 Write unit tests for `JobsPage` and `CompaniesSidebar`
    - Row update sends only `companyStability`, applies the response, restores on failure without calling `listCompanies`; success calls `listCompanies` once and does not reload the jobs list; the row keeps its position; a rejected refetch keeps old rows and shows no error; the sidebar re-orders from the new rows and keeps its sort and filters
    - Sort option on every tab (add `companyStability` to the list in `JobsPage.header.test.tsx`); filter options, URL write, Clear all, first-page request after a change; `totalCount` text while filtered
    - "Most stable company" sits right after "Easiest wishlist to apply"; legacy saved view-state strings do not select `mostStable`
    - _Requirements: 5.8, 5.9, 5.10, 6.1, 6.9, 6.13, 7.1, 7.5, 7.9, 7.13, 7.14, 9.3, 9.10, 9.13, 9.14, 9.15_

- [x] 13. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 14. Update agent and user documentation
  - [x] 14.1 Update `AGENTS.md`
    - After the Application_Ease paragraph, document `--company-stability`, `--clear-company-stability`, `jt list --min-company-stability`, the `companyStability` JSON field, and that the flags do not favorite; add the table row from the design
    - _Requirements: 10.1, 10.2, 10.3, 10.4, 10.5, 10.6_

  - [x] 14.2 Update `README.md`
    - In the Jobs CSV section, document the `company_stability` column, blank = unscored, and that a file without the column leaves stored scores unchanged
    - _Requirements: 10.7, 10.8, 10.9_

- [x] 15. Final checkpoint - Ensure all tests pass
  - Run `cargo test` in `src-tauri/` and `npx vitest --run` in `desktop/`. Ensure all tests pass, ask the user if questions arise.

## Notes

- Tasks marked with `*` are optional and can be skipped for a faster MVP.
- Property tests are kept only for Properties 8, 10 + 11 (one test), 12, 16, and 17. The other design properties are covered by the example and table tests named in each task; Properties 7 and 19 are covered by the CSV round trip and the existing view-state tests.
- Task 10.3 edits `JobsPage.tsx` to call the shared filter count helper, so it runs before tasks 12.4 and 12.5.

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1", "1.2", "10.1", "10.2", "12.1"] },
    { "id": 1, "tasks": ["1.3", "2.1", "8.1", "10.3", "10.5", "12.3"] },
    { "id": 2, "tasks": ["3.1", "6.1", "7.1", "8.2", "10.4", "10.6", "12.2"] },
    { "id": 3, "tasks": ["3.2", "4.1", "6.2", "7.2", "12.4"] },
    { "id": 4, "tasks": ["3.3", "6.3", "7.3", "12.5"] },
    { "id": 5, "tasks": ["2.2", "6.4", "12.6", "14.1", "14.2"] },
    { "id": 6, "tasks": ["3.4", "12.7"] },
    { "id": 7, "tasks": ["3.5"] }
  ]
}
```
