# CLAUDE.md — rsx-playground

Local to `rsx-playground/`. Inherits everything from the repo-root
`../CLAUDE.md`. This file records the **visual design system** for the
dashboard (server-rendered FastAPI + HTMX + Tailwind, dark theme).

## The one visual rule: LEFT BARS

Callouts, hints, section accents, and typed status blocks use a
**left-border accent bar** — never a top banner, never a full-border
box.

- **ALWAYS** `border-l-4` (prominent: hints, primary callouts) or
  `border-l-2` (subtle: inline rows, secondary notes) on a
  `bg-slate-800/50` panel.
- **NEVER** top-border banners (`border-t`), full-ring boxes
  (`border` all sides) for callouts, or full-width colored header
  bars. They read as chrome; the left bar reads as an annotation on
  the content.
- The **bar colour carries the meaning** (see palette): the text stays
  neutral slate; the semantic signal is the bar, not a loud background.

Reference: the walkthrough hint (`_hint_bar`, `pages.py`) is the
canonical form — `border-l-4 border-blue-500 bg-slate-800/50`, ≤2
sentences, dismissable. Copy that shape for new callouts.

## Palette + semantics

Dark theme, class-based (`html.dark`, synchronous `<head>` toggle to
avoid flash). Base is slate; colour is reserved for meaning, used
sparingly.

| Role | Class | Use |
|---|---|---|
| base text | `text-slate-300/400` | body |
| muted | `text-slate-500/600` | labels, captions, secondary |
| success / live | `text-emerald-400`, bar `border-emerald-500` | ok, filled, running |
| info / hint / link | `text-blue-400`, bar `border-blue-500` | hints, links, neutral status |
| error / reject | `text-red-400`, bar `border-red-500` | failures, rejects, down |
| warning / degraded | `text-amber-400`, bar `border-amber-500` | stale, paused, "no live book" |
| accent / heading | `text-cyan-400` | section headers, the ⚡ speed motif |
| panel bg | `bg-slate-800/50` | cards, callouts, hint bars |

Rules:
- Colour = meaning, never decoration. A green thing is *live/filled*, a
  red thing *failed/down*, amber *degraded* — nothing is coloured "to
  look nice."
- One accent per block. Don't stack a coloured bg + coloured border +
  coloured text; pick the bar.

## Components

- **Panel / card:** `bg-slate-800/50 rounded`, content padded; a
  left-bar only when the block is a typed callout (status/hint/error).
- **Hint bar:** left-bar blue, ≤2 sentences, flow/direction only
  ("orders enter here — next → Risk"), global `localStorage.rsxHints`
  toggle. NEVER metrics or component re-explanations in a hint.
- **Status cell:** the word carries a semantic text colour
  (`filled`=emerald, `resting`=blue, `rejected`=red, `hung`=amber);
  a stale/degraded surface gets a left-bar + `opacity-40 grayscale`.
- **Tables:** slate borders, muted header row (`text-slate-500`),
  numbers right-aligned; no zebra, no heavy gridlines.

## Copy / density (inherits root CLAUDE.md)

- Lowercase info, Capitalize errors ("checking…" vs "Failed: …").
- Terse. No walls of text — a callout is ≤2 sentences; deeper detail
  lives on the component page / docs, not in a bar.
- HTMX partials: one concern per `/x/...` endpoint; render server-side.

## When you touch the UI

- New callout → left bar, semantic colour, `bg-slate-800/50`. No
  banners.
- New colour → only if it maps to a new *meaning*; otherwise reuse the
  table above.
- Keep the Playwright gate green (nav order, tab count, redirects).
