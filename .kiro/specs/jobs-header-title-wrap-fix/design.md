# Jobs Header Title Wrap Bugfix Design

## Overview

The Jobs page header (`desktop/src/pages/JobsPage.tsx`, around line 971) is a flex container that switches from a column to a single non-wrapping row at the `md` breakpoint (`flex flex-col gap-4 md:flex-row md:items-end md:justify-between`). It has two children:

- **Title block**: a bare `<div>` holding the optional favorites/archived icon badge, the `h1` (`font-display text-3xl`), and the subtitle `p` (`text-sm`).
- **Toolbar**: `flex flex-wrap items-center gap-2 self-start md:self-auto`, holding the Sort by control (`.jobs-sort-control`, select `min-width: 12rem`), the Order control (`.jobs-sort-control--compact`, select `min-width: 9.5rem`), the conditional `.jobs-sort-hint`, the List/Board toggle, and the `+ Add job` link.

The row cannot wrap, and both children use the flex default `flex: 0 1 auto`. When the two max-content widths add up to more than the row width, the browser shrinks both items in proportion to their base sizes. The toolbar's base size (roughly 60rem with the sort hint visible) is several times the title's, so the toolbar keeps most of the row. The title shrinks until it reaches its automatic minimum width (`min-width: auto`, which equals min-content, the longest word). The heading then renders one word per line and the subtitle wraps.

The fix is a className-only change in `JobsPage.tsx`:

1. Let the `md+` row wrap (`md:flex-wrap`), so the toolbar moves to its own line when it does not fit beside the title.
2. Stop the title block from shrinking (`md:shrink-0`) and cap it at the header width (`max-w-full`), so it stays at its natural width and wraps text only when the title alone is wider than the header.

When both children fit on one line, the flex layout is the same as today: same items, same `justify-between`, same `items-end`, same `gap-4`. The wide layout stays pixel-identical. No CSS in `index.css` changes, no strings change, and no behavior changes.

## Glossary

- **Bug_Condition (C)**: The header is in its `md+` row layout and the title block's max-content width plus `gap-4` plus the toolbar's max-content width is greater than the header's content width. Under the current code the title block then collapses toward min-content.
- **Property (P)**: For any header width and any heading/subtitle variant, the title block's rendered width equals `min(titleMaxContent, headerWidth)`. A heading wraps only when it cannot fit on one line at the full header width.
- **Preservation**: Every header width where C is false (wide row layout and the `< md` stacked layout), plus all non-layout behavior: sort controls, URL params, sort hint and `aria-describedby`, view toggle, Add job navigation, icon badges, and heading/subtitle strings.
- **Header container**: The `div` wrapping the title block and toolbar (`JobsPage.tsx` ~line 971).
- **Title block**: The first child of the header container (icon badge row with `h1`, plus subtitle `p`).
- **Toolbar**: The second child of the header container (sort controls, sort hint, view toggle, Add job link).
- **`heading` / `subtitle`**: Strings computed in `JobsPage` (~lines 800–823). Variants: `"All active jobs"`, `"Favorites Board"`, `"Archived Postings"`, a `jobStatusPresentation(status).label` such as `"Interviewing"`. Subtitles include counts, empty states, and the `100+` forms.
- **max-content / min-content**: The CSS intrinsic widths. max-content is the width with no soft wraps. min-content is the width of the longest unbreakable run (one word here).
- **`md` breakpoint**: Tailwind v4 default, 48rem (768px) viewport width. The two-column page grid (`lg:grid-cols-[240px_1fr]`) starts at `lg` (64rem), so the header width drops by the 240px sidebar plus `gap-8` when the viewport crosses `lg`.

## Bug Details

### Bug Condition

The bug shows up only in the `md+` row layout, when the title block and toolbar cannot both sit at their natural widths on one line. Because the row uses `flex-wrap: nowrap` and both items shrink, the browser takes the overflow out of both items in proportion to their base sizes. The title block has no `flex-shrink: 0` and no explicit minimum width, so it shrinks to min-content. Its text then wraps word by word. The toolbar's own `flex-wrap` never takes effect, because the toolbar still gets the larger share of the row.

**Formal Specification:**
```
FUNCTION isBugCondition(input)
  INPUT: input of type HeaderRenderInput {
           viewportWidth: px,
           headerWidth: px,             // content box of the header container
           heading: string,
           subtitle: string,
           badge: none | favorite | archived,
           sortField: JobListSortField, // controls whether .jobs-sort-hint renders
           isAppliedTab: boolean
         }
  OUTPUT: boolean

  titleMax   := maxContentWidth(titleBlock(input.heading, input.subtitle, input.badge))
  toolbarMax := maxContentWidth(toolbar(input.sortField, input.isAppliedTab))
  GAP        := 16px                    // gap-4

  RETURN input.viewportWidth >= 768px                      // md row layout active
         AND titleMax + GAP + toolbarMax > input.headerWidth  // row is crowded
END FUNCTION
```

Under the unfixed code, when `isBugCondition` is true, the rendered title width is `max(minContentWidth(titleBlock), headerWidth - GAP - shrunkToolbarWidth)`. This value is often close to min-content, so a three-word heading stacks one word per line.

### Examples

- **"All active jobs", sort = Date added, viewport ≈ 1280px (sidebar visible)**: The header is about 900–950px wide. The toolbar max-content with the hint is about 950px. Actual: the heading renders "All" / "active" / "jobs", and "2 roles on your radar." wraps onto two lines. Expected: the toolbar drops to a second line, and the heading and subtitle each stay on one line.
- **"Favorites Board" with star badge, viewport ≈ 900px (no sidebar)**: Actual: "Favorites" / "Board" stack beside the badge, and "3 priority roles on your favorite board." wraps into a narrow column. Expected: the title block stays at its natural width and the toolbar wraps below it.
- **Status "Interviewing", sort = Appeal (no hint)**: The toolbar is narrower without the hint, so the crowded range starts at a smaller width. When it does occur, the subtitle "Showing 100+ roles on your radar." wraps per word. Expected: the subtitle stays on one line, and the toolbar moves below.
- **Edge case: header narrower than the title's own max-content (for example a long status label near 768px)**: Expected: the title block takes the full header width (`max-w-full`), and the text wraps at normal word boundaries for that width, never at min-content. The toolbar sits below and wraps its own controls.
- **Non-bug: viewport ≈ 1600px**: Both blocks fit on one row. Expected and actual: title on the left, toolbar on the right, bottom-aligned, as today.

## Expected Behavior

### Preservation Requirements

**Unchanged Behaviors:**
- Wide widths where the title and full toolbar fit on one row: title left, toolbar right, `items-end` alignment, the same `gap-4` minimum spacing, and the same heading font size (`text-3xl`). (3.1)
- Below `md`: a stacked column with the title above the toolbar and the toolbar left-aligned (`self-start`). (3.2)
- Sort by / Order selects: same options (including "Date applied" only on the applied tab), same `setSortField` / `setSortDirection` handlers, and the same URL parameter updates. (3.3)
- The `.jobs-sort-hint` text "Based on when the job was added to the tracker." renders only when `sortSpec.field === "postingDate"`, and it is referenced by the Sort by select's `aria-describedby` only in that case. (3.4)
- The List/Board toggle switches `viewMode` and highlights the active option. (3.5)
- The `+ Add job` link goes to `/jobs/new`. (3.6)
- The star badge shows for favorites, and the archive badge shows for archived. (3.7)
- The `heading` and `subtitle` strings are produced by the same expressions. (3.8)
- No toolbar control is clipped, overlapping, or hidden at any width. Each control keeps its current minimum widths from `index.css`. (3.9)

**Scope:**
Inputs where `isBugCondition` is false must render identically. This covers:
- Any viewport below 768px (stacked layout).
- Any `md+` width where `titleMax + 16px + toolbarMax <= headerWidth`.
- All user interactions with the header controls, at every width.
- The DOM structure, accessible names, ids, and text content of the header.

## Hypothesized Root Cause

1. **Non-wrapping row with shrinkable children (primary)**: `md:flex-row` without `md:flex-wrap` forces both children onto one line. When the line overflows, both children shrink. Shrinking is weighted by `flex-shrink × flex-basis`, and the toolbar's base size is far larger, so the browser takes the space from the title. This turns a "not enough room" situation into "squeeze the title".
2. **Title block has no shrink protection**: The title `div` has no `shrink-0`, `min-w-*`, or `whitespace-nowrap`. Its automatic minimum is min-content (the longest word), so nothing stops the word-per-line collapse.
3. **Toolbar `flex-wrap` never applies**: The toolbar already has `flex flex-wrap`, but its items only wrap once the toolbar itself is narrower than their combined width. Because the toolbar wins the shrink competition, it stays wide, and the title absorbs the loss instead.
4. **Large, variable toolbar width**: The Sort by select's `min-width: 12rem`, the Order select's `9.5rem`, and the optional sort hint (~17rem) add up to a toolbar of about 60rem. That is wider than the header across most of the `md`–`xl` range, especially once the 240px sidebar appears at `lg`. The hint's presence (sort field) moves the crowded range, which is why the bug seems to come and go.

This hypothesis is confirmed if, in a real WebView, adding only `md:flex-wrap` + `md:shrink-0` stops the collapse at every width (see Exploratory Bug Condition Checking).

## Correctness Properties

Property 1: Bug Condition - Title block keeps its natural width when the header row is crowded

_For any_ header render input where the bug condition holds (`md+` row layout and `titleMax + gap + toolbarMax > headerWidth`), across all heading variants (all active jobs, favorites, archived, every job status label) and all subtitle variants (counts, empty states, 100+), the fixed header SHALL render the title block at `min(titleMax, headerWidth)` wide. A short heading such as "All active jobs" and a short subtitle such as "2 roles on your radar." SHALL each render on a single line. Longer text SHALL wrap only at the full header width, never at min-content. The toolbar SHALL move to a line below the title and wrap its own controls as needed.

**Validates: Requirements 2.1, 2.2, 2.3, 2.4**

Property 2: Preservation - Uncrowded layouts and all header behavior are unchanged

_For any_ header render input where the bug condition does NOT hold (viewport below `md`, or `md+` with room for both blocks on one row), the fixed header SHALL produce the same layout as the original: stacked title above a left-aligned toolbar below `md`, and title left / toolbar right with bottom alignment and the same gap on wide rows. For every input, the fixed header SHALL keep the same heading and subtitle strings, icon badges, sort select options and handlers, sort-hint visibility and `aria-describedby` wiring, List/Board toggle behavior, and `+ Add job` navigation. No toolbar control SHALL be clipped or overlapping.

**Validates: Requirements 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 3.8, 3.9**

## Fix Implementation

### Changes Required

Assuming the root cause analysis is correct:

**File**: `desktop/src/pages/JobsPage.tsx`

**Component**: `JobsPage` header markup (~lines 971–1056)

**Specific Changes**:
1. **Allow the md+ row to wrap**: Change the header container from
   `flex flex-col gap-4 md:flex-row md:items-end md:justify-between`
   to
   `flex flex-col gap-4 md:flex-row md:flex-wrap md:items-end md:justify-between`.
   - When both children fit, this is a no-op: one flex line with the same justification and alignment.
   - When they do not fit, the toolbar breaks onto a second line. Line breaking uses each item's max-content base size, so the toolbar moves down before anything shrinks. `gap-4` becomes the row gap between lines, which matches the stacked `< md` spacing.
   - With a single item on a line, `justify-between` places the toolbar at the start. The wrapped toolbar is left-aligned under the title, the same as the `< md` stacked layout.

2. **Prevent the title block from shrinking**: Change the title block's bare `<div>` to `<div className="max-w-full md:shrink-0">`.
   - `md:shrink-0` keeps the title at max-content, so it never collapses toward min-content in the row layout.
   - `max-w-full` caps the title at the header width. When the header is narrower than the title's max-content (edge case), text wraps at normal word boundaries for the full width instead of overflowing.
   - Below `md` the column layout already gives the title the full width. `max-w-full` is harmless there.

3. **Leave the toolbar classes unchanged**: `flex flex-wrap items-center gap-2 self-start md:self-auto` stays as is. In the wrapped state, the toolbar's default `flex: 0 1 auto` shrinks it to the header width, and its existing `flex-wrap` then wraps controls onto more rows. Each control keeps its `min-width`, so nothing clips (3.9). `md:self-auto` keeps `items-end` alignment on the shared row.

4. **No CSS file changes**: `desktop/src/index.css` (`.jobs-sort-*`, lines ~137–184) is unchanged.

5. **No fluid heading size (deliberately out of scope)**: A `clamp()` font size for the `h1` was considered. It would change the heading size at some widths that are not crowded today, which conflicts with 3.1 (wide layout identical). Changes 1 and 2 already stop the word-per-line collapse at every width. A fluid size can be a separate follow-up if wanted.

**Alternatives considered:**
- *Toolbar `md:flex-1 md:min-w-0 md:justify-end`*: With `flex-basis: 0`, the toolbar would never move to its own line. It would stay beside the title and wrap controls into a ragged right-aligned column, and `min-w-0` makes clipping possible. This was rejected because it behaves worse in the crowded range and puts 3.9 at risk.
- *`whitespace-nowrap` on the `h1`*: This would keep short headings on one line, but long status labels could overflow on narrow headers (2.4) and the subtitle would still collapse. It was rejected.
- *Fixed `min-w-[Nrem]` on the title*: Any fixed value is either too small for long headings or too big for short ones. It was rejected in favor of the content-based `shrink-0` + `max-w-full`.
- *Wrapped toolbar right-aligned (`md:ml-auto`)*: This keeps the toolbar on the right edge after wrapping. Left-aligned was chosen to match the stacked layout and normal reading order. Adding `md:ml-auto` later is a one-class change if preferred.

## Testing Strategy

### Validation Approach

jsdom (used by the existing `desktop/` tests via `// @vitest-environment jsdom`, Vitest 4, `fast-check` 4.10.2, `react-dom/client` + `act`, no Testing Library) does not run layout. Testing is therefore split by what each environment can show:

- **Real layout (WebView/Chromium)**: confirms the bug condition and the fix by measuring rendered widths and line counts. This is the source of truth for Property 1 and for the layout half of Property 2.
- **jsdom unit + property tests**: lock in the className contract that produces the layout (`md:flex-wrap`, `md:shrink-0`, `max-w-full`, unchanged toolbar classes) and all non-layout preservation behavior. They run in `npm test` (`vitest run`) and catch regressions in CI.

### Exploratory Bug Condition Checking

**Goal**: Surface counterexamples that demonstrate the bug BEFORE implementing the fix, and confirm or refute the root cause analysis.

**Test Plan**: Run the unfixed UI in a real browser engine. The user runs `npm run dev` in `desktop/` manually, or uses the gstack browse tooling already used for QA under `.gstack/qa-reports`. Resize the viewport from 768px to 1600px in 16px steps for each scenario below. At each width, record:
- `h1` line count = `Math.round(h1.getBoundingClientRect().height / parseFloat(getComputedStyle(h1).lineHeight))`
- subtitle line count (same formula)
- title block width vs. its max-content width (measure a `width: max-content` clone)
- whether the toolbar's top is below the title block's bottom

**Test Cases**:
1. **All active jobs + Date added hint, sidebar visible (≥1024px)**: Expect `h1` line count of 3 across part of the 1024–1400px range (will fail on unfixed code).
2. **Favorites Board with badge, 768–1023px**: Expect stacked words beside the badge (will fail on unfixed code).
3. **Archived Postings, 100+ subtitle**: Expect the subtitle to wrap per word or two (will fail on unfixed code).
4. **Status label + Appeal sort (no hint)**: The crowded range shifts narrower. Expect collapse within that range (may fail on unfixed code, depending on width).
5. **Root-cause probe**: In DevTools, add only `md:flex-wrap` + `md:shrink-0` to the unfixed DOM and repeat case 1. Expect line count 1. If it is not 1, re-hypothesize.

**Expected Counterexamples**:
- At some `md+` width, the title block width equals its min-content width and `h1` line count > 1 for "All active jobs".
- Possible causes: the non-wrapping flex row, proportional shrink favoring the toolbar, and no title min-width.

### Fix Checking

**Goal**: Verify that for all inputs where the bug condition holds, the fixed header produces the expected behavior.

**Pseudocode:**
```
FOR ALL input WHERE isBugCondition(input) DO
  render fixed header at input.viewportWidth with input.heading/subtitle/badge/sortField
  titleMax := maxContentWidth(titleBlock)
  ASSERT width(titleBlock) == min(titleMax, headerWidth)       // ±1px
  IF titleMax <= headerWidth THEN
    ASSERT lineCount(h1) == 1
    ASSERT lineCount(subtitle) == 1
  END IF
  ASSERT top(toolbar) >= bottom(titleBlock)                    // toolbar moved below
  ASSERT no toolbar control overflows header or overlaps another control
END FOR
```

This runs in the real-browser sweep (same scenarios and widths as the exploratory checks, on fixed code).

### Preservation Checking

**Goal**: Verify that for all inputs where the bug condition does NOT hold, the fixed header produces the same result as the original.

**Pseudocode:**
```
FOR ALL input WHERE NOT isBugCondition(input) DO
  ASSERT layoutBoxes(originalHeader, input) == layoutBoxes(fixedHeader, input)   // real browser
END FOR

FOR ALL input DO   // jsdom
  ASSERT headerText(original, input) == headerText(fixed, input)
  ASSERT controlsBehavior(original, input) == controlsBehavior(fixed, input)
END FOR
```

**Testing Approach**: Property-based testing is recommended for preservation checking because:
- It generates many test cases automatically across the input domain (filters, statuses, job counts, sort fields, applied tab).
- It catches edge cases that manual unit tests might miss (empty lists, singular vs. plural counts, the `100+` cursor case).
- It gives strong evidence that behavior is unchanged for all non-buggy inputs.

**Test Plan**: Before the fix, run the jsdom preservation tests on the unfixed code and record the heading/subtitle strings and control behavior. They must pass unchanged after the fix. For real-layout preservation, capture title/toolbar bounding boxes at 375px, 600px, and wide widths (1440px, 1600px, 1920px) on unfixed code, then compare after the fix.

**Test Cases**:
1. **Wide row preservation**: At ≥1440px with every heading variant, the title and toolbar boxes match the unfixed code (same `x`, `y`, `width`, `height` ±1px).
2. **Stacked preservation**: Below 768px, the title is above the toolbar, the toolbar is left-aligned, and the boxes match the unfixed code.
3. **Sort controls preservation**: Changing Sort by / Order updates `location.search` exactly as before. "Date applied" appears only on the applied tab.
4. **Sort hint preservation**: The hint renders and is referenced by `aria-describedby` only when the sort field is `postingDate`.
5. **View toggle and Add job preservation**: The List/Board click updates the view and active styling. `+ Add job` has `href="/jobs/new"`.
6. **Badge and text preservation**: The star/archive badge appears for favorites/archived. Heading/subtitle strings match the existing expressions.

### Unit Tests

New file `desktop/src/pages/JobsPage.header.test.tsx`, following `JobsPage.filters.test.tsx`: `// @vitest-environment jsdom`, `vi.hoisted` API mocks for `@/lib/api`, a mocked `@/lib/RunMonitorContext`, `MemoryRouter` + `createRoot` + `act`, and a `Location` helper to read `location.search`. Locate elements without adding test ids: header container = the `h1`'s title block's parent. Title block = `h1.parentElement.parentElement`. Toolbar = the `+ Add job` link's parent.

- The header container's class list contains `flex`, `flex-col`, `gap-4`, `md:flex-row`, `md:flex-wrap`, `md:items-end`, and `md:justify-between`.
- The title block's class list contains `md:shrink-0` and `max-w-full`.
- The toolbar's class list equals exactly `flex flex-wrap items-center gap-2 self-start md:self-auto` (unchanged).
- The `h1` keeps `font-display text-3xl font-semibold tracking-tight`, and the subtitle `p` keeps `mt-1 text-sm text-[var(--muted)]`.
- Sort hint presence and `aria-describedby` for `postingDate` vs. other fields.
- The favorites/archived routes render the expected badge and headings "Favorites Board" / "Archived Postings".

### Property-Based Tests

Use `fast-check` (already a devDependency) inside the jsdom test file, with a modest `numRuns` (for example 25) because each run mounts the page.

- **Header class contract across variants (Property 1 guard)**: Generate `{ filter: all | favorites | archived | status(s ∈ jobStatuses), jobCount: 0..120, hasNextCursor: boolean, sortField, isAppliedTab }`. Mock `listJobsPage` to match, render, and assert the `md:flex-wrap` / `md:shrink-0` / `max-w-full` contract holds for every variant. A variant must not be able to bypass the layout fix.
- **Heading/subtitle string preservation (Property 2)**: For the same generated inputs, assert the rendered `h1` and subtitle text equal an oracle that copies today's expressions (`"All active jobs"`, `"Favorites Board"`, `"Archived Postings"`, `jobStatusPresentation(s).label`, and the count / empty / `100+` subtitle branches, including singular vs. plural).
- **Toolbar behavior preservation (Property 2)**: Generate sequences of Sort by / Order selections and List/Board clicks. Assert `location.search` and the active-toggle styling match the unfixed behavior, and assert the hint appears and is linked via `aria-describedby` if and only if the field is `postingDate`.

### Integration Tests

Run these in a real browser engine. They are manual or scripted with the existing gstack browse QA flow. No new browser test dependency is added by this fix.

- **Resize sweep**: For each heading variant × {hint visible, hint hidden}, sweep the viewport 375px→1920px in 16px steps. Assert `h1` line count is 1 whenever `titleMax <= headerWidth`, the title width is never below `min(titleMax, headerWidth) - 1px`, and no toolbar control overflows the header or overlaps another control.
- **Breakpoint transitions**: Check the 767/768px (`md`) and 1023/1024px (`lg` sidebar appears) boundaries. The layout moves between stacked, wrapped, and single-row with no width where the heading collapses.
- **Wide-layout visual diff**: Compare screenshots at 1440px and 1920px before and after the fix for All active jobs, Favorites, Archived, and one status. Expect no pixel differences in the header region.
- **Tauri WebView spot check**: In the packaged or dev Tauri app, resize the window through the formerly crowded range to confirm WebKit (macOS WKWebView) matches the Chromium sweep.
