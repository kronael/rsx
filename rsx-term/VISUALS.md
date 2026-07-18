# VISUALS — rsx-term design system

The complete visual language of the RSX trading terminal (Go / Bubble Tea +
Lipgloss). It is the terminal-native sibling of the dashboard's design system
(`rsx-playground/CLAUDE.md`) — same palette, same "colour = meaning" rule,
rendered in a TTY instead of a browser.

This file is the **rebuildable reference**: every colour and its meaning, every
glyph and its role and measured ink-coverage, the three nonlinear compressions
with exact boundaries, the ramps with their exact math, the axes, the trade +
persistence layers, and the degradation ladder. A competent dev with this file,
`SCREENS.md`, and `CLAUDE.md` — and no other context — can reproduce the look on
a GPU/bitmap renderer.

Source of truth is the code, cited `file:symbol` throughout. Colours live in
`ui/styles.go`; the glyph table and ramps in `ui/stream.go`; the compressions in
`book/heatmap.go` (why: `notes/compression.md`); the glyph calibration in
`notes/glyph-bank.md`.

## The one rule: colour is meaning, never decoration

A green thing is *live / long / bid / filled / positive*. A red thing is
*short / ask / down / rejected / negative*. Amber is *degraded / stale /
offline*. Violet is *heading / accent / overlay*. Nothing is coloured "to look
nice." Add a colour only when it maps to a new **meaning** — never a new shade
for its own sake.

Because the whole scheme leans on green-vs-red, and that pair is invisible under
deuteranopia/protanopia (~8% of men), `RSX_TUI_THEME=colorblind` (`styles.go`
`UseTheme`) swaps *only* the bid/ask pair to a colourblind-safe
blue(bid)/orange(ask); the rest of the palette is unchanged. Non-colour cues —
`B`/`S` tape glyphs, `LONG`/`SHORT` labels, bid-left/ask-right geometry, `▸`/`‹`
marks, the `▲`/`▼` own-order shapes — carry the same meaning without any colour.

## Palette — Ayam Cemani ("black iridescence")

A green-tinged near-black base with the bird's beetle-green + violet
feather-sheen. Hexes are verbatim-shared with the dashboard and spec 55
(`specs/2/55-terminal.md`), so the terminal and the web UI read as one product.
Every const is in `ui/styles.go`.

| Meaning | Const | Hex |
|---|---|---|
| live / long / bid / filled / positive | `ColorLive` = `ColorBid` | `#22f5a1` |
| short / ask / down / reject / negative | `ColorAsk` | `#f87171` |
| heading / badge / ⚡ accent | `ColorHeading` | `#bd83ff` |
| info / secondary accent | `ColorAccent` | `#a992ff` |
| overlay ring (confirm / trace / help) | `ColorRing` | `#7c3aed` |
| body text | `ColorText` | `#a9bcb2` |
| bright / focused / active statusline | `ColorTextBright` | `#e7eeea` |
| muted — labels / captions / help / idle | `ColorMuted` | `#586b62` |
| degraded / stale / offline | `ColorDegraded` | `#fbbf24` |
| panel background | `ColorPanelBg` | `#0d1712` |
| page background + ramp floor | `ColorPageBg` | `#040806` |
| panel border | `ColorBorder` | `#16211b` |

Colourblind theme (`UseTheme("colorblind")`) reassigns only:
`ColorLive`/`ColorBid` → `#2f9bff` (blue), `ColorAsk` → `#ff9e3d` (orange), and
rebuilds `StyleLive`/`StyleAsk`/`StyleArmed`.

### Derived colours (computed at render, not constants)

- **Size ramp** — `rampColor(tier)` = `blendHex(ColorPageBg, ColorLive,
  tier/sizeTiers)`: the page background linearly interpolated toward the live
  hue, tier 0 (`#040806`) → tier 4 (`#22f5a1`). ONE ramp for both book sides.
- **Sector tile** — `tile` (`ui/market.go`): `blendHex(ColorPageBg, hue,
  0.15 + 0.85*t*0.7)` where `hue` = `ColorBid` (up) or `ColorAsk` (down) and
  `t` = `moveTier/len(moveTiers)`. Foreground always `ColorTextBright`.
- `blendHex` (`ui/stream.go`) is a straight per-channel lerp of two `#rrggbb`.

## Style roles

Foreground pairings + the two panel frames, all in `ui/styles.go`:

- `StyleText` / `StyleTextBright` / `StyleMuted` — body, focused/active, and
  labels/dim.
- `StyleLive` / `StyleAsk` — positive/long/bid vs negative/short/ask figures.
- `StyleHeading` — section titles, the symbol badge, the ⚡ speed motif.
- `StyleAccent` — the secondary-accent violet: the sparkline, the own-order
  `◇`/`▸`, the microscope selected-row `▸`, the keycast chip, the mode-line tag.
- `StyleDegraded` — offline dot, the "no live book" row, an unpriceable uPnL,
  a stale-marketdata flag.
- `StyleDerived` — **dim + italic (muted + italic)**, always paired with a `~`
  prefix, for any client-computed value (mark=mid, uPnL off it) and for a
  legitimately-not-yet-real field (a pending latency leg, the dashed risk row).
  This is the terminal's tell that a number is a local *estimate* or *pending*,
  not exchange-authoritative data. Never render a derived value in a plain style.
- `StyleArmed` — bright text on the ask-red danger ground, bold: the persistent
  confirm-off (ARMED) banner. Its own meaning: "the guardrail is down."
- `PanelStyle` — the standard bordered panel: `NormalBorder` in `ColorBorder`;
  title is the first content line, in `StyleMuted`.
- `RingPanelStyle` — a `NormalBorder` in the violet `ColorRing`, for overlays
  that demand attention: the confirm preview, the F3 trace HUD, the `?` help.

## The glyph vocabulary

Two layers: the **calibrated streaming table** (`ui/stream.go` `glyphs`, one
data-driven struct so a re-measured ramp drops in without a render change) and
the **fixed structural glyphs** the DOM view and chrome use. Each glyph means
exactly one thing. `TestGlyphVocabularyDocumented` asserts every table glyph
appears in this file; `TestGlyphVocabularyExcludesBraille` guards against tofu.

### Streaming table (`glyphs`, calibrated per font)

| Field | Glyph(s) | Channel | Meaning |
|---|---|---|---|
| `countRamp` | ` ░▒▓█` | count density | resting ORDER COUNT: 0 · 1 (whale) · 2–3 · 4–7 · 8+ (wall) |
| `microRamp` | `▁▂▃▄▅▆▇█` | micro-bar | NOW-row exact live depth (even ~0.13/step ink ladder) |
| `tradeRamp` | `○◆●■` | trade magnitude | print size small → huge, drawn in the aggressor hue |
| `newsRamp` | `·►►‼` | news severity | routine · tagged · market-moving (amber) · critical (red, bold) |
| `persistent` | `▚` | persistence | long-standing liquidity (level held ≥ 30 s; L2 proxy, `book.AgeSource`) |
| `ownOrder` | `◇` | own order | your resting order, on the heatmap (accent) |
| `ownBuy` / `ownSell` | `▲` / `▼` | own order | your buy / sell, on the ruler (side by shape) |
| `cursor` | `┃` | cursor | the price cursor on the ruler (`f` fires here) |
| `touchTick` | `┼` | touch ticks | the two touch columns on the ruler |
| `rulerLine` | `─` | ruler | the fisheye baseline |
| `railIdle` | `│` | rail idle | news rail with nothing in the window |

The `newsRamp` deliberately repeats `►` at tiers 1 and 2 — the *hue* separates
them (`newsMarker`: tier 1 body text, tier 2 amber). Only four distinct runes,
four severities.

### Fixed structural glyphs (chrome + DOM view)

| Glyph | Where | Meaning |
|---|---|---|
| `▸` | heatmap microscope row rail; news feed / DOM orders cursor | selection / "this row is armed" (accent) |
| `┆` | BOOK tape rail | trade-feed column separator (muted) |
| `▊` | DOM ladder (`depthGlyph`) | per-level depth bar cell, scaled to deepest visible level |
| `█` | DOM imbalance bar, latency hop-bar | solid proportional-bar fill |
| `‹` | DOM ladder row | the last print's price row (bright) |
| `≡ = ≠ ·` | NEWS co-move row (`coMoveGlyph`) | together-ness: `≡` strong-with · `=` with · `·` independent · `≠` inverse |
| `●` | header / status bar link dot | `● live` (green) / `● offline` (amber) |
| `◀` `▶` | BOOK header axis | "bids ◀ … asks ▶" price-axis legend |
| `⚡` | speed strip / latency line | the round-trip speed motif (heading) |
| `⚠` | DOM ARMED banner | confirm-off warning (bright on ask-red) |
| `⏳` | LLM status | a turn is in flight, awaiting the assistant |
| `⌨` `␣` `↑↓←→` | keycast chip | the pressed key (`␣` = space, arrows as glyphs) |
| `░` | DOM risk row (`░░░░░░░░`) | unknown margin health — a dashed, unfilled bar (StyleDerived) |
| `~` | `~mark`, `~uPnL`, freeze/window labels | the derived/estimate marker (always with StyleDerived) |
| `B` / `S` | DOM tape | taker side without relying on colour |
| `_` | focused field / chat input | text cursor |
| `──` | NEWS / LLM section dividers | muted rules ("── context handed off ──") |
| `…` | overflow | withheld remainder (tiles, co-move, switcher, headline truncation) |

### Ramps that reuse eighth-blocks

`▁▂▃▄▅▆▇█` appears in two independent places: `glyphs.microRamp` (NOW-row depth)
and `sparkRamp` (`ui/sparkline.go`, the RTT sparkline). They are the same eight
runes declared twice — the one glyph family the glyph-bank measured as a clean
linear ladder, safe for a magnitude bar (`notes/glyph-bank.md`).

### Calibration (why these, not others)

Measured by the glyph-bank rasterizer against DejaVuSansMono (black-pixel
fraction; re-run per deployment font — `tools/glyphbank`):

- `░▒▓█` = **0.22 / 0.56 / 0.86 / 1.00** — steps 0.22/0.34/0.30/0.14, NOT even.
  So the shade family carries only the COARSE categorical count channel; fine
  intensity rides on the colour ramp.
- `○◆●■` = **0.16 / 0.26 / 0.40 / 0.51** — ascending-ink distinct *shapes*, so
  trade-flow never reads as resting liquidity.
- Eighth-blocks `▁▂▃▄▅▆▇█` = a clean ~0.13/step ladder (0.13 → 1.00), the one
  family safe for a linear bar; ink is bottom-weighted, so bars only.
- **Braille is excluded** — every codepoint renders as identical 0.23 tofu in
  DejaVuSansMono. `▚` (quadrant) is the sanctioned sub-cell mark instead.

## The three nonlinear compressions — exact boundaries

The heatmap fits a whole book + its history into a fixed character grid by
applying one idea per axis: replace a linear mapping with a nonlinear one that
magnifies what matters and compresses the periphery. Full rationale:
`notes/compression.md`; the state lives in `book/heatmap.go`.

### Time — a log cadence (vertical axis)

- Bottom = the **NOW row**: the current book, repainted every frame
  (`book.LiveFold`), never scrolls.
- Above it, a **live ring** of exact per-bin rows, one row per `binInterval` =
  **100 ms** (`ui/stream.go`).
- Above those, **far tiers**: each row aggregates an exponentially longer window
  on the fixed `book.farSpans` schedule — **10 s, 60 s, 120 s, 300 s, 600 s**,
  then **1 h** (`farSpanMax`) for every row past the schedule. `maxFarRows` caps
  the aggregated block at **12** so live detail keeps most of a tall terminal.
- Near rows carry the **exact** book (per level); far rows carry a
  **time-weighted** (ms-weighted) liquidity profile of the whole book — mean
  size/count per price bucket, dominant side, summed trade-flow.
- The right gutter (`gutterWidth` = **6**) labels each far row's horizon
  (`−10m`, `−1h`) and the NOW row `now`.

### Price — a fisheye (horizontal axis)

- Mid-centred, **bids left / asks right** of a spread gap; anchored in doubled
  price units (`bid+ask`) with **hysteresis**: the anchor re-centres only when
  the mid drifts beyond `hysteresisTicks` = **8** ticks (`book.updateAnchor`) —
  it does not jitter every tick.
- `book.FisheyeCol` maps price→column: the touch-most `linearTicks` = **8**
  ticks are 1 tick/cell (full resolution); beyond that a **triangular schedule**
  compresses ever-wider tick ranges into single columns — the k-th extra column
  spans k ticks (cumulative k(k+1)/2), computed in bounded closed form. So near
  the touch is sharp and the deep book aggregates into the edges.
- Rows store **price-space** profiles, not screen columns; the fold to columns
  (`foldCells`) happens at render time against the CURRENT anchor, so a
  re-anchor re-aligns the whole history and a resize preserves it.
- `book.FisheyePx` is the inverse (cursor placement): an aggregated deep column
  resolves to its **inner (touch-side)** price — the least-surprising choice.

### Size — a log colour ramp on a stable basis

- `sizeTier(size, refMax)` = `logTier(size, refMax, sizeTiers)` with
  `sizeTiers` = **4**: `int( log(size+1)/log(refMax+1) * 4 )`, floored at 1 for
  any nonzero size so a thin level never vanishes. Heavy-tailed sizes → log.
- `refMax` is a **stable basis** (`market.sizeBasis`, `stableBases`): it rises
  instantly to a new max but decays slowly (`foldBasis`), floored by the live
  frame — so the map never strobes as the biggest order comes and goes, and a
  fresh spike still registers between bin ticks.
- Sizes render as a handful of tiers, **never as an exact number** on the map
  (exact figures live only in the touch ladder and tape).

## The cell — the render stack

Each heatmap cell (`ui/stream.go` `cell`, `cellStr`) composites up to three
channels; **background = size (colour), glyph = shape**. Precedence, top wins:

1. **Trade override** (`tradeCellStr`) — if a print landed in the cell:
   `tradeRamp[tradeTier]` glyph in the **aggressor hue** (`aggressorStyle`: buy =
   live/bid, sell = ask), bold, over the resting-size background. `tradeTier` =
   `logTier(qty, tradeMax, 4)` floored at 1.
2. **Own order** (`ownCellStr`) — `◇` in accent, bold, over the size background.
3. **Book cell** — background = `rampColor(sizeTier)`; glyph =
   `countRamp[countTier]` (`countTier`: 0 / 1 / 2–3 / 4–7 / 8+ → 0..4);
   foreground = `rampColor(sizeTier+1)` for a faint relief. If the level has
   held ≥ `persistThreshold` = **30 s** the glyph becomes `▚` (persistent).
4. Empty cell → a space.

The **NOW row** (`microCellStr`) is the one variant: instead of the shade ramp
it draws `microRamp[idx]` where `idx` is **linear** — `size/refMax * 8` — an
exact miniature histogram of the live book (trade override and `◇` still apply).

## The axes, ladders, and status regions

Described by what each draws; exact per-screen positions are in `SCREENS.md`.

- **BOOK header** (`streamHeader`) — reverse-video `RSX <name>` badge (heading
  bg), `● live`/`● offline` link dot, and the `◀ bids  mid <px>  asks ▶` axis
  legend. `mid` dashes (`—`) until anchored.
- **Ruler** (`rulerLine`) — the interaction axis under the NOW row: `│` gutter +
  a baseline of `─`, the two touch columns as `┼`, own orders as `▲`/`▼`
  (accent), the price cursor as `┃` (bright). Game order entry (`h`/`l`) moves
  the cursor; `f` fires at it.
- **Touch ladder** (`touchLadderLine`) — up to three **exact** levels per side
  at display precision (`px×qty`), nearest the touch first; `—` when a side is
  empty. Ask line in ask hue, bid line in live hue. This is where exact numbers
  live, the map being tiers-only.
- **Tape rail** (`tapeRail`) — a right-edge trade feed, `┆` separator + aggressor
  glyph (`tradeRamp`) + exact price, one print per body row, newest at top.
  `tapeRailWidth` = **14**. The same prints appear both here and at their level
  on the map.
- **News rail** (`railChar`) — the leftmost column: `newsRamp` severity marker
  when an enabled source has a headline in the row's window, else `│`.
- **DOM ladder** (`ladderRow`) — a static price axis (recentres on drift only):
  `mark bidBar bidQty <price> askQty askBar`. `▊` depth bars scaled to the
  deepest visible level, bid grows left / ask right converging on the spread;
  price coloured by the resting side; own rows `▸`, last print `‹`. Below it an
  `imbalanceBar` (`█` green|red split, 20 wide) with the bid share %.

## Mode line — "what does my next keystroke do"

`modeLine` (`ui/stream.go`), the persistent strip under the header on every
streaming screen: the **screen tag** (reverse-video accent block:
`BOOK`/`NEWS`/`LLM`), the active **venue** (heading), the `RO`/`PO` **modifier
toggles** (`StyleArmed` red when ON, muted when off — always visible so the
trader knows what mode fires), the armed **size preset** (`size N [n]`), the
active **watchlist** (`list <name>`), plus capture-mode prompts: the `x` symbol
switcher buffer with candidates, the `F9` venue picker list. Orders fire on one
key in the streaming terminal, so the mode line is a safety surface.

## Latency strip (DOM speed strip + BOOK latency line)

- `⚡ RTT <total>` (`viewSpeed`), the number amber past `slaWarnNs` = 50 µs, red
  past `slaCritNs` = 100 µs — a live health light, not just a readout.
- Split `= net + internal + engine`; a leg the live server doesn't stamp yet
  renders `·· pending` in `StyleDerived` (never a bare dash, never a fabricated
  number — `legValue`).
- `hopBar` — a proportional stacked `█` bar of the *known* legs, each its own
  hue (net = heading, internal = accent, engine = live), 18 cells.
- Rolling `p50 · p99 · best` + a `sparkline` (accent) of the recent RTT window,
  plus a `· md <age>` / amber `· md stale` marketdata-freshness tag.

## State → style

| State | How it looks |
|---|---|
| order link up / down | `● live` green / `● offline` amber |
| marketdata down or empty book | amber `no live book — market-data stream down` row |
| value is a client estimate | `~` prefix + dim italic (`StyleDerived`) |
| field has no server source | dashed `—` (never a fabricated number) |
| unpriceable uPnL (no mid) | amber `—  (needs live book)` |
| pending latency leg / risk row | `·· pending` / `░░░░` in `StyleDerived` |
| positive / negative figure | green / red |
| needs-attention overlay | violet ring (`RingPanelStyle`) |
| confirm-off armed | red `⚠ ARMED` banner (`StyleArmed`), persistent |
| far heatmap row (aggregate) | dimmed, gutter-labelled `−10s`…`−1h`, never "a book" |
| microscope row selected | rail `▸` in accent; footer shows the honest window label |

## Degradation ladder

`detectMode` (`ui/stream.go`): `RSX_TERM_COLOR=plain|shade|true` forces; else
`NO_COLOR` / `TERM` (`dumb`/empty → plain) / `COLORTERM` (`truecolor`/`24bit`) /
`256` decide. lipgloss further maps RGB down to the real terminal palette, so
`modeTrue` is safe to request on a 256-colour terminal.

- **`modeTrue`** — the full encoding: `rampColor` background + all glyph
  channels + trade/own/persistence hues.
- **`modeShade`** (16-colour) — no ramp background; glyph *density* (`countRamp`
  indexed by size tier) carries size, trades keep aggressor hues, `◇` keeps
  accent. The NOW micro-bar keeps its glyph in plain text.
- **`modePlain`** (colourless) — glyphs only, zero escapes; size reads off the
  shade/micro glyph, trades off the `○◆●■` shape, own off `◇`.

The theme swap (`colorblind`) is orthogonal to the mode tier — it only remaps
the bid/ask pair, applying in `modeShade`/`modeTrue`.

## Rules when you touch the UI

- New colour only for a new **meaning** — otherwise reuse the palette.
- One accent per element; don't stack a coloured bg + border + text.
- Any client-derived number gets `StyleDerived` + `~`. No exceptions — this is
  what keeps the terminal honest about what's real vs estimated.
- A missing server field is a dash, never a plausible-looking zero.
- New glyph → add it to `glyphs` (streaming) and to the legend above, and
  re-run `tools/glyphbank` if the target font changed. The doc-sync test fails
  otherwise.
