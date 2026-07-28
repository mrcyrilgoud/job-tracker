# Job Tracker — Design System

The source of truth for how Job Tracker looks and feels. When in doubt, this file wins.

## The one thing to remember

Opening Job Tracker in the morning should feel **calm, encouraging, and personal** — a quiet
place that gives you a sense of momentum in a stressful search, not a database console. Every
choice below serves that feeling.

## Visual thesis

A warm, editorial workspace. Ivory paper, a clay-coral accent, a characterful serif for
headings, and a friendly geometric sans for everything else. Depth comes from soft shadows and
generous whitespace, not from hard borders and boxes. The product speaks like a helpful person,
not like a schema.

Anchored between two references:

- **Claude** — the calm warm canvas, the serif headings, the breathing room, the human tone.
- **Airbnb** — the soft elevation, the rounded friendliness, the confident use of color for
  status and delight.

## What we are moving away from

The previous UI felt austere and technical:

- Cold teal accent on warm paper (temperature clash).
- Default Geist Sans/Mono — no personality.
- Hard `border` boxes everywhere, no depth or hierarchy.
- Raw database language leaking into the UI: `postingState: unknown`, "board slug",
  "consecutive sync failures", ISO/`toLocaleString()` timestamps, "From greenhouse".
- Flat lowercase status tokens instead of human, color-coded states.

## Color

Warm neutrals + a single clay-coral accent + a soft, friendly semantic palette for status.
All tokens live in `desktop/src/index.css` as CSS custom properties.

### Neutrals & accent

| Token | Value | Use |
| --- | --- | --- |
| `--background` | `#faf8f4` | App canvas (warm ivory) |
| `--surface` | `#ffffff` | Cards, panels, inputs |
| `--surface-muted` | `#f6f2ec` | Subtle fills, hover, table headers |
| `--foreground` | `#292524` | Primary text (warm near-black) |
| `--muted` | `#78716c` | Secondary text |
| `--faint` | `#a8a29e` | Tertiary text, placeholders |
| `--border` | `#ece5db` | Hairline separators (used sparingly) |
| `--accent` | `#c25e3a` | Primary actions, links, active state (clay coral) |
| `--accent-hover` | `#a94e2f` | Accent hover/press |
| `--accent-soft` | `#f7e8df` | Accent-tinted fills, active nav, chips |
| `--accent-ink` | `#8c4326` | Text/icon on accent-soft |

### Semantic status palette (soft, colorful)

Each state is a soft tinted fill + a saturated ink. Used for pipeline status and posting freshness.

| Meaning | Fill | Ink |
| --- | --- | --- |
| Neutral / early (wishlist) | `--violet-soft #ede9fb` | `--violet-ink #5b4fbe` |
| In progress (applied) | `--blue-soft #e5eefa` | `--blue-ink #2f5f94` |
| Attention / active (interviewing, open posting) | `--amber-soft #fbefd8` | `--amber-ink #96660f` |
| Positive (offer, open) | `--green-soft #e4f2e9` | `--green-ink #1f7a48` |
| Closed / ended (rejected, withdrawn, closed, inactive) | `--stone-soft #f0ece7` | `--stone-ink #857b72` |
| Error | `--danger-soft #fbe9e6` | `--danger #b0432f` |

Rule: never show a raw enum value to the user. Map it to a human label + a tone (see
`desktop/src/lib/ui.ts`).

## Typography

Two variable fonts loaded from Google Fonts in `desktop/index.html`, exposed as CSS variables.

- **Display / headings — Fraunces.** A soft modern old-style serif. Warm, editorial,
  characterful. Used for page titles, the wordmark, and large numbers. Weights 400–600.
- **Body / UI — Plus Jakarta Sans.** A friendly geometric-humanist sans. Highly readable,
  modern, approachable. Everything else: nav, labels, inputs, table text, body copy.

| Role | Font | Size / weight |
| --- | --- | --- |
| Page title (`h1`/`h2`) | Fraunces | `text-2xl`/`text-3xl`, weight 500, tight tracking |
| Section heading | Fraunces | `text-lg`, weight 500 |
| Body | Plus Jakarta Sans | `text-sm`/`text-base`, weight 400 |
| Emphasis / labels | Plus Jakarta Sans | weight 500–600 |
| Numbers / metrics | Fraunces | large, weight 500 |

Banned: no all-caps `tracking-[0.2em]` micro-labels as a primary device (that was the old
"technical" tell). Use sentence case and weight for hierarchy instead. A single small-caps
eyebrow is allowed sparingly.

## Shape, depth & spacing

- **Radius:** cards/panels `rounded-2xl` (16px) to `rounded-3xl` (24px) for hero surfaces.
  Inputs/buttons `rounded-xl` (12px). Pills `rounded-full`.
- **Depth:** prefer soft shadows over borders. Tokens:
  - `--shadow-sm: 0 1px 2px rgba(41,37,36,.04), 0 1px 3px rgba(41,37,36,.06)`
  - `--shadow-md: 0 4px 12px rgba(41,37,36,.06), 0 2px 4px rgba(41,37,36,.04)`
  - `--shadow-lg: 0 12px 32px rgba(41,37,36,.10)`
  Cards use `--shadow-sm`, lift to `--shadow-md` on hover. Borders only as faint hairlines
  when a shadow would be too heavy (e.g. inside-card dividers).
- **Whitespace:** generous. Card padding `p-5`/`p-6`. Section gaps `gap-6`/`gap-8`.
  Let content breathe; density is the enemy of calm.

## Components

- **Buttons.**
  - Primary: `--accent` fill, white text, `rounded-xl`, `--shadow-sm`, hover `--accent-hover`.
  - Secondary: `--surface` fill, hairline border, `--foreground` text, hover `--surface-muted`.
  - Ghost/link: accent text, no fill.
- **Cards:** `--surface` on `--background`, `rounded-2xl`, `--shadow-sm`, `p-5`.
- **Inputs:** `--surface` fill, hairline border, `rounded-xl`, focus ring in `--accent` at low
  opacity. Comfortable `px-3.5 py-2.5`.
- **Status pill:** soft tinted fill + ink from the semantic palette, `rounded-full`,
  sentence case, small dot optional. Never uppercase raw enum.
- **Nav:** pill links; active link uses `--accent-soft` fill + `--accent-ink` text.
- **Empty states:** friendly one-liner + a single clear primary action. No dead ends.

## Voice & microcopy

Talk like a helpful person. Humanize every machine detail:

- Posting freshness: `active → "Open"`, `inactive → "Closed"`, `unknown → "Not checked yet"`.
- Timestamps: relative ("Updated 2 days ago", "Checked yesterday"), never raw ISO or
  `toLocaleString()` in the main flow.
- Sources: "Found via Greenhouse watch" instead of "From greenhouse".
- Watches: "Syncing automatically" / "Needs attention" instead of "consecutive sync failures".
  Keep board slug and other plumbing as a small, de-emphasized detail — visible if you look,
  never shouting.
- Gmail: explain in plain language what connecting does; tuck OAuth/redirect-URI/Keychain
  specifics under a quiet "Technical details" area rather than in the headline.

## Accessibility

- Body text meets WCAG AA on the ivory canvas (`--foreground` on `--background` ≈ 12:1).
- Status inks meet AA on their soft fills; never rely on color alone — pair with a label.
- Visible focus ring (accent, 2px, low-opacity halo) on all interactive elements.
- Respect `prefers-reduced-motion`: hover lifts and transitions drop to none.
