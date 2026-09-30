# Job hunt tooling proposals (free / reasonable build)

**Status:** draft for review
**Date:** 2026-09-29
**Context:** Ideas from New job career bot based on Cyril’s passive hunt workflow (Job Tracker + `jt`, a16z Jobs, Lenny’s/TrueUp digests, Ashby/Greenhouse live-vet, Friday routine). Goal: improve hunt quality/speed with tools that can be built or incorporated for free (or near-free).

**Suggested first build:** `jt vet-url` + safe archive close — unblocks every Friday hunt and cleanup pass.

---

## Highest leverage — extend Job Tracker / `jt`

### 1. `jt vet-url` / batch live-vet
Given Ashby / Greenhouse / Lever URLs, return:
- open vs closed
- title, location, posted compensation
- Ashby `isListed` (or GH board presence)

We already scrape boards by hand during hunts; one CLI command would cut a large share of hunt time and reduce “already tracked” / SPA-shell confusion.

### 2. Safe close = archive by default
Document and/or fix the quirk where `jt update --status closed` hard-deletes rows in the current CLI build. Add something like:

```bash
jt close <id> --reason "…"
```

Behavior: always `--archive`, append a dated note, then sync CSV. Matches how cleanups are already done safely.

### 3. Comp parser on wishlist
Pull posted bands from Ashby/Greenhouse into `comp_min` / `comp_max` (or equivalent). Enable filters such as:

```bash
jt list --max-comp 130000
```

Directly supports cleanup passes (“close under $130k”) without manual note scraping.

### 4. Ghost-job heuristics flag
Signals (non-exhaustive):
- age since publish / YC `lastActive`
- empty Work at a Startup sibling page
- SPA shell title “Jobs” with no JD body
- soft-close copy without a hard “closed” state

Set `posting_state=suspect` (or similar) **without** auto-closing — operator reviews, same as the Deeptrace ghost flow.

### 5. Company risk notes join
Optional link from company → risk tier in `company-risk-notes-YYYY-MM-DD.xlsx` (Desktop Job Tracker app folder) so digests can surface “Notable = MEDIUM watch” next to wishlist roles.

---

## Free sources / connectors (use or wire — don’t buy)

### 6. Lenny’s / TrueUp full list without login wall
Browser playbook: open the digest link while signed into personal Gmail, export the job table to CSV for de-dupe against Job Tracker. Biggest recurring Friday gap (teaser strip only; Jerry skip still applies).

### 7. a16z Jobs archive watcher
Cron/routine that diffs `https://a16zjobs.substack.com/` archive titles since the last Friday hunt. Manual today.

### 8. SEC Form D + Crunchbase-lite
Free EDGAR Form D pull for “months since last raise” on MEDIUM-risk names (used for Aerdos). Small script; no paid PitchBook.

### 9. Greenhouse / Ashby board watch diffs
Watches already exist. Expose triage such as:

```bash
jt watches triage --since 7d --fit-keywords "data,LLM,ML,infra"
```

Avoids manually dismissing Thinking Machines policy/legal/HR noise while keeping stretch eng fits.

---

## Apply-side (free; high friction today)

### 10. Apply-packet generator from template
Given a wishlist id:
1. Copy Jul22 CV into `apply-packets/<Co>-<Role>-<date>/` (never mutate the original Jul22 CV).
2. Draft bullet diffs for **approval only** before any new version is written.
3. Pull JD text into `jd.md`.

Matches Cyril’s resume preferences (minimal edits, show proposed edits first).

### 11. Gmail application-receipt matcher
Search personal Gmail for application confirmations and suggest flipping wishlist → applied. Gmail connector already available to the career bot.

---

## Nice-to-have (still free)

### 12. Public careers sitemap / API ping
For non-Ashby employers (e.g. Apple jobs API) with clearer open/closed than HTML string guessing.

### 13. Duplicate-company merger
Merge duplicate company rows (e.g. old “Thinking Machine Labs” vs “Thinking Machines Lab”) in `jt` / app.

### 14. Weekly risk refresh script
Re-run Form D + news queries only for MEDIUM/HIGH rows in the risk spreadsheet.

---

## Skip / not worth free DIY

| Idea | Why skip |
|------|----------|
| Paid LinkedIn Sales Nav / Layoffs.fyi Pro | Headcount signals too noisy for the cost |
| Auto-apply bots | Against process and usually against ATS ToS |
| Full PitchBook clone | Form D + company blogs covered ~80% of risk-screen needs |

---

## Workflow constraints these tools must respect

- Never auto-favorite / pin jobs (Cyril favorites manually).
- Live-vet canonical ATS URLs before recommending.
- Comp filters: surface roles likely **>$150k**; relocate-elsewhere only if **≥$175k**. Cleanup passes may use a lower explicit threshold (e.g. $130k) when Cyril asks.
- Priority themes: AI/LLM → data eng → combo; DS tracks OK; **always skip Jerry**.
- Keep Job Tracker app and CSV mirror in sync (`jt sync`).
- Digests: workflow log → new vetted keepers → still-open wishlist.

---

## Review checklist

- [ ] Agree on first build: `jt vet-url` + `jt close --reason`?
- [ ] Comp fields schema (`comp_min` / `comp_max` / currency / source)?
- [ ] Ghost flag UX: badge in app vs CLI-only?
- [ ] Lenny export: browser playbook skill vs in-app helper?
- [ ] Any of 10–14 to schedule after the first build?
