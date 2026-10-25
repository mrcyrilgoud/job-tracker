# Implementation Plan: Enhanced Filters

## Overview

This plan implements multi-select status, date-range, and location/remote filtering across the shared Rust `Job_Service`, the Tauri IPC boundary, the `jt list` CLI, and the desktop React UI. Work proceeds bottom-up by layer so each step is testable in isolation before it is wired in: pure helpers first (Rust `partition_statuses`/`escape_like`/date-boundary mapping and TS `job-filters.ts` parse/encode/validate), then the parameterized SQL fragments in `append_job_filters`, then the IPC and CLI call sites that validate input and construct `JobFilters`, and finally the `JobsPage.tsx` popover, URL round-trip, and active-filter count. Property-based tests (Rust `proptest`, TS `fast-check`) cover the 12 correctness properties and are placed next to the code they exercise. The existing single `status` field is preserved so applied-tab sort and cursor pagination behavior stays intact; the new multi-select uses the separate `statuses` field.

## Tasks

- [x] 1. Add pure Rust helpers for status partitioning, LIKE escaping, and date boundaries
  - [x] 1.1 Implement `partition_statuses`, `escape_like`, `date_column`, and date-boundary mapping
    - Add `partition_statuses(requested: &[String]) -> (Vec<String>, Vec<String>)` to `src-tauri/src/jobs/service.rs` (or `models.rs`), using `is_job_status` to split valid/invalid, preserving first-seen order and dropping duplicates from the valid set
    - Add `escape_like(input: &str) -> String` escaping `\`, `%`, and `_` for use with `ESCAPE '\'`
    - Add `date_column(field: Option<&str>) -> Option<&'static str>` mapping `appliedAt`→`j.applied_at`, `postingDate`→`j.created_at`, `lastCheckedAt`→`j.last_checked_at`, returning `None` for any other value (static map, no user text reaches SQL)
    - Add a pure date-boundary helper that, given a validated `YYYY-MM-DD` string, produces the inclusive start boundary (bare date) and end boundary (date suffixed with `~` / `0x7E`) per the Data Models section
    - _Requirements: 1.8, 2.3, 2.4, 2.5, 2.6_

  - [x]* 1.2 Write property test for status partition
    - **Property 2: Mixed-validity status partition**
    - **Validates: Requirements 1.8**
    - Use `proptest`, min 100 cases; generators must include duplicate tokens, non-member tokens, and mixed lists
    - Tag: `// Feature: enhanced-filters, Property 2: Mixed-validity status partition`

- [x] 2. Add new fields to Rust `JobFilters`
  - [x] 2.1 Extend `JobFilters` struct with optional filter fields
    - In `src-tauri/src/jobs/service.rs` add `statuses: Option<Vec<String>>`, `remote_mode: Option<String>`, `date_field: Option<String>`, `date_from: Option<String>`, `date_to: Option<String>`, plus precomputed `date_from_boundary` / `date_to_boundary` string fields
    - Keep all fields `Option`, defaulting to `None`, so existing callers compile unchanged; preserve the existing single `status` field
    - _Requirements: 1.2, 2.3, 3.5_

- [x] 3. Implement the parameterized SQL fragments in the shared service
  - [x] 3.1 Add the multi-status `IN (...)` fragment to `append_job_filters`
    - Apply only when `statuses` is `Some` and non-empty; build `AND j.status IN (?, …)` with a placeholder per value and bind each as a param; combine conjunctively with any single `status`
    - _Requirements: 1.2, 1.3, 1.4, 4.3_

  - [x] 3.2 Add the date-range fragment to `append_job_filters`
    - Apply only when `date_field` maps to a column via `date_column` and at least one bound is present; emit `AND {col} IS NOT NULL`, then `AND {col} >= ?` / `AND {col} <= ?` for present boundaries, binding the precomputed boundary strings as params
    - _Requirements: 2.3, 2.4, 2.5, 2.6, 2.11, 2.12_

  - [x] 3.3 Extend the location fragment and add the remote-mode fragment in `append_job_filters`
    - Add `AND j.location IS NOT NULL` when non-empty location text or a `remoteOnly`/`onsiteOnly` mode is active; keep the existing `LIKE ? ESCAPE '\'` text branch using `escape_like`
    - Add the remote-mode branches: `remoteOnly` → OR of `j.location LIKE ?` over the four indicator terms; `onsiteOnly` → one `AND j.location NOT LIKE ?` per term; `any`/`None` → no restriction
    - Define `REMOTE_TERMS = ["remote", "anywhere", "work from home", "wfh"]`
    - _Requirements: 3.3, 3.4, 3.5, 3.6, 3.7, 3.9_

  - [x]* 3.4 Write property test for date-range inclusivity with null exclusion
    - **Property 5: Date-range inclusivity with null exclusion**
    - **Validates: Requirements 2.3, 2.4, 2.5, 2.6, 2.11, 2.12**
    - `proptest` against in-memory SQLite; generators include bare-date vs full-ISO timestamps, `00:00:00` and end-of-day values, and null field values
    - Tag: `// Feature: enhanced-filters, Property 5: Date-range inclusivity with null exclusion`

  - [x]* 3.5 Write property test for location-text substring match
    - **Property 7: Location-text substring match**
    - **Validates: Requirements 3.3, 3.4, 3.9**
    - `proptest` against in-memory SQLite; generators include `%`/`_`/mixed-case/null-location strings and empty/whitespace text
    - Tag: `// Feature: enhanced-filters, Property 7: Location-text substring match`

  - [x]* 3.6 Write property test for remote-mode predicate
    - **Property 8: Remote-mode predicate**
    - **Validates: Requirements 3.5, 3.6, 3.7, 3.9**
    - `proptest` against in-memory SQLite; generators include indicator-term substrings (e.g. "Greater Remoteville"), onsite strings, and null locations
    - Tag: `// Feature: enhanced-filters, Property 8: Remote-mode predicate`

- [x] 4. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 5. Wire the IPC boundary (`ListJobsArgs` + `From` impl)
  - [x] 5.1 Extend `ListJobsArgs` and its `From<ListJobsArgs> for JobFilters` impl
    - In `src-tauri/src/commands/mod.rs` add camelCase fields `statuses`, `remote_mode`, `date_field`, `date_from`, `date_to`
    - In the `From` impl resolve `statuses` via `partition_statuses` and return the Req 1.9 error ("No valid Job_Status value was provided") when every value is a non-member, otherwise apply the valid set and drop invalid values
    - Validate `date_field` ∈ domain and dates via the shared date validator; on invalid date or start>end return an error; compute and store the day-boundary strings on `JobFilters`
    - Confirm `list_jobs_cmd`, `list_jobs_page_cmd`, and `list_job_ids_cmd` inherit the behavior through `.into()` with no further change
    - _Requirements: 1.8, 1.9, 2.7, 2.8, 4.3_

  - [x]* 5.2 Write unit tests for the IPC status/date validation boundary
    - Test all-invalid status → error; some-invalid → dropped; invalid date and start>end → error
    - _Requirements: 1.8, 1.9, 2.7, 2.8_

- [x] 6. Add CLI flags and strict validation in `handle_list`
  - [x] 6.1 Extend `ListArgs` with the new flags
    - In `src-tauri/src/cli/args.rs` change `status` to `Vec<String>` with `value_delimiter = ','` and `num_args = 1..`; add `date_field`, `date_from`, `date_to`, and `remote` options with help text
    - _Requirements: 6.1, 6.2, 6.3_

  - [x] 6.2 Implement strict validation and `JobFilters` construction in `handle_list`
    - In `src-tauri/src/cli/handlers.rs` reject any non-member `--status` value, non-member `--remote`, non-member `--date-field`, malformed date, or `date_from > date_to` with an `AppError` naming the bad input (nonzero exit, no rows)
    - Map `--status` members to `statuses` (single member → one-element `statuses`; CLI does not use the single `status` field); generalize the applied-tab default `is_archived` logic so a non-empty `statuses` suppresses the default `is_archived = Some(false)`
    - Ensure `--json` emits the filtered results in the existing `jt list --json` shape
    - _Requirements: 1.7, 1.10, 3.12, 3.13, 6.1, 6.2, 6.3, 6.4, 6.6, 6.7, 6.8_

  - [x]* 6.3 Write unit tests for CLI error contract
    - Assert nonzero exit, named offending value, and no rows for invalid status/remote/date-field/date inputs
    - _Requirements: 1.10, 3.13, 6.6, 6.7_

  - [x]* 6.4 Write property test for CLI / IPC parity
    - **Property 12: CLI / IPC parity**
    - **Validates: Requirements 6.1, 6.2, 6.3, 6.5**
    - `proptest`: build equivalent `JobFilters` via both paths and assert identical rows from `list_jobs` against in-memory SQLite
    - Tag: `// Feature: enhanced-filters, Property 12: CLI / IPC parity`

- [x] 7. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 8. Add remaining shared-service property tests for composition
  - [x]* 8.1 Write property test for status-set membership filtering
    - **Property 1: Status-set membership filtering**
    - **Validates: Requirements 1.2, 1.3, 1.4, 1.7**
    - `proptest` against in-memory SQLite; include the empty-set (no restriction) case
    - Tag: `// Feature: enhanced-filters, Property 1: Status-set membership filtering`

  - [x]* 8.2 Write property test for conjunctive composition
    - **Property 9: Conjunctive composition**
    - **Validates: Requirements 3.8, 4.1, 4.2, 4.3**
    - `proptest`: combined result equals the intersection of each active filter applied alone, over an in-memory job model
    - Tag: `// Feature: enhanced-filters, Property 9: Conjunctive composition`

  - [x]* 8.3 Write property test for order-independence
    - **Property 10: Order-independence (confluence)**
    - **Validates: Requirements 4.4**
    - `proptest`: reordering status values and order-insensitive inputs yields an identical job set
    - Tag: `// Feature: enhanced-filters, Property 10: Order-independence (confluence)`

- [x] 9. Extend the TypeScript `JobFilters` type and `api.ts`
  - [x] 9.1 Add the new optional fields to the TS `JobFilters` type
    - In `desktop/src/lib/api.ts` add `statuses?: string[]`, `remoteMode?: RemoteMode`, `dateField?: "appliedAt" | "postingDate" | "lastCheckedAt"`, `dateFrom?: string`, `dateTo?: string`, preserving the single `status` field
    - _Requirements: 1.2, 2.9, 3.10_

- [x] 10. Implement pure `job-filters.ts` parse/encode/validate helpers
  - [x] 10.1 Add status, date, and remote URL helpers plus date validators
    - In `desktop/src/lib/job-filters.ts` add `parseStatusesFromUrl` (dedupe + drop non-members), `encodeStatusesToUrl` (comma-joined, canonical `jobStatuses` order), `isValidCalendarDate` (strict `YYYY-MM-DD`), `validateDateRange` (null = ok), `parseDateFilterFromUrl`, and `parseRemoteModeFromUrl` (non-member → `any`)
    - All parse helpers must be total (never throw) and drop malformed/out-of-domain values
    - _Requirements: 1.5, 1.6, 2.2, 2.9, 2.10, 3.10, 3.11, 5.1, 5.2, 5.6_

  - [x]* 10.2 Write property test for URL round-trip and normalization
    - **Property 3: URL Filter_State round-trip and normalization**
    - **Validates: Requirements 1.5, 1.6, 2.9, 2.10, 3.10, 3.11, 5.1, 5.2, 5.5**
    - `fast-check` in `desktop/src/lib/job-filters.test.ts`, `{ numRuns: 100 }`; inactive dimension encodes to absent param and decodes to inactive
    - Tag: `// Feature: enhanced-filters, Property 3: URL Filter_State round-trip and normalization`

  - [x]* 10.3 Write property test for restore robustness against malformed input
    - **Property 4: Restore robustness against malformed input**
    - **Validates: Requirements 5.6, 1.6**
    - `fast-check`: arbitrary param maps never throw and yield only in-domain values
    - Tag: `// Feature: enhanced-filters, Property 4: Restore robustness against malformed input`

  - [x]* 10.4 Write property test for date-range validation
    - **Property 6: Date-range validation**
    - **Validates: Requirements 2.7, 2.8**
    - `fast-check`: validator rejects iff a date is invalid `YYYY-MM-DD` or start is strictly later than end
    - Tag: `// Feature: enhanced-filters, Property 6: Date-range validation`

- [x] 11. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 12. Build the Filter_Popover controls and draft state in `JobsPage.tsx`
  - [x] 12.1 Add popover controls for status set, date field/range, and location/remote
    - Extend `JobDraft`/`JobFilterDraft` with `statuses: JobStatus[]`, `dateField`, `dateFrom`, `dateTo`, `location`, `remoteMode`
    - Render the multi-select status control (zero to eight members), the single-select date-field control with start/end date inputs, the 0–200 char location text input, and the single-select remote-mode control
    - _Requirements: 1.1, 2.1, 2.2, 3.1, 3.2_

  - [x] 12.2 Implement submit, restore, and empty-result wiring
    - In `handleFilterSubmit` call `validateDateRange` first; on failure set `filterError` and return without `setSearchParams` (retain applied Filter_State); otherwise write the status set, date triple, location, and remote params (omitting empties), preserving sort, favorites, and archived
    - Implement the restore effect using the `job-filters.ts` helpers, dropping malformed/out-of-domain values and showing a visible notice when a value was ignored
    - Render the empty-result state (no rows + message) when the active Filter_State matches zero jobs
    - _Requirements: 2.7, 2.8, 4.5, 5.1, 5.2, 5.3, 5.4, 5.5, 5.6_

  - [x] 12.3 Extend the active-filter indicator count
    - Add the number of active new dimensions (0–3: status set non-empty, date filter active, location filter active) to the existing active-filter count
    - _Requirements: 4.6_

  - [x]* 12.4 Write property test for active-filter count
    - **Property 11: Active-filter count**
    - **Validates: Requirements 4.6**
    - `fast-check` in `job-filters.test.ts` (extract the count logic into a pure helper), `{ numRuns: 100 }`
    - Tag: `// Feature: enhanced-filters, Property 11: Active-filter count`

  - [x]* 12.5 Write component tests for popover controls and submit preservation
    - Assert the controls render the correct domains and that submit preserves sort/favorites/archived
    - _Requirements: 1.1, 2.1, 2.2, 3.1, 3.2, 5.3, 5.4_

- [x] 13. Final checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- Each task references specific requirements for traceability
- Checkpoints ensure incremental validation
- Property tests validate the 12 universal correctness properties from the design; unit/component tests cover rendering, CLI error contracts, and `--json` shape
- The single `status` field is preserved for applied-tab sort/cursor behavior; the multi-select popover uses the separate `statuses` field
- All user-supplied values flow through parameterized SQL; only static placeholders, column names, and operators are interpolated
- Backend stays Rust + SQLite with the CLI mirroring the shared `Job_Service`; no Node/Next server is introduced

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1", "2.1", "9.1"] },
    { "id": 1, "tasks": ["1.2", "3.1", "3.2", "3.3", "10.1"] },
    { "id": 2, "tasks": ["3.4", "3.5", "3.6", "5.1", "6.1", "10.2", "10.3", "10.4"] },
    { "id": 3, "tasks": ["5.2", "6.2", "8.1", "8.2", "8.3", "12.1"] },
    { "id": 4, "tasks": ["6.3", "6.4", "12.2", "12.3"] },
    { "id": 5, "tasks": ["12.4", "12.5"] }
  ]
}
```
