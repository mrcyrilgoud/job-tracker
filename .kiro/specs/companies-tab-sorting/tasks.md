# Implementation Plan: companies-tab-sorting

## Overview

Rust computes six new per-company pipeline aggregates in `list_companies_with_watches` (`src-tauri/src/companies.rs`), using a strict timestamp grammar in a new `src-tauri/src/instant.rs`. The desktop UI gains a matching TypeScript parser (`desktop/src/lib/instant.ts`), eight new Sort_Options and two new Company_Filters in `desktop/src/lib/company-sidebar.ts`, an annotation slot in `SidebarLink`, menu separators and a scrollable Sort_Menu in `CompaniesSidebar`, and failed-reload retention in `JobsPage`. Both parsers are checked against one shared fixture corpus. Work proceeds bottom-up: Rust grammar, Rust aggregates, TS types and grammar, pure sidebar logic, components, page, CSS.

Commands used by checkpoints:

- Rust: `cargo test --manifest-path src-tauri/Cargo.toml --lib` (root `npm test`)
- Desktop tests: `npm test` in `desktop/` (`vitest run`)
- Desktop typecheck: `npx tsc -b` in `desktop/` (the repo has no separate lint script)

## Tasks

- [x] 1. Rust timestamp grammar and shared corpus
  - [x] 1.1 Create the shared timestamp fixture corpus
    - Create `desktop/src/lib/__fixtures__/instant-corpus.json` with `valid` entries (`input`, `epochMs`, `subMsNanos`) and `invalid` strings as listed in design §Shared timestamp corpus
    - Extend it with leap-day (`2024-02-29`, invalid `2025-02-29`), year-boundary, lowercase `t`/`z`, negative-offset, space-separator, and minutes-only time cases; compute each `epochMs` from the equivalent `Z` form
    - _Requirements: 1.3, 4.5_

  - [x] 1.2 Implement `src-tauri/src/instant.rs` and register it in `src-tauri/src/lib.rs`
    - `parse_instant(&str) -> Option<DateTime<Utc>>` as a hand-written byte scanner for the grammar in design §1 (no leading/trailing whitespace, years 0001–9999, valid days per month, HH 00–23, no leap second, 1–9 fraction digits, `Z`/`z`/`±HH:MM` offsets), built with `NaiveDate::from_ymd_opt`, `NaiveTime::from_hms_nano_opt`, and `FixedOffset::east_opt`; do not use `parse_from_rfc3339`
    - Date-only values map to 00:00:00 UTC; offset-less date-times are UTC
    - `LatestTimestamp` with `offer(&str)` (ignores invalid strings; replaces on later instant, or equal instant and greater string by bytes) and `into_value() -> Option<String>` returning the stored string verbatim
    - _Requirements: 1.2, 1.3, 1.4, 4.5_

  - [x] 1.3 Write property test for timestamp parsing round trip (Rust half) plus corpus test
    - **Property 5: Timestamp parsing round trip and cross-language agreement**
    - Generate date/time/fraction/separator/offset components, format to a grammar string, compare `parse_instant` with a `chrono` value built from the components; `with_cases(100)`
    - Corpus test reads `instant-corpus.json` via `include_str!("../../desktop/src/lib/__fixtures__/instant-corpus.json")` and checks every valid entry's instant and every invalid entry's rejection
    - **Validates: Requirements 1.3, 4.5**

  - [x] 1.4 Write property test for order-independent latest-timestamp selection
    - **Property 3: Latest-timestamp selection is order-independent**
    - Shuffle candidate lists mixing valid, invalid, and equal-instant/different-string values; result equals the max under (instant, string bytes) or None
    - **Validates: Requirements 1.3, 1.4**

- [x] 2. Rust per-company pipeline aggregates
  - [x] 2.1 Extend `list_companies_with_watches` in `src-tauri/src/companies.rs`
    - Add `const PIPELINE_JOB_PREDICATE` and Query A (counts plus guarded ease sum/count), Query B (`applied_at` candidates), and Query C (`DISTINCT` user-activity `occurred_at` candidates excluding `posting_state_changed` and `discovered_from_watch`) exactly as in design §3
    - Add private `CompanyPipelineAggregates` and a pure helper `fold_pipeline_aggregates(counts, applied_pairs, activity_pairs)` that folds timestamps through `LatestTimestamp` and computes `wishlistEaseAverage = ease_sum / ease_count` (None when count is 0)
    - Run all queries, existing and new, inside one `conn.unchecked_transaction()` that is dropped without commit; propagate errors through `map_sqlite`/`?` so no partial list is returned
    - Add the six new JSON fields to each row with defaults (0 / null) for companies absent from the maps; keep existing fields, company set, and `ORDER BY name` order unchanged
    - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7, 1.8, 1.9, 1.10, 1.12_

  - [x] 2.2 Write property test for pipeline counts and ease average
    - **Property 1: Pipeline counts and ease average match the job model**
    - Extend the test `JobSpec` with `applied_at`, `application_ease`, `is_new_from_watch`, `watch_disposition`, and all Job_Statuses; model counts directly from glossary definitions; check `wishlist + applied ≤ pipeline` and `interviewing ≤ applied`
    - **Validates: Requirements 1.1, 1.5, 1.6, 1.7, 1.8**

  - [x] 2.3 Write property test for latest timestamps
    - **Property 2: Latest timestamps match the instant model and ignore background events**
    - Use a hand-written pool of `(string, Option<expected_instant>)` entries so the model does not call the parser; event type pool includes `created`, `status_changed`, `note_added`, `posting_state_changed`, `discovered_from_watch`; assert returned strings are byte-identical to stored values
    - **Validates: Requirements 1.2, 1.3, 1.4, 1.12**

  - [x] 2.4 Write property test for unchanged existing fields and order
    - **Property 4: Existing row fields and order are unchanged**
    - Re-run the prior spec's Property 12 and Property 13 checks with the extended generator; assert id order equals `list_companies`
    - **Validates: Requirements 1.9**

  - [x] 2.5 Write Rust example tests for aggregate edge cases
    - Dropping `job_events` (with `PRAGMA foreign_keys = OFF`) makes the call return `Err` and leaves `jobs`/`companies` row counts unchanged
    - A company whose only jobs are archived, dismissed, or `is_new_from_watch = 1` returns zeros and nulls
    - `closed` with `applied_at` counts as applied; `closed` without it counts toward neither
    - A later `posting_state_changed` event leaves `lastActivityAt` unchanged
    - _Requirements: 1.6, 1.10, 1.12_

- [x] 3. Checkpoint - Rust
  - Run `cargo test --manifest-path src-tauri/Cargo.toml --lib`. Ensure all tests pass, ask the user if questions arise.

- [x] 4. TypeScript row type and timestamp grammar
  - [x] 4.1 Add the six fields to `CompanyRow` in `desktop/src/lib/schema.ts`
    - Add `wishlistJobCount`, `appliedJobCount`, `interviewingJobCount`, `lastActivityAt`, `lastAppliedAt`, `wishlistEaseAverage` with the doc comments from design §4
    - Update existing `CompanyRow` builders in `desktop/src/lib/company-sidebar.test.ts`, `desktop/src/components/CompaniesSidebar.test.tsx`, and `desktop/src/pages/JobsPage.companies.test.tsx` to default the new fields (0 / null) so the project typechecks
    - _Requirements: 1.1, 1.2, 1.5_

  - [x] 4.2 Implement `desktop/src/lib/instant.ts`
    - `parseInstant(unknown)` with one anchored regex, range and days-in-month checks, `setUTCFullYear`/`setUTCHours` construction (not `Date.UTC`), `subMsNanos` from fraction digits 4–9, offset subtracted in minutes; null for non-strings and anything outside the grammar
    - `compareInstants`, `calendarDateOf(value, timeZone?)` (date-only keeps literal date; date-time via `Intl.DateTimeFormat(...).formatToParts` in `timeZone`), and `formatShortDate(date, now, locale?, timeZone?)` (month short + day, year only when it differs from `now`'s year in `timeZone`, formatted with timeZone `"UTC"` from the CalendarDate)
    - _Requirements: 4.5, 4.7, 6.4_

  - [x] 4.3 Write property test for timestamp parsing round trip (TS half) plus corpus test
    - **Property 5: Timestamp parsing round trip and cross-language agreement**
    - New `desktop/src/lib/instant.test.ts`; same generator shape as the Rust test, expected `ms` from components; import `instant-corpus.json` and check every entry; `numRuns: 100`
    - **Validates: Requirements 1.3, 4.5**

  - [x] 4.4 Write unit tests for `calendarDateOf` and `formatShortDate`
    - Fixed zones `UTC`, `America/Los_Angeles`, `Asia/Tokyo`, `Pacific/Kiritimati`, locale `en-US`: date-only values keep literal date in every zone; date-times shift by zone; "Mar 4" vs "Mar 4, 2025" with `now` in 2026
    - `2026-10-01` compares before `2026-10-01T00:00:01Z` and equal to `2026-10-01T00:00:00Z`
    - _Requirements: 4.5, 4.7, 6.4_

- [x] 5. Sidebar sort, filter, and annotation logic (`desktop/src/lib/company-sidebar.ts`)
  - [x] 5.1 Extend the Sort_Option and Company_Filter tables
    - Replace `COMPANY_SORT_OPTIONS` with the 13-entry table including `group` (`name`/`count`/`activity`) and the "Recently added company" label; keep `CompanySortOption`, `isCompanySortOption`, and `parseCompanyViewState` derived from it
    - Append `hasWishlistJobs` ("Has wishlist jobs") and `hasAppliedJobs` ("Has applied jobs") to `COMPANY_FILTERS` so old serialized states stay byte-identical
    - _Requirements: 2.1, 2.2, 2.6, 2.7, 7.1, 9.1, 9.2, 9.3, 9.4, 9.5, 9.6, 9.7_

  - [x] 5.2 Add sanitizers, accessors, and the new filter predicates
    - `sanitizeCount` (whole number ≥ 0 else 0) and `sanitizeEase` (finite in [1, 5] else null)
    - `wishlistCountOf`, `appliedCountOf`, `interviewingCountOf`, `easeOf`, `lastActivityOf`, `lastAppliedOf` (the last two via `parseInstant`); leave `openPositionCount`/`trackedJobCount` reads unchanged
    - Add `hasWishlistJobs` and `hasAppliedJobs` cases to `matchesCompanyFilter`
    - _Requirements: 3.10, 5.5, 7.3, 7.4, 7.5_

  - [x] 5.3 Generalize `sortCompanies` to key vectors
    - Decorate each row once with the key vector from the design §6 table, compare element-wise with `sign`, then `compareByNameOrder`; keep `nameAsc`/`nameDesc`/`recentlyAdded` behavior unchanged; never mutate input rows or array
    - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 3.8, 3.9, 3.11, 4.1, 4.2, 4.3, 4.4, 4.6, 5.1, 5.2, 5.3, 5.4, 5.6, 7.6, 8.1, 8.2, 8.3, 8.4, 8.5_

  - [x] 5.4 Implement `sortAnnotationFor` and `formatEase`
    - `SortAnnotation { text, label }` and `AnnotationContext { now, locale?, timeZone? }` per the design §6 annotation table, with singular/plural labels, "Last activity {text}", "Last applied {text}", "Average ease {text}", null for name sorts and null/invalid values
    - `formatEase(x)` = `(Math.round(x * 10 + 1e-9) / 10).toFixed(1)`
    - Keep `sortCountFor` exported until task 7.2 removes its last caller
    - _Requirements: 4.7, 4.8, 6.1, 6.2, 6.3, 6.4, 6.5, 6.6, 6.7, 6.8, 6.9_

  - [x] 5.5 Write property test for per-Sort_Option ordering
    - **Property 6: Each Sort_Option orders rows by its specification**
    - Reference comparator written from the requirement text, separate from the key-vector code; generators with collisions (small name pool with case/accent variants, counts 0..3, averages including 4.6, 4.666…, 4.25, equal instants in different forms, invalid strings)
    - **Validates: Requirements 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 4.1, 4.2, 4.3, 4.4, 4.6, 5.1, 5.2, 5.3, 5.4, 8.5**

  - [x] 5.6 Write property test for malformed aggregates
    - **Property 7: Malformed aggregates are treated as their sanitized values**
    - **Validates: Requirements 3.10, 5.5**

  - [x] 5.7 Write property test for sort permutation, idempotence, determinism, and non-mutation
    - **Property 8: Sorting is a deterministic, idempotent, non-mutating permutation**
    - Small generated sizes plus one 10,000-row example across all 13 Sort_Options
    - **Validates: Requirements 3.9, 8.1, 8.2, 8.3, 8.4, 8.7**

  - [x] 5.8 Write property test for the derived list model
    - **Property 9: Derived list matches the search, six-filter, and sort model**
    - **Validates: Requirements 3.8, 5.6, 7.3, 7.4, 7.5**

  - [x] 5.9 Write property test for filtered relative order
    - **Property 10: Filtering preserves strict relative order**
    - **Validates: Requirements 7.6**

  - [x] 5.10 Write property test for date annotations
    - **Property 11: Date annotations show the calendar date consistently**
    - Fixed zones `UTC`, `America/Los_Angeles`, `Asia/Tokyo`, `Pacific/Kiritimati`; locale `en-US`
    - **Validates: Requirements 4.7, 4.8, 6.4, 6.5**

  - [x] 5.11 Write property test for ease rounding
    - **Property 12: Ease annotation rounds the exact mean half away from zero**
    - Expected string from integers: `floor((20k + n) / (2n)) / 10` for n in 1..50, k in n..5n
    - **Validates: Requirements 6.6**

  - [x] 5.12 Write property test for count annotations
    - **Property 13: Count annotations show the sanitized count with a pluralized label**
    - **Validates: Requirements 6.1, 6.2, 6.3, 6.7, 6.8, 6.9**

  - [x] 5.13 Write property test for view-state round trip
    - **Property 14: View-state persistence round trip is canonical and backward compatible**
    - Extend the existing codec property to 13 sorts and six filters; assert pre-feature states serialize byte-identically
    - **Validates: Requirements 2.8, 9.1, 9.2, 9.4**

  - [x] 5.14 Write property test for tolerant view-state parsing
    - **Property 15: Stored view-state parsing never fails and sanitizes**
    - **Validates: Requirements 2.7, 9.5, 9.6, 9.7**

  - [x] 5.15 Write example tests and widen prior-spec properties
    - Tables equal the 13-entry list (Req 2.1) and six-entry filter list (Req 7.1)
    - 4.67 sorts above 4.6 under `easiestWishlist` and annotates "4.7" / "4.6"
    - `formatEase(4.25) === "4.3"`, `formatEase(4) === "4.0"`, `formatEase(23 / 20) === "1.2"`
    - Labels "1 wishlist job", "3 wishlist jobs", "0 applied jobs", "Last applied Mar 4", "Last activity Mar 4, 2025", "Average ease 4.5"
    - Widen the prior spec's toggle, filter-order independence, search/filter commutation, and empty-state precedence properties to the six filters
    - _Requirements: 2.1, 5.2, 6.6, 6.7, 7.1, 7.8, 7.9_

- [x] 6. Checkpoint - desktop logic
  - Run `npm test` and `npx tsc -b` in `desktop/`. Ensure all tests pass, ask the user if questions arise.

- [x] 7. Sidebar components
  - [x] 7.1 Add the `annotation` prop to `desktop/src/components/SidebarLink.tsx`
    - `annotation?: { text: string; label: string }` takes precedence over `count`; render `<span aria-hidden="true">{text}</span><span className="sr-only">{label}</span>` in the trailing slot; existing `count` callers unchanged
    - _Requirements: 6.7, 6.8_

  - [x] 7.2 Update `desktop/src/components/CompaniesSidebar.tsx`
    - Render a non-focusable `<div role="separator" className="companies-sidebar-sep" />` wherever `group` changes between adjacent Sort_Options; keep the arrow-key handler querying only menu item roles
    - On open and on window `resize` while open, set inline `maxHeight = innerHeight - sortButton.bottom - 14` (minimum 120) on the Sort_Menu and add class `companies-sidebar-menu--sort`; focus moves (including initial focus) use `focus({ preventScroll: true })` then `scrollIntoView({ block: "nearest" })`
    - Replace `sortCountFor` with `sortAnnotationFor(row, view.sort, { now: new Date() })` inside the visible-rows `useMemo` and pass it as `annotation`; then remove `sortCountFor` from `company-sidebar.ts` and move any remaining tests of it to `sortAnnotationFor`
    - Filter_Menu maps the six-entry `COMPANY_FILTERS`; toggles keep updating list, "N of M companies", and badge in the same render without calling the backend
    - _Requirements: 2.2, 2.3, 2.4, 2.5, 2.8, 6.10, 7.2, 7.7, 7.8, 7.9, 9.3, 9.8, 9.9_

  - [x] 7.3 Write component tests in `desktop/src/components/CompaniesSidebar.test.tsx`
    - 13 `menuitemradio` items in order and exactly two `role="separator"` elements at the specified positions with no `tabindex`
    - Arrow keys cross separators without landing on them and wrap; each option selection sets `aria-checked="true"` on exactly that item and writes `localStorage`
    - With stubbed `getBoundingClientRect` for a short window, the menu gets an inline `max-height` and the scroll class, and arrow navigation calls `scrollIntoView({ block: "nearest" })`
    - Switching sort replaces annotations in the same render; rerendering with new `rows` reorders and reannotates without changing sort or filters
    - Toggling `hasWishlistJobs`/`hasAppliedJobs` updates the list, count text, and badge with no `listCompanies` call; "Clear filters" clears all six and keeps sort and search
    - A throwing `setItem` keeps the selected sort applied with no error UI; search text never appears in storage
    - _Requirements: 1.11, 2.1, 2.2, 2.3, 2.4, 2.5, 2.8, 3.11, 4.9, 6.10, 7.2, 7.7, 7.8, 8.6, 9.3, 9.8, 9.9_

- [x] 8. Jobs page reload retention
  - [x] 8.1 Keep previous company rows on a failed reload in `desktop/src/pages/JobsPage.tsx`
    - Track a `companiesLoadedRef` set on successful `listCompanies`; on rejection after a prior success, leave `companyRows` and `companiesLoad` unchanged; first-load failure keeps the existing error state; successful reloads replace `companyRows`
    - _Requirements: 1.11, 8.6, 8.8_

  - [x] 8.2 Write page tests in `desktop/src/pages/JobsPage.companies.test.tsx`
    - First rejection still shows "Couldn't load companies."; a successful load followed by a rejected reload keeps the list order, sort, and filters
    - _Requirements: 8.8_

- [x] 9. Styles
  - [x] 9.1 Add Sort_Menu styles to `desktop/src/index.css`
    - `.companies-sidebar-menu--sort { overflow-y: auto; overscroll-behavior: contain; width: 14rem; }` and a `.companies-sidebar-sep` divider rule matching the existing menu theme in light and dark modes
    - _Requirements: 2.2, 2.5_

- [x] 10. Final checkpoint
  - Run `cargo test --manifest-path src-tauri/Cargo.toml --lib`, then `npm test` and `npx tsc -b` in `desktop/`. Ensure all tests pass, ask the user if questions arise.

## Notes

- Tasks marked with `*` are optional and can be skipped for a faster MVP.
- Each property test uses at least 100 iterations and carries the tag comment `Feature: companies-tab-sorting, Property {number}: {property_text}` plus a `**Validates: Requirements X.Y**` line.
- `list_companies_with_watches` is called only by the `list_companies` Tauri command, so CLI output is unaffected.
- No schema migration; the feature reads existing columns only.

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1", "1.2", "4.1"] },
    { "id": 1, "tasks": ["1.3", "2.1", "4.2", "5.1"] },
    { "id": 2, "tasks": ["1.4", "2.2", "4.3", "5.2"] },
    { "id": 3, "tasks": ["2.3", "4.4", "5.3"] },
    { "id": 4, "tasks": ["2.4", "5.4", "7.1"] },
    { "id": 5, "tasks": ["2.5", "5.5", "7.2", "8.1", "9.1"] },
    { "id": 6, "tasks": ["5.6", "7.3", "8.2"] },
    { "id": 7, "tasks": ["5.7"] },
    { "id": 8, "tasks": ["5.8"] },
    { "id": 9, "tasks": ["5.9"] },
    { "id": 10, "tasks": ["5.10"] },
    { "id": 11, "tasks": ["5.11"] },
    { "id": 12, "tasks": ["5.12"] },
    { "id": 13, "tasks": ["5.13"] },
    { "id": 14, "tasks": ["5.14"] },
    { "id": 15, "tasks": ["5.15"] }
  ]
}
```
