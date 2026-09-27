# Implementation Plan — Settings Page Redesign

All work is in `desktop/src/pages/SettingsPage.tsx` unless noted. The plan is
incremental: each step leaves the page building and behaving correctly.

- [ ] 1. Introduce per-card status state model
  - Add the `SaveStatus` union (`idle | saving | saved | error`).
  - Add `csvStatus`, `keywordsStatus`, `locationStatus` state; add `loadError` for
    page-level load failures.
  - Remove the shared `busy`, global `error`, and `locationSaved` state.
  - Keep `config`, `pendingPath`, `roleKeywords`, `locationCountry`, `locationCities`,
    `loading`.
  - _Requirements: 3.3, 4.1, 4.3, 4.4_

- [ ] 2. Rewire async handlers to per-card status
  - Update `choosePath`, `confirmPath`, `useDefault` to read/write `csvStatus`.
  - Update `handleKeywordsSubmit` to write `keywordsStatus` (saving → saved → idle).
  - Update `handleLocationSubmit` to write `locationStatus`; drop the ad-hoc
    `Saved!` flag in favor of the shared saved → idle timeout.
  - Ensure each handler clears its own prior error by setting `saving` first.
  - Preserve exact API calls and arguments (`configureCsv(path, mode)`, etc.).
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 4.3, 6.1, 6.2_

- [ ] 3. Add presentational helpers (in-file)
  - `SaveState({ status })` using `.save-state`, `.save-state-dot`, `.spinner`.
  - `SettingCard({ title, description, hint, error, footer, children })` using `.card`
    with an `h3`; renders card-scoped error via `role="alert"` + danger tokens.
  - `SettingsGroup({ title, description, children })` as a `<section aria-labelledby>`
    with `h2` and description.
  - `Hint({ children })` de-emphasized (`--faint` footnote or `<details>`).
  - _Requirements: 2.3, 3.1, 3.2, 4.2, 5.1, 5.2, 6.4_

- [ ] 4. Recompose the page into groups
  - Page title/subtitle (`h1`) unchanged in weight.
  - Group "Search filters" → Role Keywords card + Location Preferences card.
  - Group "Data & storage" → Jobs CSV location card.
  - Apply `space-y-8` between groups, `space-y-4` within a group; keep `max-w-3xl`.
  - Ensure heading order h1 → h2 → h3 with no skips.
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 2.1, 2.2_

- [ ] 5. Migrate Role Keywords card
  - Add an associated `<label>` for the keyword input (currently placeholder-only).
  - Route submit through `keywordsStatus`; render `SaveState` next to the button.
  - Keep the concise description; move any extended guidance into `Hint`.
  - _Requirements: 3.1, 3.3, 3.4, 5.3, 6.4_

- [ ] 6. Migrate Location Preferences card
  - Keep country `<select>` and cities input with their labels.
  - Move the "smart cities" note into a `Hint` (collapsible or `--faint` footnote);
    keep the input placeholder.
  - Route submit through `locationStatus`; render `SaveState`.
  - _Requirements: 3.2, 3.3, 3.4, 5.1, 5.2, 5.3_

- [ ] 7. Migrate Jobs CSV location card + modal
  - Render inside "Data & storage" group via `SettingCard`.
  - Drive its buttons/spinner and error from `csvStatus`; keep the custom/default
    pill and the path `code` block.
  - Move the cloud-sync footnote into `Hint`.
  - Leave the import/replace modal markup, roles, and semantics unchanged.
  - _Requirements: 3.5, 4.1, 4.2, 5.1, 6.2, 6.3_

- [ ] 8. Loading and page-level error states
  - Keep the loading indicator before content is ready.
  - Show `loadError` at page level (only for initial load failures).
  - _Requirements: 4.4, 6.5_

- [ ] 9. Verify
  - Run `npm run build`, `npm run lint`, and the test suite (vitest `--run`) in
    `desktop/`; fix any regressions (watch for heading-level snapshot changes).
  - Manual pass: save keywords (Saved state), save location, choose CSV (import +
    replace + confirm), use default, and force one error per card to confirm scoping.
  - _Requirements: 3.1–3.5, 4.1–4.4, 6.1–6.5_
