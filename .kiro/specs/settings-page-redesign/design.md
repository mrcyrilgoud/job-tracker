# Design — Settings Page Redesign

## Goal

Reorganize the existing Settings page for better findability and less clutter,
while preserving all behavior. This is a **presentation-layer refactor** of
`desktop/src/pages/SettingsPage.tsx`. No API, Rust command, or data-model change.

## Current state (baseline)

`SettingsPage.tsx` renders a `max-w-3xl` single column with `space-y-6` and four
peer elements of equal weight:

- **Jobs CSV location** card — path display, `Choose CSV…`, conditional
  `Use default location`, and a footnote. Triggers a modal on choose.
- **Role Keywords** card — one input + `Save` button. No success feedback.
- **Location Preferences** card — country `<select>` + cities input + `Save Locations`
  button with a transient `Saved!` label. Includes a verbose italic note.
- **Import/replace modal** — dialog for existing/new CSV paths.

State: single `error` string (global), single `busy` boolean (shared across all
actions), `loading`, plus the per-setting values. A shared `busy` means saving
keywords disables the CSV buttons too.

## Design overview

Introduce a two-tier structure:

```
Settings (page title + subtitle)
│
├─ Group: "Search filters"        ← what shows up in your feed
│   ├─ Card: Role Keywords
│   └─ Card: Location Preferences
│
└─ Group: "Data & storage"        ← where your data lives
    └─ Card: Jobs CSV location
        └─ (Import/replace modal, unchanged)
```

Groups give scannability; cards keep their identity. We keep the single column and
current max width (Req 1.4) — with only three settings, a nav rail would be
over-engineering. The improvement comes from grouping, hierarchy, consistent
feedback, and scoped errors.

## Layout & hierarchy

Three type levels, all using existing tokens:

| Level        | Element             | Style |
|--------------|---------------------|-------|
| Page title   | `h1` "Settings"     | `font-display text-3xl font-semibold` (unchanged) |
| Group header | `h2` group name     | `font-display text-lg font-semibold` + `--muted` description; a subtle divider/eyebrow distinguishes it from cards |
| Card header  | `h3` setting name   | `font-display text-base/lg font-semibold` inside `.card` |

Because card headings drop from `h2` to `h3` (group takes `h2`), the heading outline
stays correct for assistive tech (Req 6.4). A group is a `<section aria-labelledby>`
wrapping its cards with shared vertical spacing so membership is visually obvious
(Req 2.2).

Spacing: groups separated by `space-y-8`; cards within a group by `space-y-4`. This
tighter intra-group spacing reinforces grouping without borders.

## Component structure

Keep everything in `SettingsPage.tsx` but extract small presentational helpers in the
same file to remove repetition and normalize patterns. No new files required.

- `SettingsGroup({ title, description, children })` — renders the group `<section>`,
  heading, and description. Pure presentational.
- `SettingCard({ title, description, hint, error, footer, children })` — renders a
  `.card` with an `h3`, optional description, optional de-emphasized `hint`, the
  control(s) as `children`, an optional `.card-footer` region, and a card-scoped
  error slot.
- `SaveState({ status })` — renders the normalized save indicator using `.save-state`
  / `.save-state-dot` / `.spinner`, driven by a small status union.
- `Hint({ children })` — de-emphasized helper affordance (`text-xs text-[var(--faint)]`,
  or a `<details>`-based collapsible for the longer notes). Keeps content in the
  accessibility tree (Req 5.2).

These are internal to the file; if they prove reusable later they can move to
`components/`. Keeping them local now avoids premature abstraction.

## State model changes

Replace the single global `busy`/`error` with **per-card status**, so one card's
activity never disables another (fixes the shared-`busy` coupling) and errors land on
the right card (Req 4).

```ts
type SaveStatus =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "saved" }
  | { kind: "error"; message: string };
```

Per-card status state:
- `csvStatus`, `keywordsStatus`, `locationStatus` (each a `SaveStatus`).
- Keep `config`, `pendingPath`, `roleKeywords`, `locationCountry`, `locationCities`,
  `loading`.
- Drop the standalone `busy`, global `error`, and `locationSaved` boolean — folded into
  the per-card statuses. A page-level load error is allowed to remain page-level
  (Req 4.4) via `loadError`.

Each async handler sets its own card status: `saving` → `saved` (auto-reset via
timeout) or `error`. `saved` reverts to `idle` after a short delay, matching the
existing `Saved!` flash behavior but applied uniformly (Req 3.4).

## Interaction patterns

**Save (Keywords, Location):** button shows busy while `saving`; on success a
`SaveState` shows a transient "Saved" then returns to idle; on failure the card shows a
scoped error and the button re-enables (Req 3.1–3.4, 4.1–4.3).

**Jobs CSV location:** keeps its immediate-action flow and modal exactly as today
(Req 6.2–6.3). Its busy/error now read from `csvStatus` and render through the same
`SaveState`/error slot so it looks consistent with the others (Req 3.5). The modal
markup, roles, and the `import` / `replace` / confirm semantics are unchanged.

**Error clearing:** starting a new action on a card sets its status to `saving`,
implicitly clearing the prior error (Req 4.3).

## Reduced inline noise (Req 5)

- Location "smart cities" note → moved into a `Hint` (collapsible `<details>` labelled
  e.g. "How city matching works", or a `--faint` footnote). The input keeps its
  placeholder so it is usable without expanding (Req 5.3).
- CSV cloud-sync warning → `--faint` footnote via `Hint`, same as today's intent but
  visually lighter.
- Card descriptions trimmed to one concise line; extended detail lives in `Hint`.

## Accessibility

- Heading order: `h1` (page) → `h2` (group) → `h3` (card). No skipped levels.
- Each group `<section aria-labelledby>` its `h2`; each card `<section aria-labelledby>`
  its `h3` (Req 6.4).
- Card errors use `role="alert"` and danger tokens (Req 4.2).
- Form labels retained (`htmlFor` on country/cities); keyword input gets an associated
  label (currently only placeholder).
- Modal keeps `role="dialog"`, `aria-modal`, labelled title (Req 6.3).
- Respects `prefers-reduced-motion` via existing `.spinner` rule.

## Design tokens reused

`.card`, `.card-footer`, `.save-state`, `.save-state-dot`, `.spinner`, `.field`,
`.field-label`, `.btn-primary`, `.btn-secondary`, `.btn-sm`, `.pill`, and CSS
variables (`--muted`, `--faint`, `--danger`, `--danger-soft`, `--surface-muted`, etc.).
No new CSS is expected; if a small utility is unavoidable it will be a Tailwind class,
not a new global rule (Req 2.3).

## Non-goals / risks

- Not adding new settings or a nav rail (revisit if settings grow past ~5 groups).
- Risk: refactoring shared `busy` could regress the "disable during modal" behavior —
  mitigated by mapping each former `busy` use to the correct per-card status.
- Risk: heading-level change could affect any snapshot tests — verify against existing
  tests in `desktop/src/lib` and the build.

## Verification approach

- `npm run build` (type-check + Vite build) in `desktop/`.
- `npm run lint` (oxlint).
- Run existing tests (`npm test` / vitest `--run`) to catch regressions.
- Manual pass: load page, save keywords (see Saved state), save location, choose CSV
  (modal import/replace), use default, force an error path per card to confirm scoping.
