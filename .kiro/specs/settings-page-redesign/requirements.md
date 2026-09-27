# Requirements — Settings Page Redesign

## Overview

The Settings page (`desktop/src/pages/SettingsPage.tsx`) currently presents four
independent cards as a flat, single-column stack of equal visual weight: Jobs CSV
location, Role Keywords, Location Preferences, and a conditional import/replace
modal. There is no grouping, no hierarchy, and three inconsistent save-feedback
patterns. This makes settings harder to scan and adds visual clutter.

This redesign reorganizes the existing settings into logical groups with clear
hierarchy, normalizes interaction and feedback patterns, and scopes errors to the
control they belong to — **without changing any underlying behavior, API calls, or
data model.** It is a presentation-layer refactor.

## Scope

**In scope**
- Visual and structural reorganization of `SettingsPage.tsx`.
- Grouping the three existing setting areas into logical categories with headers.
- Normalizing save-state feedback across the cards.
- Scoping error messages to the relevant card instead of one global banner.
- Demoting verbose helper text into lighter, less noisy affordances.
- Reusing existing design tokens/utilities in `index.css` (`.card`, `.card-footer`,
  `.save-state`, `.field`, `.field-label`, `.btn-*`, CSS variables).

**Out of scope**
- Adding, removing, or renaming any setting or its backing Tauri command.
- Changes to `api.ts`, Rust commands, or the data model.
- Moving the theme toggle out of the global header.
- Adding brand-new settings (notifications, account, sync, etc.).

## Requirements

### Requirement 1 — Logical grouping of settings

**User story:** As a user scanning the Settings page, I want related settings grouped
under clear category headings, so that I can find what I need without reading every
card.

#### Acceptance Criteria
1. WHEN the Settings page renders THEN the system SHALL present settings under two
   category groups: "Search filters" (Role Keywords, Location Preferences) and
   "Data & storage" (Jobs CSV location).
2. WHEN a category group is rendered THEN the system SHALL display a group heading and
   a short description distinct in weight from the individual setting card headings.
3. WHEN the page renders THEN the system SHALL preserve all three existing settings
   (Role Keywords, Location Preferences, Jobs CSV location) with their current inputs
   and controls.
4. WHERE the viewport is wide enough THE system SHALL keep the content within a
   readable maximum width consistent with the current layout.

### Requirement 2 — Clear visual hierarchy

**User story:** As a user, I want a clear visual hierarchy on the page, so that the
page title, group headings, and individual settings are easy to distinguish at a glance.

#### Acceptance Criteria
1. WHEN the page renders THEN the system SHALL display a page-level title and subtitle
   distinct in weight from group headings and card headings.
2. WHEN multiple settings appear within one group THEN the system SHALL visually
   associate them as belonging to the same group (shared container, spacing, or rule).
3. WHEN the page renders THEN the system SHALL use the existing design tokens and
   utility classes rather than introducing new ad-hoc styles.

### Requirement 3 — Consistent save feedback

**User story:** As a user changing a setting, I want consistent, predictable feedback
when a change is saved, so that I always know whether my change took effect.

#### Acceptance Criteria
1. WHEN a user saves the Role Keywords setting THEN the system SHALL show a save-state
   indicator using the same pattern as the other saveable cards.
2. WHEN a user saves the Location Preferences setting THEN the system SHALL show the
   same save-state indicator pattern.
3. WHILE a save request is in flight THE system SHALL indicate the busy state on the
   control that initiated it.
4. WHEN a save succeeds THEN the system SHALL show a transient success indication and
   THEN return the control to its idle state.
5. WHERE a card triggers an immediate action without an explicit save button (Jobs CSV
   location) THE system SHALL keep its current confirmation flow while presenting its
   status consistently with the other cards.

### Requirement 4 — Errors scoped to their control

**User story:** As a user, I want an error message to appear next to the setting that
caused it, so that I understand which action failed.

#### Acceptance Criteria
1. WHEN saving a specific setting fails THEN the system SHALL display the error within
   that setting's card rather than only in a single page-level banner.
2. WHEN an error is displayed for a card THEN the system SHALL use the existing danger
   token styling (`--danger`, `--danger-soft`) and an accessible alert role.
3. WHEN a subsequent action on the same card succeeds THEN the system SHALL clear that
   card's previous error.
4. WHERE a page-level load failure occurs (initial settings load) THE system MAY show a
   page-level message, since it is not attributable to a single card.

### Requirement 5 — Reduced inline noise

**User story:** As a user, I want concise helper text, so that the primary controls read
cleanly and long explanatory notes do not dominate the page.

#### Acceptance Criteria
1. WHEN a setting has extended guidance (e.g. the smart-cities matching note, the
   cloud-sync warning) THEN the system SHALL present it in a lighter, de-emphasized
   affordance (smaller/`--faint` footnote or collapsible hint) rather than as prominent
   body text.
2. WHEN helper text is de-emphasized THEN the system SHALL keep it readable and
   accessible (not hidden from assistive technology when relevant to the control).
3. WHEN a control needs a short label or placeholder THEN the system SHALL retain enough
   guidance to use the control without the extended note.

### Requirement 6 — Preserved behavior and accessibility

**User story:** As a user, I want the redesigned page to behave exactly as before and
remain accessible, so that nothing I rely on breaks.

#### Acceptance Criteria
1. WHEN the page loads THEN the system SHALL call the same APIs (`csvConfig`,
   `getWatchRoleKeywords`, `getLocationSettings`) and populate controls with their
   current values.
2. WHEN a user performs any existing action (choose CSV, use default, import/replace,
   save keywords, save location) THEN the system SHALL invoke the same API with the same
   arguments and semantics as today.
3. WHEN the import/replace confirmation is needed THEN the system SHALL retain the modal
   dialog with its `role="dialog"`, `aria-modal`, and labelled title.
4. WHEN sections and controls render THEN the system SHALL provide accessible names via
   headings, labels, and `aria-labelledby`/`aria-label` associations.
5. WHEN the loading state is active THEN the system SHALL indicate loading before content
   is available.
