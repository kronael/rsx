# CLAUDE.md — rsx-playground

Local to `rsx-playground/`. Inherits everything from the repo-root
`../CLAUDE.md`. This file records the **visual design system** for the
dashboard (server-rendered FastAPI + HTMX + Tailwind, dark theme).

## The one visual rule: SURROUNDING ACCENTS

Callouts, hints, section accents, and typed status blocks use a
**full-ring accent border** (a "ribbon") — never a left-only bar,
never a top banner. This matches krons.fiu.wtf (`.tldr-box` = 2px
all-sides, `.insight` = 1px all-sides), both `border-radius:3px`.

- **ALWAYS** `border-2 border-{c}-500/70 rounded-[3px]` (prominent:
  hints, primary callouts) or `border border-{c}-500/60 rounded-[3px]`
  (subtle: inline rows, secondary notes) on a `bg-slate-800/50` panel.
- **NEVER** left-only bars (`border-l-*`), top-border banners
  (`border-t`), or full-width colored header bars. A surrounding ring
  reads as a framed annotation on the content; a partial edge reads as
  chrome.
- The **border colour carries the meaning** (see palette): the text
  stays neutral slate; the semantic signal is the ring, not a loud
  background.

Reference: the walkthrough hint (`_hint_bar`, `pages.py`) is the
canonical form — `border border-slate-600/60 rounded-[3px]
bg-slate-800/50`, ≤2 sentences, dismissable. Copy that shape for new
callouts.

## Palette + semantics

Dark theme, class-based (`html.dark`, synchronous `<head>` toggle to
avoid flash). Colour is reserved for meaning, used sparingly.

**Palette: Ayam Cemani ("black iridescence").** The base is a
green-tinged near-black; the accents are the bird's beetle-green +
violet feather-sheen. The Tailwind scales are **retuned in one place**
— `pages.py` `tailwind.config` `theme.extend.colors` — so class names
below are unchanged; only the rendered colour moved:
- `slate` → green-tinged near-black ramp (page `#040806`, panel
  `#0d1712`, borders `#16211b`, body text `#a9bcb2`)
- `emerald` → neon beetle-green `#22f5a1` (live / filled / speed ⚡)
- `blue` + `cyan` → dark violet (`#a992ff` info, `#bd83ff` heading,
  `#7c3aed` ring) — the purple sheen
- `red` / `amber` → default (error / degraded keep their meaning)

To change the theme, edit that one `colors` block — do NOT hand-swap
classes across the file.

| Role | Class | Use |
|---|---|---|
| base text | `text-slate-300/400` | body |
| muted | `text-slate-500/600` | labels, captions, secondary |
| success / live | `text-emerald-400`, ring `border-emerald-500` | ok, filled, running |
| info / hint / link | `text-blue-400`, ring `border-blue-500` | hints, links, neutral status |
| error / reject | `text-red-400`, ring `border-red-500` | failures, rejects, down |
| warning / degraded | `text-amber-400`, ring `border-amber-500` | stale, paused, "no live book" |
| accent / heading | `text-cyan-400` | section headers, the ⚡ speed motif |
| panel bg | `bg-slate-800/50` | cards, callouts, hint bars |

Rules:
- Colour = meaning, never decoration. A green thing is *live/filled*, a
  red thing *failed/down*, amber *degraded* — nothing is coloured "to
  look nice."
- One accent per block. Don't stack a coloured bg + coloured border +
  coloured text; pick the ring.

## Components

- **Panel / card:** `bg-slate-800/50 rounded`, content padded; a
  surrounding ring only when the block is a typed callout
  (status/hint/error).
- **Hint bar:** `border rounded-[3px]` ring, ≤2 sentences,
  flow/direction only ("orders enter here — next → Risk"), global
  `localStorage.rsxHints` toggle. NEVER metrics or component
  re-explanations in a hint.
- **Status cell:** the word carries a semantic text colour
  (`filled`=emerald, `resting`=blue, `rejected`=red, `hung`=amber);
  a stale/degraded surface gets a ring + `opacity-40 grayscale`.
- **Tables:** slate borders, muted header row (`text-slate-500`),
  numbers right-aligned; no zebra, no heavy gridlines.

## Copy / density (inherits root CLAUDE.md)

- Lowercase info, Capitalize errors ("checking…" vs "Failed: …").
- Terse. No walls of text — a callout is ≤2 sentences; deeper detail
  lives on the component page / docs, not in a callout.
- HTMX partials: one concern per `/x/...` endpoint; render server-side.

## When you touch the UI

- New callout → surrounding ring, semantic colour, `bg-slate-800/50`.
  No banners, no left-only bars.
- New colour → only if it maps to a new *meaning*; otherwise reuse the
  table above.
- Keep the Playwright gate green (nav order, tab count, redirects).
