# Implementation Plan

- [x] 1. Write bug condition exploration test
  - **Property 1: Bug Condition** - Title block keeps its natural width when the header row is crowded
  - **CRITICAL**: This test MUST FAIL on unfixed code. The failure confirms the bug exists
  - **DO NOT attempt to fix the test or the code when it fails**
  - **NOTE**: This test encodes the expected behavior. It will validate the fix when it passes after implementation
  - **GOAL**: Surface counterexamples showing that no header variant carries the layout classes that stop the title from collapsing
  - Create `desktop/src/pages/JobsPage.header.test.tsx` following `JobsPage.filters.test.tsx`: `// @vitest-environment jsdom`, `vi.hoisted` mocks for `@/lib/api`, mocked `@/lib/RunMonitorContext`, `MemoryRouter` + `createRoot` + `act`, and a `Location` helper for `location.search`
  - Locate elements without test ids: title block = `h1.parentElement.parentElement`; header container = title block's parent; toolbar = the `+ Add job` link's parent
  - jsdom has no layout, so the property guards the className contract that produces the fix (see Bug Condition and Testing Strategy in design). `isBugCondition` (md+ row with `titleMax + 16px + toolbarMax > headerWidth`) is checked in a real browser in task 3.4
  - **Scoped PBT Approach**: Use `fast-check` (`numRuns` ≈ 25) to generate `{ filter: all | favorites | archived | status(s), jobCount: 0..120, hasNextCursor, sortField, isAppliedTab }`, mock `listJobsPage` to match, render, and assert for every variant:
    - header container classes include `flex`, `flex-col`, `gap-4`, `md:flex-row`, `md:flex-wrap`, `md:items-end`, `md:justify-between`
    - title block classes include `max-w-full` and `md:shrink-0`
  - Run `npx vitest run src/pages/JobsPage.header.test.tsx` in `desktop/` on UNFIXED code
  - **EXPECTED OUTCOME**: Test FAILS (`md:flex-wrap` missing on the container; title block has no `className`)
  - Document the counterexample fast-check reports (for example `filter: all, sortField: postingDate`)
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 2.1, 2.2, 2.3, 2.4_

- [x] 2. Write preservation property tests (BEFORE implementing fix)
  - **Property 2: Preservation** - Uncrowded layouts and all header behavior are unchanged
  - **IMPORTANT**: Follow observation-first methodology. Render the UNFIXED page and record actual strings, classes, and URL changes before writing assertions
  - Add to `desktop/src/pages/JobsPage.header.test.tsx`, reusing the generator from task 1:
    - Heading/subtitle strings equal an oracle copied from today's expressions (`"All active jobs"`, `"Favorites Board"`, `"Archived Postings"`, `jobStatusPresentation(s).label`; count, empty, singular/plural, and `100+` subtitle branches) (3.8)
    - Star badge renders for favorites, archive badge for archived, none otherwise (3.7)
    - Toolbar class string equals exactly `flex flex-wrap items-center gap-2 self-start md:self-auto`; `h1` keeps `font-display text-3xl font-semibold tracking-tight`; subtitle keeps `mt-1 text-sm text-[var(--muted)]` (3.1, 3.2, 3.9)
    - Generated sequences of Sort by / Order selections and List/Board clicks produce the same `location.search` and active-toggle styling observed on unfixed code; "Date applied" option only on the applied tab (3.3, 3.5)
    - `.jobs-sort-hint` text "Based on when the job was added to the tracker." renders and is referenced by the Sort by select's `aria-describedby` if and only if the field is `postingDate` (3.4)
    - `+ Add job` link has `href="/jobs/new"` (3.6)
  - Run on UNFIXED code
  - **EXPECTED OUTCOME**: Tests PASS (baseline behavior to preserve)
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 3.8, 3.9_

- [x] 3. Fix the Jobs page header title collapse

  - [x] 3.1 Implement the fix
    - In `desktop/src/pages/JobsPage.tsx` (~line 971), change the header container className to `flex flex-col gap-4 md:flex-row md:flex-wrap md:items-end md:justify-between`
    - Change the title block's bare `<div>` to `<div className="max-w-full md:shrink-0">`
    - Leave the toolbar className, `index.css`, strings, and handlers unchanged. Do not add a `clamp()` heading size
    - _Bug_Condition: isBugCondition(input) where viewportWidth >= 768px AND titleMax + 16px + toolbarMax > headerWidth_
    - _Expected_Behavior: title block width == min(titleMax, headerWidth); toolbar moves to a line below (Property 1)_
    - _Preservation: Preservation Requirements from design (wide row and stacked layouts identical; all header controls and strings unchanged)_
    - _Requirements: 2.1, 2.2, 2.3, 2.4, 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 3.8, 3.9_

  - [x] 3.2 Verify bug condition exploration test now passes
    - **Property 1: Expected Behavior** - Title block keeps its natural width when the header row is crowded
    - **IMPORTANT**: Re-run the SAME test from task 1. Do NOT write a new test
    - **EXPECTED OUTCOME**: Test PASSES (className contract holds for every variant)
    - _Requirements: 2.1, 2.2, 2.3, 2.4_

  - [x] 3.3 Verify preservation tests still pass
    - **Property 2: Preservation** - Uncrowded layouts and all header behavior are unchanged
    - **IMPORTANT**: Re-run the SAME tests from task 2. Do NOT write new tests
    - **EXPECTED OUTCOME**: Tests PASS (no regressions)
    - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 3.8, 3.9_

  - [x] 3.4 Manual real-browser resize check (user-run)
    - The agent does not start the dev server. Ask the user to run `npm run dev` in `desktop/` (or the Tauri dev app) and resize the window from about 375px to 1920px
    - Confirm: "All active jobs" and "2 roles on your radar." each stay on one line; the toolbar wraps below the title when crowded; no control is clipped or overlapping; at ≥1440px and below 768px the header looks as before
    - _Requirements: 2.1, 2.2, 2.3, 2.4, 3.1, 3.2, 3.9_

- [x] 4. Checkpoint - Ensure all tests pass
  - Run the desktop test suite once: `npm test` in `desktop/` (`vitest run`)
  - Ensure all tests pass, ask the user if questions arise
