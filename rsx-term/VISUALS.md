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
