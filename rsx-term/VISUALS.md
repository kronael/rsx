# VISUALS — rsx-term design system

The visual language of the RSX trading terminal (Go / Bubble Tea + Lipgloss).
It is the terminal-native sibling of the dashboard's design system
(`rsx-playground/CLAUDE.md`) — same palette, same "colour = meaning" rule,
rendered in a TTY instead of a browser. Defined in code at
`rsx-term/ui/styles.go`; this file is the why + the catalogue.

## The one rule: colour is meaning, never decoration

A green thing is *live / long / bid / filled*. A red thing is *short / ask /
down / rejected*. Amber is *degraded / stale / offline*. Violet is *heading /
accent / overlay*. Nothing is coloured "to look nice." Add a colour only when
it maps to a new **meaning** — never a new shade for its own sake.

Because the whole scheme leans on green-vs-red, and that pair is invisible under
deuteranopia/protanopia (~8% of men), `RSX_TUI_THEME=colorblind` swaps *only*
the bid/ask pair to a colourblind-safe blue(bid)/orange(ask); the rest of the
palette is unchanged. The non-colour cues (B/S tape glyphs, LONG/SHORT labels,
bid-left/ask-right geometry, `▸`/`‹` marks) carry the same meaning without any
colour at all.

## Palette — Ayam Cemani ("black iridescence")

A green-tinged near-black base with the bird's beetle-green + violet
feather-sheen. Hexes are verbatim-shared with the dashboard and spec 55, so
the terminal and the web UI read as one product.

| Meaning | Const (`styles.go`) | Hex |
|---|---|---|
| live / long / bid / filled | `ColorLive` = `ColorBid` | `#22f5a1` |
| short / ask / down / reject | `ColorAsk` | `#f87171` |
| heading / badge / ⚡ accent | `ColorHeading` | `#bd83ff` |
| info / secondary accent | `ColorAccent` | `#a992ff` |
| overlay ring (confirm / trace) | `ColorRing` | `#7c3aed` |
| body text | `ColorText` | `#a9bcb2` |
| bright / focused / active statusline | `ColorTextBright` | `#e7eeea` |
| muted — labels / captions / help | `ColorMuted` | `#586b62` |
| degraded / stale / offline | `ColorDegraded` | `#fbbf24` |
| panel bg / page bg / border | `ColorPanelBg` `#0d1712` · `ColorPageBg` `#040806` · `ColorBorder` `#16211b` |

## Style roles

Foreground pairings + the two panel frames, all in `styles.go`:

- `StyleText` / `StyleTextBright` / `StyleMuted` — body, focused/active, and
  labels/dim.
- `StyleLive` / `StyleAsk` — positive/long/bid vs negative/short/ask figures.
- `StyleHeading` — section titles, the symbol badge, the ⚡ speed motif.
- `StyleDegraded` — offline dot, the "no live book" row, an unpriceable uPnL.
- `StyleDerived` — **dim + italic**, always paired with a `~` prefix, for any
  client-computed value (mark=mid, uPnL off it). This is the terminal's tell
  that a number is a local *estimate*, not exchange-authoritative data. Never
  render a derived value in a plain style.
- `PanelStyle` — the standard bordered panel: `NormalBorder` in `ColorBorder`.
  Title is the first content line, in `StyleMuted`.
- `RingPanelStyle` — a `NormalBorder` in the violet `ColorRing`, for overlays
  that demand attention: the submit confirm preview and the F3 trace HUD.

## Layout

One screen, vertically stacked (`view.go` `View`):

```
 status bar          ← symbol badge · link dot · counts · last/~mark/index/funding
 ┌ book ┐┌ order ┐┌ right ┐   three columns (JoinHorizontal)
 speed strip         ← ⚡ RTT = net + internal + engine · p50 · best
 status line         ← last event / feedback
 help line           ← keybinding legend
```

- **Three columns**, fixed content widths: `book` 38, `order` 36, `right` 34
  (`view.go` constants) — sized so the widest honest row (the degraded-book
  message, the confirm box) fits without wrapping.
- The **right column** is positions + trades stacked, OR the trace HUD when
  F3 is on (it swaps the whole column — Lipgloss has no overlay compositing,
  a noted, honest deviation from the Rust overlay).

## Marks & glyphs

- **Static ladder** — a fixed price axis, bid qty (green) left of the price
  column, ask qty (red) right; empty prices are gaps. The axis recentres only
  on drift, never per tick.
- **Depth bar** `▊` (side colour) — a per-level histogram scaled to the
  deepest visible level; bid grows left, ask right, so the bars converge on
  the spread. A lighter `▊` vs the imbalance bar's solid `█` keeps the
  per-level texture secondary to the summary.
- **Own-order mark** `▸` (accent violet) on a ladder row (or the orders-panel
  cursor row) — where the trader has a resting order.
- **Last-trade mark** `‹` (bright) on the ladder row of the last print.
- **Imbalance bar** — a green(bid)|red(ask) split of the visible depth under
  the ladder, with the bid share.
- **ARMED banner** — bright text on ask-red, bold, full-width, persistent
  while confirm-off (`F2`) is on: the guardrail-is-down warning (`StyleArmed`).
- **Risk row** (positions) — `liq — · ROE — · mgn ░░░░` dashed in
  `StyleDerived`: the risk surface's fixed home, never a fabricated number.
- **Latency hop-bar** — a proportional `net│internal│engine` segment bar in
  the speed strip (each leg its own colour); the RTT number goes amber/red on
  an SLA breach.
- **Sparkline** `▁▂▃▄▅▆▇█` (accent) — the rolling RTT window.
- **Link dot** `●` — green live / amber offline.
- **Side buttons** `  BUY  ` / `  SELL  ` — the active side is `Reverse`d.
- **Focused field** — bold + bright with a trailing `_` cursor; unfocused
  fields are muted.
- **Derived prefix** `~` — on `~mark`, `~uPnL` (see `StyleDerived`).
- **Tape glyph** `B` / `S` before each print, so side reads without colour.

## State → style

| State | How it looks |
|---|---|
| order link up / down | `● live` green / `● offline` amber |
| marketdata down or empty book | amber `no live book — market-data stream down` row |
| value is a client estimate | `~` prefix + dim italic (`StyleDerived`) |
| field has no server source | dashed `—` (never a fabricated number) |
| unpriceable uPnL (no mid) | amber `—  (needs live book)` |
| positive / negative figure | green / red |
| needs-attention overlay | violet ring (`RingPanelStyle`) |

## Rules when you touch the UI

- New colour only for a new **meaning** — otherwise reuse the table.
- One accent per element; don't stack coloured bg + border + text.
- Any client-derived number gets `StyleDerived` + `~`. No exceptions — this
  is what keeps the terminal honest about what's real vs estimated.
- A missing server field is a dash, never a plausible-looking zero.

## Streaming terminal encoding (RSX_TERM_STREAM=1)

The four-screen streaming terminal keeps the palette and adds a dense cell
language on the BOOK heatmap. Three principles govern it:

1. **Colour is magnitude, position is side.** Resting liquidity renders on
   ONE sequential ramp (page-bg → live hue, log-scaled) for BOTH sides —
   bids sit left of the mid gap, asks right, so side never needs a hue.
   The scale reference is STABLE (rises instantly to a new max, decays
   slowly) — the map never flickers from per-frame renormalisation, and a
   shown size is always a tier, never an exact number.
2. **Shape is a channel of its own.** Each glyph means exactly one thing
   (the table below); the whole vocabulary lives in one data table
   (`ui/stream.go` `glyphs`) so a re-calibrated set drops in without
   touching the renderer.
3. **Trades are co-equal with the book.** Executed flow overlays resting
   liquidity in the AGGRESSOR's hue (the bid/ask pair — free, since the
   book no longer uses it) with its own magnitude ramp, plus a tape rail
   listing the same prints.

### Glyph legend

| Glyph(s) | Channel | Meaning |
|---|---|---|
| ` ░▒▓█` | count density | resting ORDER COUNT: 0 · 1 (whale) · 2-3 · 4-7 · 8+ (wall) |
| `▁▂▃▄▅▆▇█` | micro-bar | NOW-row exact live depth (even ~0.13/step ink ladder) |
| `○◆●■` | trade magnitude | print size small → huge, aggressor-hued |
| `▚` | persistence | long-standing liquidity (level held ≥ 30s; L2 proxy behind `book.AgeSource`) |
| `◇` | own order | your resting order on the map (accent) |
| `▲` `▼` | own order | your buy / sell on the ruler (side by shape) |
| `┃` | cursor | the price cursor (game entry: `f` fires here) |
| `┼` | touch ticks | the two touch columns on the ruler |
| `─` | ruler | fisheye baseline |
| `│` | rail idle | news rail with nothing in the window |
| `·►►‼` | news severity | routine · tagged · market-moving (amber) · critical (red, bold) |
| `┆` | tape rail | trade-feed separator |
| `▸` | selection | list selection cursor (news feed; DOM orders panel) |

Calibration (DejaVuSansMono ink coverage, from the glyph-bank rasterizer):
`░▒▓█` = 0.22/0.56/0.86/1.00 — NOT an even ladder, so it only carries the
coarse categorical count channel; fine intensity rides on colour. `○◆●■` =
0.16/0.26/0.40/0.51 ascending-ink distinct shapes. Eighth-blocks are the
one even family (linear bars). Braille is EXCLUDED — tofu in
DejaVuSansMono. Re-run the rasterizer per deployment font before swapping
the table.

### Degradation tiers

`RSX_TERM_COLOR=true|shade|plain` forces; else COLORTERM/TERM/NO_COLOR
decide. `true`: full ramp background + all channels. `shade` (16-colour):
glyph density carries size, trades keep aggressor hues, no ramp background.
`plain`: glyphs only, zero escapes.

### Diverging tiles (NEWS sector map)

The one place a red↔green diverging scale exists: sector tiles coloured by
move vs the session reference, |move| graded 10/50/200/800 bp. It reuses
the bid/ask pair, so `RSX_TUI_THEME=colorblind` turns it blue↔orange
automatically. A symbol with no mid yet is a muted `—` tile — never a
fabricated 0.00%.

### Mode line

`⟦SCREEN⟧ venue RO PO size [n] list …` — the persistent answer to "what
does my next keystroke do": screen tag (accent), active venue, the RO/PO
modifier toggles (StyleArmed red when ON), armed size preset, active
watchlist, plus the pair screen's ARMED symbol and any capture-mode prompt
(x switcher, F9 venue picker). Orders fire on one key in the streaming
terminal, so the mode line is a safety surface, not decoration.
