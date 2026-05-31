# Embed-walkthrough-as-hints — opus oracle eval (2026-05-31)

Advisory only. Owner decides whether to implement.

## Verdict: DO A VARIANT (not the literal proposal)

- **Per-page hint boxes: yes** — but ≤2 sentences, dismissable,
  carrying the one thing tooltips can't: the narrative "next →".
- **Reorder nav: yes, light regroup** (teaching arc, then ops
  tools) — NOT a hard 1:1 narrative reorder (some tabs serve two
  roles, e.g. WAL).
- **Delete /walkthrough: NO — repurpose** into the Overview
  landing. Keep the hero, the Start-All launcher, the architecture
  + lifecycle diagrams; migrate the 9 `_wt_section` TL;DRs into the
  per-page hints. Redirect /walkthrough → /overview (don't 404).
- **"SCREENS" preservation: skip** — screenshots of own HTML are
  strictly worse than the HTML; git history already preserves the
  prose. Low value, real maintenance cost.

## Central tension to resolve first
The hint boxes ARE prose — exactly the "no walls of text" the
sprint-29 audit (AUDIT.md theme A) tells us to avoid, ×~18 pages.
Survivable ONLY if: ≤2 sentences, globally dismissable (sticky),
and they carry flow/direction ("orders enter here, next → Risk"),
NEVER metrics or re-explanations of components (those live in the
sprint-29 component blurbs + benches). If a hint grows past two
sentences it has become the walkthrough again, smeared across nav.

## Proposed new TABS order (teaching arc | ops tools)
`Overview · Topology · Components · Cast · Book · Risk · WAL ·
Latency · Orders · Trade · | · Maker · Control · Logs · Verify ·
Faults · Stress · Docs`

## Which pages get a narrative hint (~8), which don't
- Hints (with "next →"): Overview→Topology→Components→Cast/WAL→
  Book→Risk→Latency→Orders→Trade.
- NO narrative hint (ops tabs): Logs, Verify, Faults, Stress — at
  most a one-line purpose caption, no "next →".
- Self-describing already (no dup): Components, Docs, Trade.

## Mechanics (simplest that works)
- ONE global `localStorage.rsxHints` on/off flag (not 18 per-page
  keys). First visit shows hints; one "Hide hints" kills them
  site-wide; a nav "Show hints" toggle restores. Render server-side,
  toggle a `hidden` class synchronously in <head> (mirror the
  existing darkMode:'class' approach) to avoid flash.
- Style: reuse `_wt_section` TL;DR bar (`border-l-4 border-blue-500
  bg-slate-800/50`) so it reads as a hint, not a banner.

## MVP implementation sketch (if owner proceeds)
1. `PAGE_HINTS` dict in pages.py: `{active_tab: (hint, next_href,
   next_label)}`, only the ~8 narrative pages.
2. `layout()` injects the hint after <nav> when `active_tab in
   PAGE_HINTS`; one <script> reads the localStorage flag + nav toggle.
3. Fold walkthrough hero + launcher + diagrams into `overview_page()`;
   delete the 9 `_wt_section` detail bodies; redirect /walkthrough →
   /overview.
4. Reorder TABS.
5. Update Playwright/API tests; `make gate` then `make release-gate`
   MUST stay 421/421 (reorder + route removal will break nav-order
   and /walkthrough tests — grep first).

## Risks
1. Walls-of-text contradiction (hints are prose) — hold ≤2 sentences.
2. Duplication with component blurbs/tooltips — hints carry
   flow/direction only, never metrics.
3. Loss of whole-system artifacts (arch/lifecycle diagrams, Start-All)
   if /walkthrough hard-deleted — must land on Overview first.
4. Test/gate breakage from TABS reorder + route removal.
5. Annoyance on return visits if dismiss isn't sticky + site-wide.
6. Maintenance drift of duplicated facts — keep numbers out of hints.
