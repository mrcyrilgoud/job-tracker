# Implementation Plan: Company Employment Stability

## Overview

Implement company-scoped Company_Stability and Employment_Stability scores across the existing Tauri 2 application. The plan preserves `jobs.company_stability` as a synchronized compatibility cache, adds shared score-service semantics, migrates and backfills existing data, exposes the feature through IPC, CLI, CSV, and desktop views, and updates the required agent and CSV documentation.

Implementation uses Rust and SQLite for persistence and services, Tauri commands for desktop transport, React/TypeScript for the desktop UI, Rust `proptest` for backend properties, and TypeScript `fast-check` with Vitest for sidebar and URL behavior. Do not add an HTTP server, Node/Next backend, MCP server, or in-app LLM.

## Tasks

- [x] 1. Build the company score schema, models, validators, and migration/backfill foundation
  - [x] 1.1 Extend `src-tauri/src/db/migrate.rs` and fresh-schema SQL to add the ten nullable company score/metadata columns with identical strict score and source checks; retain all existing job columns and add no employment score column to `jobs`.
    - Add guarded, individually retryable `ALTER TABLE` steps that refresh `PRAGMA table_info(companies)` after each step and report the named failed migration step.
    - Preserve existing values, make the migration idempotent, and ensure partial column additions can resume on the next run.
    - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 1.8, 1.9_

  - [x] 1.2 Add company score data models and shared normalization/validation in `src-tauri/src/models.rs` and a new `src-tauri/src/company_scores.rs` module.
    - Define complete company score metadata models with camelCase serialization and citation arrays/null in JSON output.
    - Implement tri-state optional-field input, `manual`/`import` sources, exact range messages, rationale trimming and Unicode-code-point limits, citation normalization, URL validation, and null-score metadata invariants.
    - _Requirements: 3.1, 3.2, 3.7, 3.8, 3.9, 3.10, 3.11, 3.12, 3.13, 3.20, 3.21, 5.1, 5.11, 6.13, 6.14_

  - [x] 1.3 Add the one-time backfill conflict schema, marker handling, and transactional legacy-cache backfill in `src-tauri/src/db/migrate.rs`.
    - Persist the `company_employment_stability_backfill_v1` marker and `company_stability_backfill_conflicts` rows atomically with backfill changes.
    - Use every job status when finding distinct legacy cache values, assign one shared UTC timestamp for successful backfills, preserve conflicting caches, and retry the entire backfill when any statement or lock acquisition fails.
    - Ensure repeated migration skips all backfill work after the marker and preserves scored empty companies for later use.
    - _Requirements: 1.10, 1.11, 1.12, 4.1, 4.2, 4.3, 4.4, 4.5, 4.6, 4.10, 4.11_

  - [x] 1.4 Write migration, schema constraint, model, and backfill unit/integration tests in `src-tauri/src/db/migrate.rs`, `src-tauri/src/company_scores.rs`, and related test modules.
    - Cover fresh and legacy schemas, every missing-column subset, idempotence, exact SQLite type checks, invalid source/score atomicity, failed-step retry, marker rollback, busy-lock behavior, empty companies, all job statuses, and conflict persistence.
    - Add stable proptest label `Feature: company-employment-stability, Property 1: Company schema migration is additive and idempotent` for generated legacy schemas and `Feature: company-employment-stability, Property 6: Backfill matches a reference model and is idempotent` for generated company/job cache states.
    - Add stable proptest label `Feature: company-employment-stability, Property 2: Company score writes preserve strict domain and metadata invariants` for invalid values, metadata invariants, and normalization boundaries.
    - _Requirements: 1.1–1.12, 4.1–4.12; validates Design Properties 1, 2, and 6_

- [x] 2. Implement the transactional Score_Service and company lifecycle integration
  - [x] 2.1 Implement `Score_Service::write` and score read helpers in `src-tauri/src/company_scores.rs`.
    - Validate all supplied fields before mutation, distinguish omitted/set/clear, preserve metadata for unchanged and metadata-only writes, use one timestamp for dual score changes, and reject manual metadata attached to a null resulting score.
    - Update only changed company fields, fan out Company_Stability to every related job in the same transaction when the field is carried, preserve all unrelated job fields, and return complete company score metadata.
    - Support import `as_of` overrides and importer warnings without weakening manual write validation.
    - _Requirements: 2.1–2.9, 3.1–3.19, 5.8–5.10, 6.18, 6.21, 6.22, 10.6–10.10, 10.18, 10.19, 10.24_

  - [x] 2.2 Update `src-tauri/src/jobs/service.rs` and job query/model code to use company scores as read source and maintain the cache on insert, move, update, and delete.
    - Join company-owned scores into every serialized `Job`/`JobSummary`, set the cache from the destination company on inserts and moves, and preserve score metadata when resolving an existing company.
    - Add tri-state score fields to job updates, validate score fields before creating a destination company, apply company score and job changes in one transaction, and retain scored empty companies during cleanup.
    - Ensure job events, timestamps, Appeal, Application_Ease, favorites, notes, status, and unrelated jobs remain unchanged during cache fan-out.
    - _Requirements: 1.10–1.12, 2.2–2.9, 5.1–5.7, 5.12–5.13, 12.3, 12.5–12.7_

  - [x] 2.3 Add company-score filtering, sorting, and keyset paging to `src-tauri/src/jobs/service.rs` and query helpers.
    - Add employment minimum filtering conjunctively with company minimum filtering, exclude null scores, sort scored jobs before unscored jobs in either direction, break ties by job id ascending, and use matching count/cursor predicates.
    - Preserve all existing filters, tabs, search, and sort options.
    - _Requirements: 6.10–6.12, 7.8–7.17_

  - [x] 2.4 Add Score_Service, job lifecycle, transaction rollback, parser, sort/paging, and cache invariant tests.
    - Add stable proptest label `Feature: company-employment-stability, Property 3: Score metadata round-trips without unintended normalization` for rationale/citation round trips and metadata-only writes.
    - Add stable proptest label `Feature: company-employment-stability, Property 4: Company stability cache fan-out is exact` for generated operation sequences and unrelated-field snapshots.
    - Add stable proptest label `Feature: company-employment-stability, Property 5: Score and job transactions are atomic` for combined job, move, company creation, and score failures.
    - Add stable proptest labels `Feature: company-employment-stability, Property 7: Strict CLI and IPC parsers accept only their specified values`, `Feature: company-employment-stability, Property 8: Nullable score ordering and paging match the reference`, and `Feature: company-employment-stability, Property 9: Minimum filters are pure conjunctive restrictions` for shared backend behavior.
    - Add examples for clear semantics, unchanged timestamps, dual-score timestamps, citation-limit precedence across Score_Service, IPC/importer, and CSV boundaries, missing ids, destination-company rollback, valid move with destination creation, invalid-score rollback before company creation, later-failure rollback after creation, scored-empty-company retention, unscored cleanup after deleting/moving the last job with watches/careers history, clearing both scores, and no job-event behavior.
    - _Requirements: 1.12, 2.1–2.9, 3.1–3.21, 5.1–5.13, 6.7–6.12, 7.8–7.17, 9.5–9.14, 10.15, 10.18–10.20, 12.5–12.7; validates Design Properties 3–5 and 7–9_

- [x] 3. Add IPC contracts and company/sidebar backend responses
  - [x] 3.1 Extend `src-tauri/src/commands/`, `src-tauri/src/companies.rs`, and shared command registration with `write_company_scores`, score-aware `update_job`, complete company metadata, and validated list minimums.
    - Return `companyStability` and `employmentStability` on every job object, include all ten company score fields in company objects and `list_companies` rows, serialize citations as arrays/null, and remove legacy average/count fields.
    - Reject missing companies, invalid score/rationale/citation payloads, and invalid employment minimums before mutation; mark the CSV mirror dirty only after successful writes.
    - _Requirements: 5.2–5.13, 7.12–7.15_

  - [x] 3.2 Update `desktop/src/lib/schema.ts` and `desktop/src/lib/api.ts` for company score types, job score fields, `writeCompanyScores`, minimum employment filtering, and new sort values.
    - Preserve omitted-versus-null semantics in request types and ensure every response type requires both job score keys.
    - _Requirements: 5.1, 5.8, 5.11, 6.13–6.14, 7.8–7.15_

  - [x] 3.3 Add IPC contract tests covering omitted/null/value fields, complete JSON shapes, invalid payloads, CSV dirty behavior, and company-not-found/job-not-found atomicity.
    - Verify citations are arrays rather than JSON strings and that company scores are returned consistently for archived, closed, watch-created, dismissed, and jobless companies.
    - Exercise citation-list limit precedence at the IPC boundary and verify malformed payloads never dirty the CSV mirror.
    - _Requirements: 5.1–5.13, 7.12–7.15, 12.4, 12.7_

  - [x] 3.4 Update every company mapper, query, projection, and fixture for the complete score shape.
    - Cover `map_company`, company-detail serialization, `list_companies`, `list_companies_with_watches`, job-detail company serialization, company SQL literals, and all full company struct literals so all ten score fields and citation arrays/null are present.
    - Add assertions for scored companies with no jobs and companies represented only by archived, closed, watch-created, or dismissed jobs.
    - _Requirements: 5.1, 5.11, 7.12, 11.1, 11.12, 12.4_

  - [x] 3.5 Update every job projection and column-count assumption for both company scores.
    - Audit `JOB_COLS`, `Job`/`JobSummary` mappers, detail/list/paged projections, create/watch/CSV paths, SQL column literals, and all Rust/TypeScript fixtures; assert projection column counts and both score keys in every response.
    - _Requirements: 2.4–2.6, 5.1, 5.2, 5.11, 9.1–9.3, 12.4_

- [x] 4. Implement CLI score commands and score-file importer
  - [x] 4.1 Extend `src-tauri/src/cli/args.rs` and CLI command dispatch for both score flags, rationale flags, clear flags, employment minimums, exact help text, conflicts, and pre-database parsing.
    - Preserve leading-zero acceptance such as `03`; reject signs, whitespace, decimals, missing values, and out-of-range values with the exact company/employment messages.
    - Replace plain list/detail stability output with `CO. STABILITY`, `EMP. STABILITY`, and full score labels; include complete company metadata and both score keys in JSON.
    - Add ordinary non-quiet one-time Backfill_Conflict reporting at CLI startup for every ordinary command, write only to stderr, mark conflicts reported only after successful stderr output, and keep stdout parseable for JSON. Explicitly cover JSON, quiet, and run-jobs suppression while leaving conflicts unreported in those modes.
    - _Requirements: 4.7–4.8, 6.1–6.9, 6.13–6.20, 12.2–12.4, 12.7_

  - [x] 4.2 Implement the score-file planner, top-level validator, and summary envelope in `src-tauri/src/cli/` and a dedicated importer module.
    - Validate the file before writes, enforce the 5000-entry cap, preserve zero-based positions, map all six supported keys, ignore unmapped fields, normalize `as_of`, and produce applied/unchanged/unmatched/conflicts/invalid/warnings/failed groups.
    - Keep JSON output to exactly one parseable object and implement quiet/dry-run formatting and exit classification without opening the database for file-level failures.
    - _Requirements: 10.1–10.7, 10.10, 10.15–10.17, 10.21, 10.23–10.25_

  - [x] 4.3 Implement score-file matching, merged conflict planning, and per-entry transactional application.
    - Implement tracker-name precedence, name fallback, ambiguity handling, duplicate-entry grouping, and one merged conflict record when ambiguity and duplicate matching affect the same company; write none of the affected entries.
    - Apply valid entries through Score_Service with null/omitted semantics, discarded-metadata warnings, citation-limit precedence, `as_of` handling, per-entry rollback, CSV refresh, and continuation after failures.
    - _Requirements: 10.5–10.14, 10.18–10.20, 10.24–10.25_

  - [x] 4.4 Add CLI and importer tests for help/output contracts, parser grammar, conflict reporting, metadata preservation, dry-run/apply parity, idempotence, file validation, matching, warnings, per-entry failures, and mirror-refresh failure.
    - Add stable proptest label `Feature: company-employment-stability, Property 12: Score-file planning is deterministic and idempotent` for generated Score_Files and repeated applies.
    - Add stable proptest label `Feature: company-employment-stability, Property 13: Import matching conflicts are merged per company` for ambiguous names and duplicate entries, including the case where both affect the same company; assert one merged conflict record and no affected writes.
    - Add stable proptest label `Feature: company-employment-stability, Property 15: Score scope never changes posting scores or favorites` across CLI/importer writes.
    - Cover ordinary non-quiet startup conflict reporting, JSON stdout parseability, quiet/run-jobs suppression, and reported-marker writes only after successful stderr output.
    - Cover citation-list limit precedence in importer validation and the dual-invalid CLI case: one first-error message is acceptable, SQLite is never opened, and the database remains unchanged.
    - _Requirements: 4.7–4.8, 6.1–6.9, 6.13–6.22, 10.1–10.25, 12.5–12.7; validates Design Properties 12, 13, and 15_

- [x] 5. Extend the CSV mirror for company-scoped scores
  - [x] 5.1 Update `src-tauri/src/jobs/csv.rs` export/import models, headers, sidecar baselines, and merge/overwrite application.
    - Place `employment_stability` immediately after `company_stability` after `application_ease`, repeat stored company values on every row, preserve legacy files/sidecars missing employment, and emit no metadata columns.
    - Parse trimmed score cells with existing plus/leading-zero behavior, skip invalid rows, group changed cells by destination company, enforce agreement, apply stored-value-wins conflicts, preserve non-score fields, support dry runs, and export/rebaseline after real writes.
    - _Requirements: 9.1–9.15_

  - [x] 5.2 Add CSV field-list, sidecar, and export tests in `src-tauri/src/jobs/csv.rs` and related fixtures.
    - Cover `CSV_HEADERS`, `EditableFields`, presence flags, `EDITABLE_KEYS`, `field_norm`, export column order/cell counts, company repetition, old sidecars, missing employment columns, and post-import export plus sidecar rebaseline.
    - Add stable proptest label `Feature: company-employment-stability, Property 10: CSV score cells and unedited files round-trip` for formatting/parsing and unedited export/import.
    - _Requirements: 9.1–9.4, 9.13, 9.15–9.17_

  - [x] 5.3 Add CSV merge, overwrite, and lifecycle tests.
    - Cover merge and overwrite modes, new rows, company moves, changed-cell presence tracking, baseline/database/CSV conflicts, disagreement isolation, blank clears, invalid-row continuation, dry-run no-write behavior, citation-list limit precedence at the CSV boundary, and preservation of metadata/cache/job fields.
    - Add stable proptest label `Feature: company-employment-stability, Property 11: CSV three-way merge is baseline-correct` for baseline/database/CSV combinations, company agreement, conflict handling, and company moves.
    - _Requirements: 9.5–9.14; validates Design Property 11_

- [x] 6. Implement desktop Jobs list, board, detail, company editor, and sidebar
  - [x] 6.1 Update `desktop/src/pages/JobsPage.tsx` and supporting list/filter components for Company and Employment selects, optimistic company-wide updates, minimum employment URL state, filters, counts, and sorts.
    - Keep rows in place until reload, send only the changed field, restore every affected row on failure, preserve favorites/navigation behavior, and refetch the sidebar without surfacing refetch errors.
    - Add exact compact/full labels, titles, helper text, option sets, URL sanitization, active filter counts, and Applied-tab sort options.
    - _Requirements: 7.1–7.17, 12.1–12.4, 12.7_

  - [x] 6.2 Update `desktop/src/components/JobsBoardView.tsx` to show both company score chips and summary segments with null omission and exact ordering/titles.
    - Replace bare Stability text with Company and Employment labels while preserving Appeal/Ease controls, positions, styling, and existing closed/archived favorites behavior.
    - _Requirements: 7.18–7.23, 12.1–12.4, 12.7_

  - [x] 6.3 Update `desktop/src/components/JobDetailClient.tsx` for both company score selects, changed-only update payloads, company helper/link, read-only rationale/citation display, and failed-save dirty-state preservation.
    - Use the existing leave-page warning behavior and open citation links through the existing system-browser convention.
    - _Requirements: 8.1–8.6, 8.13, 12.2–12.4, 12.6–12.7_

  - [x] 6.4 Add `CompanyScoresEditor` and integrate it into `desktop/src/pages/CompanyDetailPage.tsx`.
    - Implement two score sections with rationale and citation editing, exact local validation, disabled metadata controls for Unscored, changed-only payloads, read-only scored-at/source, clear notices, failed-save retention, and unsaved leave/reload warning.
    - _Requirements: 8.7–8.16, 12.2–12.4, 12.7_

  - [x] 6.5 Update `desktop/src/lib/company-sidebar.ts` and sidebar components for stored score sorting, `mostStableEmployment`, filters, annotations, saved state, and malformed-score sanitization.
    - Use Name_Order and null-last ordering, omit annotations for an unscored active target, preserve old saved views, and never use averages, cache values, or job-level scores.
    - _Requirements: 11.1–11.13, 12.1–12.4_

  - [x] 6.6 Add TypeScript Vitest/fast-check tests for Jobs list/board/detail/editor/sidebar behavior and URL state.
    - Add stable property label `Feature: company-employment-stability, Property 14: Sidebar sorting sanitizes malformed scores and suppresses irrelevant annotations` for malformed values, filters, ordering, and annotations.
    - Cover exact option labels/titles, optimistic success/failure restoration, changed-only payloads, null separators, helper links, citation/rationale validation, leave warnings, saved sort state, invalid URL values, filter counts, and score-scope preservation.
    - _Requirements: 7.1–7.23, 8.1–8.16, 11.1–11.13, 12.1–12.7; validates Design Property 14_

  - [x] 6.7 Add focused CompanyScoresEditor and company-detail integration tests.
    - Cover both sections, disabled metadata for Unscored, 2000-character and 20-entry validation, citation URL validation, changed-only omitted/null/value payloads, clear notices, failed-save retention, returned metadata replacement, and leave/reload warnings.
    - _Requirements: 8.7–8.16, 12.2–12.4, 12.7_

- [x] 7. Update documentation and repository contracts
  - [x] 7.1 Update `AGENTS.md` to replace the per-job company-stability description and document company scope, both scales, clear/rationale flags, metadata JSON, Backfill_Conflict reporting, score-file import, CSV refresh, exit rules, and unchanged Appeal/Application_Ease/favorite behavior.
    - Update only the specified command-table rows/additions and keep unrelated command rows unchanged.
    - Ensure every feature flag, subcommand, JSON key, and CSV field matches the implemented CLI/help/output spelling.
    - _Requirements: 13.1–13.6, 13.9–13.10_

  - [x] 7.2 Update the Jobs CSV section of `README.md` for both score columns, company repetition, blank/unscored behavior, absence of metadata columns, baseline/merge/conflict rules, invalid-row behavior, and stored-value-wins semantics.
    - Place `employment_stability` immediately after `company_stability` in the editable-column documentation.
    - _Requirements: 13.7–13.8, 13.10_

  - [x] 7.3 Add documentation contract tests or targeted repository checks for required flags, mappings, JSON fields, CSV columns, scales, importer exit rules, and removal of stale per-job wording.
    - _Requirements: 13.1–13.10_

- [x] 8. Integrate, reconcile regression coverage, and perform final validation
  - [x] 8.1 Rewrite existing company-stability tests and fixtures that assert per-job semantics to assert company scope, synchronized cache behavior, and the new two-score contract.
    - Remove contradictory expectations while retaining unrelated regression coverage for Appeal, Application_Ease, favorites, status, events, and existing CSV behavior.
    - _Requirements: 2.3, 3.14, 3.16, 3.17, 6.18, 9.16, 10.19, 12.5–12.7_

  - [x] 8.2 Wire all modules, command registrations, CLI dispatch, database migration startup, CSV refresh paths, and desktop API call sites, then run the complete Rust and desktop test suites.
    - Verify `cargo test` from `src-tauri/` and `npx vitest --run` from `desktop/`; resolve compile, serialization, transaction, and snapshot failures without adding out-of-scope infrastructure.
    - _Requirements: 1.1–13.10_

  - [x] 8.3 Run final automated acceptance checks for migration/backfill, Score_Service invariants, CLI/IPC/CSV contracts, importer idempotence, UI behavior, documentation spelling, and all 15 design properties.
    - Confirm no HTTP/Node/Next/MCP/in-app LLM code was introduced, no `employment_stability` job column was added, and no score operation changes posting scores or favorites.
    - _Requirements: 1.1–13.10_

## Notes

- Tasks are ordered so schema and shared service contracts precede all transport and UI work.
- Property-based tests are required because `design.md` defines Correctness Properties 1–15; each property has a stable feature/property label in its task.
- Tasks marked with no `*` are required implementation or validation work. No test task is optional because the requirements and design explicitly require the corresponding coverage.
- Score metadata remains company-owned. `jobs.company_stability` is only a synchronized compatibility cache, and Employment_Stability never gets a jobs-table column.
- The implementation must use the existing Tauri invoke path and CLI/service architecture; do not create a server or alternate agent API.

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1", "1.2"] },
    { "id": 1, "tasks": ["1.3", "2.1"] },
    { "id": 2, "tasks": ["1.4", "2.2", "2.3"] },
    { "id": 3, "tasks": ["2.4", "3.1", "3.2", "5.1"] },
    { "id": 4, "tasks": ["3.3", "3.4", "3.5", "4.1"] },
    { "id": 5, "tasks": ["4.2", "4.3", "5.2", "6.1", "6.2", "6.5"] },
    { "id": 6, "tasks": ["4.4", "5.3", "6.3", "6.4", "7.1", "7.2"] },
    { "id": 7, "tasks": ["6.6", "6.7", "7.3", "8.1"] },
    { "id": 8, "tasks": ["8.2"] },
    { "id": 9, "tasks": ["8.3"] }
  ]
}
```
