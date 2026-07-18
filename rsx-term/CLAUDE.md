# CLAUDE.md — rsx-term

Local to `rsx-term/`. Inherits the root `../CLAUDE.md`. This file pins the
**documentation conventions and the invariants that must not regress** — the
keeper list, so a later "cleanup" can't quietly gut what makes this crate good.

## Doc topology (which file answers which question)

Follow the `doc-topology` skill (itself distilled from rsx-cast/rsx-book):

| File | The one question | Notes |
|---|---|---|
| `README.md` | what / why / how-to-run | elevator pitch line 2, glossary before jargon, quick-start, guarantees, "when NOT to use", "how to read this" |
| `ARCHITECTURE.md` | how it's built | module table, data-flow diagram, the heatmap algorithm, invariants, decisions *with the rejected alternatives* |
| `ONEPAGER.md` | one-page distillation | keep it to a page |
| `DEMO.md` | guided first run that *teaches* order-flow reading | education-first |
| `notes/*.md` | why each design choice | Problem → Fix → Cost-it-removes, cited, one file per decision |
| `SCREENS.md` / `VISUALS.md` / `FLOWS.md` | what each screen shows / the visual language / user journeys | keep in lockstep with the code |

`notes/` is the *why* layer: `compression.md` (the three nonlinear
compressions), `glyph-bank.md` (empirical glyph calibration), `honesty.md`
(never fabricate), `venue-seam.md` (exchange = plugin), `assistant.md`
(chat pane → arizuko over a route token + SSE), `element-inventory.md` (the
full visual-element list, tagged for the GPU-rebuild prune). Add one per new
non-obvious decision.

## Visual design system

The render is a **pure function of the model**, so the whole visual surface is
specified — not just implied by the code. Full reference: `VISUALS.md` (the
language), `SCREENS.md` (per-screen element inventory), `notes/element-
inventory.md` (the minimize/optimize list). Definitions live in `ui/styles.go`
(palette), `ui/stream.go` (`glyphs` table + ramps), `book/heatmap.go`
(compressions). This section is the terse index.

**Palette — Ayam Cemani, colour = meaning.** Green-tinged near-black base
(`ColorPageBg #040806`, `ColorPanelBg #0d1712`, `ColorBorder #16211b`) with the
bird's beetle-green + violet sheen. Hexes verbatim-shared with the dashboard
(`rsx-playground/CLAUDE.md`) and spec 55. Each colour is one meaning: `ColorLive`
=`ColorBid` `#22f5a1` (live/long/bid/filled/positive), `ColorAsk` `#f87171`
(short/ask/down/reject/negative), `ColorHeading` `#bd83ff` (badge/⚡),
`ColorAccent` `#a992ff` (own-order/sparkline/mode-tag/keycast), `ColorRing`
`#7c3aed` (overlay ring), `ColorDegraded` `#fbbf24` (stale/offline), plus
text/muted greys. `UseTheme("colorblind")` swaps ONLY bid/ask → blue/orange.

**The encoding laws** (heatmap cell = up to three co-equal channels):
- **Cell = background-colour(size) + glyph-layer(shape).** Background is the
  resting size on ONE sequential ramp; the glyph is a separate channel.
- **ONE ramp, both sides.** `rampColor(sizeTier)` = `blendHex(PageBg→Live,
  tier/4)`, log-scaled on a stable (rise-fast/decay-slow) basis. **Side is
  position** (bids left / asks right of the mid gap), NEVER colour.
- **Shape is its own channel.** `countRamp ░▒▓█` = order count, `▚` = ≥30 s
  persistence, `◇` = your order, `▲`/`▼` on the ruler, `┃` = cursor — each glyph
  means exactly one thing (the `glyphs` table; calibrated by `tools/glyphbank`).
- **Trades are co-equal.** Prints overlay the book in the AGGRESSOR hue with a
  distinct magnitude ramp `○◆●■` (free — the book uses no hue).
- **Colour = meaning, never decoration.** Add a colour only for a new meaning.
- **Honesty markers.** `—` unknown · `~` + `StyleDerived` (dim italic) estimate ·
  hard-block over the fat-finger cap (never a dismissable warning) · far rows
  dimmed + window-labelled (aggregate ≠ a book) · withhold on overflow.

**Layout regions.** Streaming grid (repainted in place, no scrollback): header
(badge · link dot · price-axis legend) → mode line (screen tag · venue · RO/PO ·
size · list) → body (news rail · heatmap cells · log-time gutter · tape rail;
far/live/NOW rows) → ruler (fisheye baseline · touch `┼` · own `▲▼` · cursor `┃`)
→ footer (touch ladders · position/PnL · latency ⚡ · hint / microscope line) →
keycast chip (bottom-right, opt-in). Classic DOM view: status bar · three
columns (book 44 / order 36 / right 34) · speed strip · status · help.

**Rebuild-from-this.** The three files above specify every colour, glyph, ramp
(exact math), the three compressions (exact boundaries: fisheye `linearTicks`
8 + triangular schedule, log-time `farSpans` 10s/60s/120s/300s/600s/1h, size
`sizeTiers` 4), and the degradation ladder (`modeTrue`→`modeShade`→`modePlain`).
A GPU/desktop successor replaces only the model→render boundary; nothing here
depends on the terminal beyond the glyph-coverage calibration.

## Keeper invariants — do NOT regress

These are load-bearing; a "simplification" that drops one is a bug, not a cleanup:

- **Keyboard-only.** No mouse input, ever. The price cursor is `h`/`l`; there is
  no click-to-price.
- **Never fabricate a number.** Dash the unknown (`—`), mark estimates (`~`),
  hard-block over the fat-finger cap (never a dismissable warning), withhold an
  overflowing P&L rather than show it wrapped. See `notes/honesty.md`.
- **Offline by default.** The mock/default path makes zero network calls; every
  dial site sits behind an env opt-in (`RSX_TERM_VENUE`, `RSX_TERM_NEWS`) and a
  named goroutine.
- **The public feed is untrusted** — validate market-data frames at the `conn`
  edge (skip malformed sides/prices, don't coerce them).
- **The DOM view is byte-locked.** `TestDomViewGolden` / `TestBookViewGolden` /
  `TestDefaultViewUnchanged` must stay green; the streaming work can't regress the
  classic view or the book render.
- **The model is UI-agnostic.** `book/` folds are pure and clock-free; the
  render is a pure function of the model; the LLM handoff (`news.AssistantContext`)
  is a plain struct — so a future GPU/bitmap frontend replaces only the render.
- **Honest compression.** Far heatmap rows are aggregate windows, labelled as
  such — never dressed up as an exact book. No replay buffer, no lead-lag over
  jittery 100 ms mids (both deliberately rejected — see the direction pivot).

## When you touch this crate

- New design decision → a `notes/` file (Problem → Fix → Cost).
- New screen behaviour → update `SCREENS.md` + the keymap-generated help stays the
  source of truth for keys.
- Run `make fmt` + `go test -race ./...` + the goldens before committing.
- The glyph vocabulary is calibrated per font — re-run `tools/glyphbank` if the
  target terminal font changes.
