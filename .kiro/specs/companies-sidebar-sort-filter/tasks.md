# Implementation Plan: Companies Sidebar Sort and Filter

## Overview

Build bottom-up. First add the `trackedJobCount` aggregate in Rust and update the `CompanyRow` type. Then build the pure `company-sidebar.ts` module (sort, filter, derive, view-state codec, empty-state selection) with its property tests. Then add the `CompaniesSidebar` component, and finally wire it into `JobsPage.tsx` with a separate companies load status. Rust is tested with `cargo test` in `src-tauri/` (proptest). TypeScript is tested with `npm test` (`vitest run`, fast-check, jsdom) and typechecked with `npm run build` (`tsc -b && vite build`) in `desktop/`. Tauri `invoke` is the only data path. There is no Node server, no new IPC command, and no CLI change. `mockup.html` is a design artifact and is not shipped.

## Tasks

- [x] 1. Add `trackedJobCount` to the company data service
  - [x] 1.1 Compute tracked job counts in `list_companies_with_watches`
    - In `src-tauri/src/companies.rs`, add the `SELECT company_id, COUNT(*) FROM jobs WHERE status <> 'archived' GROUP BY company_id` query into a `HashMap<String, i64>`, using the same pattern as the open-position counts
    - Emit `"trackedJobCount"` on every row, defaulting to `0` for companies with no entry
    - Propagate query errors with `?` before any row is built, so no partial list is returned
    - Leave company order, `company`, `watches`, `reviews`, and `openPositionCount` unchanged
    - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.6, 4.7_

  - [x]* 1.2 Write property test for tracked job counts
    - **Property 12: Tracked job count matches the non-archived job model**
    - proptest with `ProptestConfig::with_cases(100)` on the in-memory `test_connection()`: 0–8 companies, 0–40 jobs with random company, every status (including `archived` and `closed`), source, `posting_state`, and favorite flag
    - Tag: `// Feature: companies-sidebar-sort-filter, Property 12: ...`
    - **Validates: Requirements 4.1, 4.2, 4.3, 4.4**

  - [x]* 1.3 Write property test for unchanged existing fields and order
    - **Property 13: Existing row fields and order are unchanged**
    - Same generator as Property 12. Assert row order matches `list_companies(conn)` and `openPositionCount` matches the active ATS-job model
    - **Validates: Requirements 4.6**

  - [x]* 1.4 Write unit test for the error path
    - Drop the `jobs` table on a test connection and assert `list_companies_with_watches` returns `Err`
    - _Requirements: 4.7_

- [x] 2. Update the `CompanyRow` type and existing fixtures
  - [x] 2.1 Add `trackedJobCount: number` to `CompanyRow`
    - Edit `desktop/src/lib/schema.ts`
    - Compute `trackedJobCount` (jobs for the company with `status !== "archived"`) in `desktop/src/dev/fixtures.ts`
    - Add `trackedJobCount` to any other `CompanyRow` literals in tests and mocks so `tsc -b` passes. `CompaniesClient.tsx` and `CompanyCard.tsx` ignore the field
    - _Requirements: 4.1_

- [x] 3. Checkpoint - Ensure all tests pass
  - Run `cargo test` in `src-tauri/`, and `npm test` plus `npm run build` in `desktop/`. Ensure all tests pass, ask the user if questions arise.

- [x] 4. Implement the pure sidebar logic module
  - [x] 4.1 Create sort and filter primitives in `desktop/src/lib/company-sidebar.ts`
    - Define `COMPANY_SORT_OPTIONS`, `CompanySortOption`, `DEFAULT_COMPANY_SORT`, `COMPANY_FILTERS`, `CompanyFilter`, `CompanyViewState`, `DEFAULT_COMPANY_VIEW_STATE`
    - Implement `compareByNameOrder` with one shared `Intl.Collator(undefined, { sensitivity: "accent" })`, tie-breaking on `id` by code unit
    - Implement `sortCompanies` (new array, no mutation): `nameAsc`, `nameDesc` as the exact reverse, `openPositionsDesc`, `trackedJobsDesc`, `recentlyAdded` (unparseable `createdAt` → `-Infinity`), all compared through a `sign()` helper with Name_Order fallback
    - Implement `matchesCompanyFilter` and `applyCompanyFilters` (conjunction, order-preserving, empty set returns rows unchanged)
    - Implement `sortCountFor`, `toggleCompanyFilter`, `clearCompanyFilters`
    - _Requirements: 2.5, 2.6, 2.7, 2.8, 2.9, 2.12, 2.13, 2.14, 3.2, 3.3, 3.4, 3.7, 3.9_

  - [x]* 4.2 Write property test for sort key ordering
    - **Property 1: Sort order follows the option's key, then Name_Order**
    - Create `desktop/src/lib/company-sidebar.test.ts` with shared fast-check arbitraries (unique ids, small mixed-case and accented name alphabet, counts 0–5, repeated ISO timestamps plus an invalid one, watches with `consecutiveSyncFailures` 0–2, 0–2 reviews), `{ numRuns: 100 }`
    - **Validates: Requirements 2.5, 2.7, 2.8, 2.9**

  - [x]* 4.3 Write property test for Name Z–A reversal
    - **Property 2: Name Z–A is the exact reverse of Name A–Z**
    - **Validates: Requirements 2.6**

  - [x]* 4.4 Write property test for sort permutation, idempotence, and determinism
    - **Property 3: Sorting is a deterministic, idempotent permutation**
    - Bias toward small lists, plus one 10,000-row run. Assert the input is not mutated
    - **Validates: Requirements 2.12, 2.13**

  - [x]* 4.5 Write property test for filter conjunction
    - **Property 4: Filters are a conjunction of the glossary predicates**
    - Use an independent `oracle` written from the Glossary definitions
    - **Validates: Requirements 3.3, 3.4, 3.7**

  - [x]* 4.6 Write property test for filter toggling
    - **Property 5: Toggling flips exactly one filter**
    - **Validates: Requirements 3.2**

  - [x] 4.7 Add search composition and empty-state selection
    - Implement `deriveCompanyList` as `filterCompaniesBySearch` (from `job-filters.ts`, unchanged) → `applyCompanyFilters` → `sortCompanies`, returning `visible`, `total`, and `isRefined`
    - Implement `companyEmptyState` with precedence `loading` → `error` → `noCompanies` → `null` → `noSearchMatch` → `noFilterMatch`
    - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.9, 5.10, 7.1, 7.2, 7.4, 7.6_

  - [x]* 4.8 Write property test for filter activation order
    - **Property 6: Filter activation order does not matter**
    - **Validates: Requirements 3.8**

  - [x]* 4.9 Write property test for the derived list model
    - **Property 7: Derived list matches the search + filter + sort model**
    - Include empty, whitespace-only, and mixed-case search strings
    - **Validates: Requirements 5.1, 5.2, 5.3, 5.9, 5.10**

  - [x]* 4.10 Write property test for search/filter commutativity
    - **Property 8: Search and filters commute**
    - **Validates: Requirements 5.4**

  - [x]* 4.11 Write property test for empty-state precedence
    - **Property 11: Empty-state selection follows the precedence rule**
    - **Validates: Requirements 7.1, 7.2, 7.4, 7.6**

  - [x] 4.12 Add view-state persistence codec and storage wrappers
    - Implement `COMPANY_VIEW_STORAGE_KEY = "jobTracker.companiesSidebar.view.v1"`
    - Implement `serializeCompanyViewState` writing `{"version":1,"sort":…,"filters":[…]}` with filters in `COMPANY_FILTERS` order
    - Implement `parseCompanyViewState` per the parsing-rules table (never throws, invalid sort → `nameAsc`, keep known de-duplicated filters, tolerate any `version`)
    - Implement `loadCompanyViewState` and `saveCompanyViewState` so storage errors never throw (load → defaults, save → `false`)
    - _Requirements: 2.10, 6.3, 6.4, 6.5, 6.6_

  - [x]* 4.13 Write property test for the persistence round trip
    - **Property 9: View state persistence round trip**
    - **Validates: Requirements 6.4**

  - [x]* 4.14 Write property test for sanitizing stored state
    - **Property 10: Parsing stored state never fails and sanitizes**
    - Generate arbitrary strings, `null`, and JSON objects with arbitrary `sort` and `filters` values
    - **Validates: Requirements 6.5, 6.3, 2.10**

  - [x]* 4.15 Write unit tests for module examples
    - "Abby Care" < "AKASA" < "beta" under `nameAsc`; `parseCompanyViewState(null)` defaults; `sortCountFor` for all five options; load/save with a throwing storage; `clearCompanyFilters` keeps sort
    - _Requirements: 2.5, 2.10, 2.14, 3.9, 6.3, 6.6_

- [x] 5. Checkpoint - Ensure all tests pass
  - Run `npm test` and `npm run build` in `desktop/`. Ensure all tests pass, ask the user if questions arise.

- [x] 6. Build the `CompaniesSidebar` component
  - [x] 6.1 Extract `SidebarLink` into a shared export
    - Move `SidebarLink` (with its `count` prop) out of `desktop/src/pages/JobsPage.tsx` into a shared component file and import it from JobsPage, with no behavior change
    - _Requirements: 2.14, 5.11_

  - [x] 6.2 Create `desktop/src/components/CompaniesSidebar.tsx` with search, list, count, and empty states
    - Props: `rows`, `load`, `activeCompanyId`, `hrefForCompany`
    - State: in-memory `search` (starts `""`, never persisted), `view` from a lazy `useState(() => loadCompanyViewState(safeLocalStorage()))`, `openMenu: "sort" | "filter" | null`
    - `derived = useMemo(() => deriveCompanyList(rows, search, view), …)`
    - Add a `commitView` helper that calls `setView` and `saveCompanyViewState` synchronously in the handler, ignoring write failures
    - Render the search input with `aria-label="Search companies"` and the current "Search companies…" placeholder, in a `flex items-stretch gap-1.5` row with `min-w-0 flex-1` on the input. Render the controls only when `load === "ready"` and rows exist
    - Render visible rows with `SidebarLink`, using `sortCountFor` for the count and `activeCompanyId` for the active mark
    - Render `"N of M companies"` (`aria-live="polite"`) when `isRefined`
    - Render empty states from `companyEmptyState`: nothing while loading, `role="alert"` "Couldn't load companies." with no controls or links on error, "No companies yet." with no Reset, and "No matching companies." / "No companies match the selected filters." with a Reset button
    - Reset clears search, commits `clearCompanyFilters(view)`, keeps sort, and focuses the search input
    - Never read or write URL search params
    - _Requirements: 1.1, 1.9, 2.14, 2.15, 3.11, 5.1, 5.5, 5.6, 5.7, 5.8, 5.9, 5.10, 5.11, 5.12, 6.1, 6.2, 6.6, 6.7, 7.1, 7.2, 7.3, 7.4, 7.5, 7.6, 7.7_

  - [x] 6.3 Add the Sort_Button and Sort_Menu
    - Square `shrink-0` button with no fixed height, `aria-label="Sort companies"`, `aria-haspopup="menu"`, `aria-expanded`, `aria-controls`, and `aria-describedby` pointing to a visually hidden "Sorted by {label}"
    - Show an accent dot only when `view.sort !== "nameAsc"`
    - `role="menu"` with five `menuitemradio` items in `COMPANY_SORT_OPTIONS` order, `aria-checked`, and a check icon on the active one. Focus the active item on open
    - Selecting commits the sort only when it changed, then closes the menu and focuses the Sort_Button. Activating the button toggles its menu, or switches over from the filter menu
    - _Requirements: 1.2, 1.3, 1.7, 1.10, 1.11, 2.1, 2.2, 2.3, 2.4, 2.11_

  - [x] 6.4 Add the Filter_Button and Filter_Menu
    - `aria-label="Filter companies"`, same ARIA wiring as Sort_Button, a numeric badge and hidden "{n} filters active" description only when filters are active
    - `role="menu"` with four `menuitemcheckbox` items with `aria-checked` and a checkbox glyph. Focus the first item on open
    - Toggling commits `toggleCompanyFilter` and keeps the menu open with focus on the item
    - Add a separator and a "Clear filters" `menuitem` last, `aria-disabled` and a no-op when no filters are active, still focusable
    - _Requirements: 1.2, 1.4, 1.7, 1.10, 1.11, 3.1, 3.2, 3.5, 3.6, 3.9, 3.10_

  - [x] 6.5 Add shared menu dismissal and keyboard handling
    - One `useEffect` keyed on `openMenu`, modeled on the jobs Filter_Popover in `JobsPage.tsx`: outside `pointerdown` (outside menu and its button) closes without changing view state; Escape closes, refocuses the opening button, and calls `preventDefault` + `stopPropagation`
    - ArrowUp/ArrowDown move focus among menu items. Keep items `tabIndex=0` so Tab/Shift+Tab work natively
    - _Requirements: 1.5, 1.6, 1.8_

  - [x]* 6.6 Write component tests for menus, focus, and ARIA
    - Create `desktop/src/components/CompaniesSidebar.test.tsx` with `// @vitest-environment jsdom`
    - Cover: DOM order of search, Sort_Button, Filter_Button in one `items-stretch` row; open/focus for both menus; Escape refocus; outside click keeps state; re-click closes; switching leaves one menu open; ARIA names, `aria-expanded`, `aria-haspopup`, exactly one checked sort item for each option; Tab reach and Enter/Space activation; sort select closes and refocuses; re-selecting changes nothing; sort dot and description; filter badge; toggle keeps menu open and focus; "Clear filters" position, disabled state, and keeping sort and search
    - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7, 1.8, 1.10, 1.11, 2.1, 2.2, 2.3, 2.4, 2.11, 3.1, 3.2, 3.5, 3.6, 3.9, 3.10_

  - [x]* 6.7 Write component tests for persistence, search, list, and empty states
    - Cover: `setItem` called inside the click handler; pre-seeded storage applied on first render; search text never written and empty on mount; search edits keep view state and view changes keep search (including whitespace); count line shown and hidden; active company marking; zero companies hides controls with "No companies yet." and no Reset; both no-match messages with Reset; Reset clears search and filters, keeps sort, writes storage, and focuses search; loading shows no message; error shows `role="alert"` and no links
    - _Requirements: 1.9, 5.5, 5.6, 5.9, 5.10, 5.11, 6.1, 6.2, 6.7, 7.1, 7.2, 7.3, 7.4, 7.5, 7.6, 7.7_

- [x] 7. Wire `CompaniesSidebar` into the Jobs page
  - [x] 7.1 Replace the inline sidebar in `desktop/src/pages/JobsPage.tsx`
    - Replace `companies: {id,name}[]` with `companyRows: CompanyRow[]` and `companiesLoad` (initial `"loading"`)
    - Fetch with `Promise.allSettled([api.getJobsDashboard(), api.listCompanies()])`. On companies failure set `companyRows = []` and `companiesLoad = "error"`. Keep the dashboard branch's existing `setCounts` / `setActivity` / `setError` / `reportRefreshFailed` behavior
    - Remove `companySidebarSearch` and `visibleCompanies`. Render `<CompaniesSidebar rows={companyRows} load={companiesLoad} activeCompanyId={companyId} hrefForCompany={(id) => withCurrentSort(`/?companyId=${id}`)} />` inside the existing card, under the unchanged heading and Manage link
    - _Requirements: 2.15, 3.11, 4.5, 5.7, 5.8, 5.12, 7.6, 7.7_

  - [x]* 7.2 Write JobsPage integration tests for the companies sidebar
    - Extend `desktop/src/pages/JobsPage.filters.test.tsx` or add `JobsPage.companies.test.tsx` (jsdom)
    - Cover: sort, filter, search, and restore-on-mount keep every URL param including `companyId`; clicking a company sets `companyId` and keeps view state and search; a hidden active company keeps `companyId` and the scoped jobs list and marks nothing; Reset keeps URL params; `listCompanies` rejection shows the error state and leaves storage unchanged while the dashboard still renders; loading shows no empty message; a reload with new `trackedJobCount` values re-applies the same sort and filters
    - _Requirements: 2.15, 3.11, 4.5, 5.7, 5.8, 5.12, 7.3, 7.6, 7.7_

- [x] 8. Final checkpoint - Ensure all tests pass
  - Run `cargo test` in `src-tauri/`, and `npm test` plus `npm run build` in `desktop/`. Ensure all tests pass, ask the user if questions arise.
  - Manual visual check (user runs `npm run dev` from the repo root): at the narrowest sidebar width the search/sort/filter row does not wrap and the buttons are no taller than the input (Req 1.1, 1.2). Note whether the "Search companies…" placeholder truncation at about 116px reads poorly. The placeholder stays as is, so report the result to the user instead of changing it. VoiceOver announces checked states for menu items.

## Notes

- Tasks marked with `*` are optional and can be skipped for a faster MVP
- Each task references specific requirements for traceability
- Property tests use fast-check (`{ numRuns: 100 }`) in `desktop/` and proptest (`with_cases(100)`) in `src-tauri/`, each tagged `// Feature: companies-sidebar-sort-filter, Property N: <title>`
- `mockup.html` is design-only and gets no implementation task
- The placeholder truncation question is deliberately left open. Task 8 only checks it visually

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1", "2.1"] },
    { "id": 1, "tasks": ["1.2", "4.1", "6.1"] },
    { "id": 2, "tasks": ["1.3", "4.2"] },
    { "id": 3, "tasks": ["1.4", "4.3"] },
    { "id": 4, "tasks": ["4.4"] },
    { "id": 5, "tasks": ["4.5"] },
    { "id": 6, "tasks": ["4.6"] },
    { "id": 7, "tasks": ["4.7"] },
    { "id": 8, "tasks": ["4.8"] },
    { "id": 9, "tasks": ["4.9"] },
    { "id": 10, "tasks": ["4.10"] },
    { "id": 11, "tasks": ["4.11"] },
    { "id": 12, "tasks": ["4.12"] },
    { "id": 13, "tasks": ["4.13"] },
    { "id": 14, "tasks": ["4.14"] },
    { "id": 15, "tasks": ["4.15", "6.2"] },
    { "id": 16, "tasks": ["6.3"] },
    { "id": 17, "tasks": ["6.4"] },
    { "id": 18, "tasks": ["6.5"] },
    { "id": 19, "tasks": ["6.6", "7.1"] },
    { "id": 20, "tasks": ["6.7", "7.2"] }
  ]
}
```
