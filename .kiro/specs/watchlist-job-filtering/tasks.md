# Implementation Plan: Watchlist Job Filtering

## Overview

This plan converts the Watchlist Job Filtering design into incremental coding steps for the existing Tauri desktop app (Rust backend in `src-tauri/`, React/TypeScript frontend in `desktop/`). Work proceeds bottom-up: first the pure Rust filter model, alias table, and engine (the single matching authority), then the resolver and DB migration, then integration into query-time listing and ingest-time sync, then the Tauri command surface, then the frontend editor, with property-based and unit tests woven in close to each unit. Every step builds on the previous one and ends by wiring new code into an existing call site so there is no orphaned code. A hard, non-destructive constraint runs through the whole plan: filtering only changes what is displayed at query time and must never delete jobs or alter CSV export row selection (Requirement 18).

Correctness properties from the design (Properties 1–12) are turned into `proptest`-based property tests, each as its own optional sub-task annotated with its property number and the requirement clauses it validates.

## Tasks

- [x] 1. Scaffold the filtering module and criteria model
  - [x] 1.1 Create the `filtering` module skeleton and register it
    - Create `src-tauri/src/filtering/mod.rs` re-exporting `model`, `aliases`, `engine`, `resolver` submodules (create empty submodule files `model.rs`, `aliases.rs`, `engine.rs`, `resolver.rs`)
    - Declare `pub mod filtering;` in `src-tauri/src/lib.rs`
    - Add `proptest` to `[dev-dependencies]` in `src-tauri/Cargo.toml` for later property tests
    - _Requirements: 10.2, 11.1_

  - [x] 1.2 Implement the Filter_Criteria data model in `filtering/model.rs`
    - Define `FilterCriteria`, `TitleCriteria`, `LocationCriteria`, `MatchMode` (Word default, Substring), `RemoteMode` (Any default, RemoteOnly, OnsiteOnly) with serde `rename_all = "camelCase"`, `#[serde(default)]` on fields, and `version` defaulting to 1
    - Implement `FilterCriteria::match_all()` (all lists empty, RemoteMode::Any) and `is_match_all()`
    - Implement a normalization-on-write helper that trims tokens and drops empties (used later by setters, Req 17.3)
    - _Requirements: 3.2, 6.6, 17.3_

  - [x]* 1.3 Write unit tests for the model
    - Test camelCase JSON round-trip serialize/deserialize, default `version`, `match_all()`/`is_match_all()` equivalence, and token trim/drop-empty helper
    - _Requirements: 3.2, 17.3_

- [x] 2. Implement the alias table
  - [x] 2.1 Implement `AliasTable` and its default seed in `filtering/aliases.rs`
    - Define `AliasTable { version, regions, countries, remote_tokens }` with serde camelCase
    - Implement `default_seed()` with parity to the current `expand_country_keywords`/`expand_location_keywords` coverage: all US state names + two-letter abbreviations, major US city hubs, and regions `bay area`, `greater new york`, `greater seattle`, `greater los angeles`; `remote_tokens` = `remote`, `anywhere`, `distributed`, `wfh`
    - _Requirements: 5.8, 6.5_

  - [x] 2.2 Implement location expansion, country disambiguation, and remote detection
    - Implement `expand_location(token, country)`: region tokens expand to self + members; unknown tokens pass through unchanged (Req 5.4); `ca` expands to California tokens only when country is United States, never for Canada or when country is absent (Req 5.5–5.7)
    - Implement country expansion contribution to the effective include set and `is_remote(location_norm)` using `remote_tokens` case-insensitively
    - _Requirements: 5.1, 5.4, 5.5, 5.6, 5.7, 6.5_

  - [x]* 2.3 Write property test for country disambiguation
    - **Property 6: Country disambiguation**
    - **Validates: Requirements 5.5, 5.6, 5.7**

  - [x]* 2.4 Write unit tests for alias expansion and remote detection
    - Test region expansion, unknown-token passthrough, dedup expectations, and `is_remote` token detection
    - _Requirements: 5.1, 5.4, 6.5_

- [x] 3. Implement the pure filter engine
  - [x] 3.1 Implement normalization and tokenization in `filtering/engine.rs`
    - Implement `normalize` (lowercase, trim, collapse internal whitespace runs to a single space) and `tokenize` (split on whitespace and delimiters `, / ( ) - •`, drop empty tokens)
    - Define `JobView<'a> { title, location }` and `MatchResult { included, reason }`
    - _Requirements: 1.4, 1.5, 1.7_

  - [x] 3.2 Implement `term_matches` for Word and Substring modes
    - Word mode: single-token term matches a whole token only; multi-word term matches as an ordered contiguous token run; zero tokens => no Word match; empty-after-normalization terms report no match
    - Substring mode: normalized term matches as a contiguous substring regardless of boundaries
    - _Requirements: 1.1, 1.2, 1.3, 1.6, 1.7, 2.2, 2.3_

  - [x] 3.3 Implement `matches` applying full precedence and remote/location gates
    - Short-circuit `is_match_all` to included with reason "no criteria" (Req 3.3); exclude-wins-over-include on both title and location; include is any-of; empty include satisfies its dimension; apply RemoteOnly/OnsiteOnly/Any gates; expand+dedup the effective location include set; remote-bypasses-location-name rule when RemoteMode != OnsiteOnly and no non-remote place present; return `MatchResult` with human-readable reason
    - Keep the function pure and deterministic (no I/O, no input mutation)
    - _Requirements: 2.1, 2.4, 2.5, 2.6, 2.7, 2.8, 3.1, 5.2, 5.3, 6.1, 6.2, 6.3, 6.4, 11.2, 11.3_

  - [x]* 3.4 Write property test: empty criteria matches all
    - **Property 1: Empty criteria matches all**
    - **Validates: Requirements 3.1**

  - [x]* 3.5 Write property test: exclude precedence
    - **Property 2: Exclude precedence**
    - **Validates: Requirements 2.1**

  - [x]* 3.6 Write property test: include is any-of (disjunctive)
    - **Property 3: Include is any-of**
    - **Validates: Requirements 2.4, 2.5**

  - [x]* 3.7 Write property test: case-insensitivity
    - **Property 4: Case-insensitivity**
    - **Validates: Requirements 4.1, 4.2**

  - [x]* 3.8 Write property test: no substring false positives in Word mode (ca/California ambiguity class)
    - **Property 5: No substring false positives in Word mode**
    - **Validates: Requirements 1.1, 1.2**

  - [x]* 3.9 Write property test: determinism & purity
    - **Property 7: Determinism & purity**
    - **Validates: Requirements 11.1, 11.2**

  - [x]* 3.10 Write property test: remote gate soundness
    - **Property 11: Remote gate soundness**
    - **Validates: Requirements 6.1, 6.2, 6.3**

  - [x]* 3.11 Write property test: whitespace/punctuation invariance
    - **Property 12: Whitespace/punctuation invariance**
    - **Validates: Requirements 4.3**

  - [x]* 3.12 Write unit tests for engine edge cases
    - Test empty-token text, multi-word contiguous runs, exclude-over-include on the location dimension, and the remote-bypasses-location-name rule
    - _Requirements: 1.2, 1.7, 2.1, 6.4_

- [x] 4. Checkpoint - engine layer green
  - Ensure all tests pass, ask the user if questions arise.

- [x] 5. Implement the filter resolver and settings access
  - [x] 5.1 Implement global and per-watch criteria accessors in `filtering/resolver.rs`
    - Implement `get_global_criteria`/`set_global_criteria` against `app_settings` key `filter_criteria_v1`, `get_watch_criteria`/`set_watch_criteria` against `company_watches.filter_criteria`, and `load_alias_table` (default seed on absent/invalid, Req 16.2)
    - On deserialize failure or unknown version, fall back to `match_all()` for that scope and log a warning (Req 16.1)
    - Setters normalize/trim tokens and drop empties before persisting (Req 17.3)
    - _Requirements: 16.1, 16.2, 17.3_

  - [x] 5.2 Implement `resolve_effective_criteria` with per-call caching
    - Return per-watch override if present, else global, else `match_all()` (override replaces, never field-merges; null override returns exactly global)
    - Cache resolved criteria per `(company_id, source)` for a single list call so mid-call criteria changes do not affect an in-flight call (Req 7.4, 7.5)
    - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5_

  - [x]* 5.3 Write property test: override replaces, not merges
    - **Property 9: Override replaces, not merges**
    - **Validates: Requirements 7.1, 7.2**

  - [x]* 5.4 Write unit tests for resolver fallback and caching
    - Test match_all fallback when both scopes absent, malformed-JSON fail-open to match_all (Req 16.1), and mid-call cache stability (Req 7.5)
    - _Requirements: 7.3, 7.5, 16.1_

- [x] 6. Implement DB migration and legacy conversion
  - [x] 6.1 Add additive schema columns in `src-tauri/src/db/migrate.rs`
    - Add nullable `company_watches.filter_criteria TEXT` and nullable `jobs.watch_filtered INTEGER` columns using the existing PRAGMA table_info guard + `ALTER TABLE` pattern (idempotent, no data loss)
    - _Requirements: 9.2, 14.5_

  - [x] 6.2 Seed the alias table and migrate legacy settings idempotently
    - If `location_aliases_v1` is absent, seed `AliasTable::default_seed()` (Req 14.7)
    - If `filter_criteria_v1` is absent, build global `FilterCriteria` from legacy `location_country`/`location_cities`/`watch_role_keywords`: keywords -> `title.include` with Word mode, country -> `location.country` when non-empty, cities -> `location.include`, remote Any; preserve non-empty legacy values without loss or reordering; empty/absent keywords => empty title include with no failure recorded
    - Guard both with key-absent checks so repeated runs produce identical output and never overwrite user-modified criteria; retain legacy keys; on failure leave prior state intact and surface a migration error
    - _Requirements: 14.1, 14.2, 14.3, 14.4, 14.5, 14.6, 14.7_

  - [x]* 6.3 Write property test: idempotent migration
    - **Property 8: Idempotent migration**
    - **Validates: Requirements 14.4**

  - [x]* 6.4 Write unit tests for migration mapping and guards
    - Test legacy->structured field mapping, empty-keyword case (no failure), legacy keys retained after migration, and re-run does not overwrite existing criteria
    - _Requirements: 14.1, 14.2, 14.3, 14.5_

- [x] 7. Checkpoint - resolver and migration green
  - Ensure all tests pass, ask the user if questions arise.

- [x] 8. Integrate the engine into query-time listing
  - [x] 8.1 Replace hardcoded location SQL in `jobs::service::list_jobs`
    - Remove the inline `expand_country_keywords`/`expand_location_keywords` SQL block; fetch candidates with a coarse SQL pre-filter (state + provider) bounded by a fetch cap, resolve criteria per `(company_id, source)` via the resolver cache, evaluate `engine::matches` in memory, keep only included rows, and apply `LIMIT` after filtering
    - _Requirements: 8.1, 8.2, 8.3, 8.4_

  - [x] 8.2 Replace the duplicated location SQL in `list_open_watch_positions` and `cli::handlers::load_watch_positions`
    - Remove the duplicated `expand_*` blocks and route both through the shared resolver + `engine::matches` path so all watch listings use the single engine authority; apply limit after filtering
    - _Requirements: 8.1, 8.2, 8.3, 10.1, 10.2_

  - [x]* 8.3 Write unit/integration tests for query-time filtering
    - Test that changing criteria re-filters existing rows without a re-sync (Req 8.4), that limit applies post-filter (Req 8.3), and that no rows are deleted by listing (Req 18.1, 18.5)
    - _Requirements: 8.3, 8.4, 18.1, 18.5_

- [x] 9. Integrate the engine into ingest-time sync (non-destructive annotation)
  - [x] 9.1 Wire `engine::matches` into `ats::sync::apply_watch_sync`
    - Resolve effective criteria per watch, evaluate each ingested remote role, and set the `jobs.watch_filtered` hint to the inclusion result; store every remote role regardless of result; never delete a stored job; when a role's result is not included, retain the job and leave any previously recorded hint value unchanged
    - _Requirements: 9.1, 9.2, 9.3, 9.4, 9.5, 10.1_

  - [x]* 9.2 Write property test: query/ingest agreement
    - **Property 10: Query/ingest agreement**
    - **Validates: Requirements 10.1, 10.2**

  - [x]* 9.3 Write unit tests for non-destructive sync annotation
    - Test that all roles are stored regardless of match, no deletion occurs, and a not-included result leaves the prior hint unchanged (Req 9.5)
    - _Requirements: 9.3, 9.4, 9.5_

- [x] 10. Verify CSV export remains independent of filter criteria
  - [x] 10.1 Audit and lock `jobs::csv::load_job_rows` selection
    - Confirm `load_job_rows` selects rows by existing pipeline-tracking rules only, with no reference to `filter_criteria` or `watch_filtered`; add a code comment marking the non-destructive contract
    - _Requirements: 18.2, 18.3, 18.4_

  - [x]* 10.2 Write regression test: CSV export unaffected by filter criteria
    - Set restrictive global criteria that would omit tracked jobs from watch listings, then assert `load_job_rows` output (and exported CSV row set) is identical to output with match-all criteria
    - _Requirements: 18.2, 18.3, 18.4_

- [x] 11. Checkpoint - backend integration green
  - Ensure all tests pass, ask the user if questions arise.

- [x] 12. Implement the Tauri command surface
  - [x] 12.1 Add filter commands in `commands/mod.rs` and register in `lib.rs`
    - Implement `get_filter_criteria`, `set_filter_criteria`, `get_watch_filter_criteria`, `set_watch_filter_criteria` (None clears override), and `preview_filter_match` (returns `{ included, reason }` per sample); register all in the `tauri::generate_handler!` list
    - Validate on write: reject unknown version or non-array token fields without persisting, return watch-not-found for unknown `watch_id`, and trim/drop-empty tokens; a later valid set overwrites previously bad data
    - _Requirements: 12.1, 12.2, 12.3, 12.4, 12.5, 16.3, 17.1, 17.2, 17.3_

  - [x] 12.2 Wire legacy setters to write through to structured criteria
    - Keep `get/set_watch_role_keywords` and `get/set_location_settings_cmd` registered; have their setters write through to `filter_criteria_v1`, and fail the legacy setter with an error if the write-through does not succeed
    - _Requirements: 15.1, 15.2, 15.3_

  - [x]* 12.3 Write unit tests for command validation and legacy write-through
    - Test invalid-version/non-array rejection without persist (Req 17.1), watch-not-found (Req 17.2), null override clears (Req 12.4), preview output shape (Req 12.5), and legacy setter fails on write-through error (Req 15.3)
    - _Requirements: 12.4, 12.5, 15.3, 17.1, 17.2_

- [x] 13. Implement frontend types and API wrappers
  - [x] 13.1 Add FilterCriteria types and api.ts wrappers
    - Add `MatchMode`, `RemoteMode`, and `FilterCriteria` TypeScript types plus `getFilterCriteria`, `setFilterCriteria`, `getWatchFilterCriteria`, `setWatchFilterCriteria`, `previewFilterMatch` wrappers in `desktop/src/lib/api.ts`
    - Add optional `filterCriteria: FilterCriteria | null` to the `CompanyWatch` type in `desktop/src/lib/schema.ts`
    - _Requirements: 12.1, 12.2, 12.3, 12.4, 12.5_

- [x] 14. Implement the shared FilterCriteriaEditor and wire it into the UI
  - [x] 14.1 Build the shared `FilterCriteriaEditor` component
    - Create `desktop/src/components/companies/FilterCriteriaEditor.tsx` with title include/exclude chip inputs, a country selector, location include/exclude chip inputs, a remote-mode selector (Any / Remote only / Onsite only), and a live preview list driven by `previewFilterMatch`
    - _Requirements: 13.1, 13.5_

  - [x] 14.2 Wire the global editor into SettingsPage
    - Replace the free-text "Role Keywords" and "Location Preferences" sections in the Settings page with a single Global Filter editor using `FilterCriteriaEditor`, loading via `getFilterCriteria` and saving via `setFilterCriteria`
    - _Requirements: 13.1, 13.5_

  - [x] 14.3 Wire the per-watch editor into WatchRow
    - Add a collapsible "Filter for this board" control to `desktop/src/components/companies/WatchRow.tsx` that shows "Using global filter" when no override exists, saves customizations via `setWatchFilterCriteria(watch.id, criteria)`, and offers a "Reset to global" action calling `setWatchFilterCriteria(watch.id, null)`
    - _Requirements: 13.2, 13.3, 13.4_

  - [x]* 14.4 Write vitest tests for editor logic
    - Following the `desktop/src/lib/*.test.ts` patterns, test chip add/trim/drop-empty, "using global" vs override state derivation, reset-to-global producing a null override, and preview result mapping
    - _Requirements: 13.2, 13.3, 13.4, 13.5, 17.3_

- [x] 15. Final checkpoint - full stack green
  - Ensure all Rust and frontend tests pass, ask the user if questions arise.

## Notes

- Tasks marked with `*` are optional test sub-tasks and can be skipped for a faster MVP; core implementation tasks are never optional.
- Each task references specific requirement clauses for traceability; all 18 requirements are covered.
- Property-based tests (using `proptest`) validate the 12 universal correctness properties from the design; unit and integration tests cover specific examples and edge cases.
- Requirement 18 (non-destructive filtering and CSV preservation) is enforced by tasks 9.1, 10.1, and the regression tests in 8.3, 9.3, and 10.2.
- Checkpoints (tasks 4, 7, 11, 15) ensure incremental validation at layer boundaries.

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1"] },
    { "id": 1, "tasks": ["1.2", "2.1"] },
    { "id": 2, "tasks": ["1.3", "2.2"] },
    { "id": 3, "tasks": ["2.3", "2.4", "3.1"] },
    { "id": 4, "tasks": ["3.2"] },
    { "id": 5, "tasks": ["3.3"] },
    { "id": 6, "tasks": ["3.4", "3.5", "3.6", "3.7", "3.8", "3.9", "3.10", "3.11", "3.12"] },
    { "id": 7, "tasks": ["5.1"] },
    { "id": 8, "tasks": ["5.2", "6.1"] },
    { "id": 9, "tasks": ["5.3", "5.4", "6.2"] },
    { "id": 10, "tasks": ["6.3", "6.4", "8.1"] },
    { "id": 11, "tasks": ["8.2", "9.1"] },
    { "id": 12, "tasks": ["8.3", "9.2", "9.3", "10.1"] },
    { "id": 13, "tasks": ["10.2", "12.1"] },
    { "id": 14, "tasks": ["12.2", "13.1"] },
    { "id": 15, "tasks": ["12.3", "14.1"] },
    { "id": 16, "tasks": ["14.2", "14.3"] },
    { "id": 17, "tasks": ["14.4"] }
  ]
}
```
