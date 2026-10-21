---
status: draft
---

# 02 — Design system

The design system is the set of colours, shapes, type and honesty markers every
glass screen draws with, stated as meanings and measurable rules so it applies
to pixels as well as to terminal cells. The heatmap's own encoding (ramps,
compressions, cell precedence) is in [03-heatmap.md](03-heatmap.md); per-screen
placement is in the screen files listed in [index.md](index.md).

Source pin: every `rsx-term/…` and `rsx-core/…` citation is at commit
`5d175b6ff04755e02fcca5ebbb918b01b929c23b` (recover with
`git show 5d175b6ff04755e02fcca5ebbb918b01b929c23b:<path>`). Source tags:
`[code …]` a file at the pin, `[doc …]` a doc at the pin, `[founder <date>]` a
founder statement (ledger: [15-founder-requirements.md](15-founder-requirements.md)),
`[BUGS <id>]` an entry in the repo `BUGS.md`.

## The one rule: colour is meaning

| ID | Requirement | Source |
|---|---|---|
| DS-01 | Every colour maps to exactly one meaning; a new colour is added only for a new meaning, never to decorate. | [doc rsx-term/VISUALS.md "The one rule"], [code rsx-term/ui/styles.go:5-9] |
| DS-02 | Green = live / long / bid / filled / positive. Red = short / ask / down / rejected / negative. Amber = degraded / stale / offline. Violet = heading / accent / overlay. | [doc rsx-term/VISUALS.md], [doc specs/2/55-terminal.md "Principles"] |
| DS-03 | Every meaning carried by colour is also carried by a visible non-colour channel (shape, glyph, word, or position); hue alone never carries a meaning (WCAG 1.4.1). | [doc rsx-term/notes/trade-side.md], [doc rsx-glass/CLAUDE.md "Meaning = colour + a VISIBLE redundant channel"] |
| DS-04 | The hexes are shared verbatim with the dashboard (`rsx-playground`) and spec 55; a change to one is a coordinated palette revision across all three. | [doc rsx-term/VISUALS.md "Known design debt"] |

## Palette (semantic tokens)

[code rsx-term/ui/styles.go:15-51]; mirrored in `rsx-glass/grid.go` (subset).

| Token | Hex | Meaning |
|---|---|---|
| `Live` = `Bid` | `#22f5a1` | live / long / bid / filled / positive; the size-ramp ceiling |
| `Ask` | `#f87171` | short / ask / down / reject / negative |
| `Heading` | `#bd83ff` | heading, symbol badge, the ⚡ speed motif, session-open labels |
| `Accent` | `#a992ff` | secondary accent: own order, sparkline, screen tag, selection cursor, keycast |
| `Ring` | `#7c3aed` | overlay border (confirm preview, trace HUD, help) |
| `Text` | `#a9bcb2` | body text |
| `TextBright` | `#e7eeea` | focused field, active status line, reserved light glyph ink |
| `Muted` | `#586b62` | labels, captions, help, idle, derived values (with `~`) |
| `Degraded` | `#fbbf24` | stale / offline / degraded |
| `PanelBg` | `#0d1712` | panel background |
| `PageBg` | `#040806` | page background, size-ramp floor, reserved dark glyph ink |
| `Border` | `#16211b` | panel border |

**Colourblind theme** (`RSX_TUI_THEME=colorblind`): reassigns only
`Live`/`Bid` → `#2f9bff` (blue) and `Ask` → `#ff9e3d` (orange); every derived
colour (ramp, tiles, aggressor hues, armed ground) follows. Selected once at
startup before the first frame. [code rsx-term/ui/styles.go:108-124],
[code rsx-glass/grid.go UseTheme]

```go
// rsx-term/ui/styles.go:108-117 (exact)
func UseTheme(name string) {
	if name == "colorblind" {
		ColorLive = lipgloss.Color("#2f9bff") // bid / long / positive — blue
		ColorBid = lipgloss.Color("#2f9bff")
		ColorAsk = lipgloss.Color("#ff9e3d") // ask / short / negative — orange
	} else {
		ColorLive = lipgloss.Color("#22f5a1") // Ayam Cemani green / red
		ColorBid = lipgloss.Color("#22f5a1")
		ColorAsk = lipgloss.Color("#f87171")
	}
```

### Token tiers

| Tier | Holds | Rule |
|---|---|---|
| primitive | ramp stops, spacing and type scales | never read by a screen directly |
| semantic | the palette above, by meaning; the two reserved glyph inks | the only tier a theme rebinds (bid/ask only) |
| component | `cell.bg`, `cell.glyph.fg`, `tile.bg`, `banner.bg` … | resolved from semantic tokens by the composer, never by the renderer |

[doc rsx-glass/CLAUDE.md "Three token tiers"]. In glass, colours are resolved
when the `Grid` is built, so a renderer backend can never change the palette
[code rsx-glass/grid.go Cell doc].

### Derived colours (computed, not constants)

`blend(a, b, t)` is a straight per-channel linear interpolation of two sRGB
colours with integer truncation, `t` clamped to [0,1].

```go
// rsx-term/ui/stream.go:466-480 (exact)
func blendHex(a, b string, t float64) string {
	if t <= 0 {
		return a
	}
	if t >= 1 {
		return b
	}
	ar, ag, ab := hexRGB(a)
	br, bg, bb := hexRGB(b)
	r := ar + int(float64(br-ar)*t)
	g := ag + int(float64(bg-ag)*t)
	bl := ab + int(float64(bb-ab)*t)
	return fmt.Sprintf("#%02x%02x%02x", r, g, bl)
}
```

| Derived colour | Formula | Default theme | Colourblind theme |
|---|---|---|---|
| size ramp tier 0..4 | `blend(PageBg, Live, tier/4)` | `#040806` `#0b432c` `#137e53` `#1ab97a` `#22f5a1` | `#040806` `#0e2c44` `#195182` `#2476c0` `#2f9bff` |
| sector tile, up, tier 0..4 | `blend(PageBg, Bid, 0.15 + 0.85*(tier/4)*0.7)` | `#082b1d` `#0c4e34` `#11724b` `#159562` `#1ab879` | `#0a1e2b` `#103350` `#174975` `#1d5f9a` `#2475bf` |
| sector tile, down, tier 0..4 | `blend(PageBg, Ask, same t)` | `#281716` `#4c2725` `#713635` `#954645` `#b95655` | `#291e0e` `#4e3416` `#744b1e` `#996126` `#be772e` |

Values computed from the hexes and formulas at the pin
[code rsx-term/ui/stream.go:258-260], [code rsx-term/ui/news_view.go:138-140].

## Contrast rules

| ID | Requirement | Source |
|---|---|---|
| DS-10 | Every ink a heatmap cell can carry clears 3:1 (WCAG 2.1, 1.4.11) on every background it can land on, in both themes. | [code rsx-term/ui/palette_test.go TestPaletteContrast], [doc rsx-glass/CLAUDE.md] |
| DS-11 | Glyph inks are **reserved**: a density / persistence / micro-bar glyph is drawn in `TextBright` or `PageBg`, whichever has the higher contrast on that cell's background. NEVER "the ramp one tier up" (that collapsed to fg == bg on the brightest walls). | [code rsx-term/ui/stream.go:432-441], [doc rsx-term/VISUALS.md "The cell"] |
| DS-12 | An overlay (trade print, own order) sits on the size ramp capped at tier 1, so both aggressor hues and the accent clear 3:1 in both themes; a print outranks the wall it lands on. | [code rsx-term/ui/stream.go:416-430], [doc rsx-term/notes/trade-side.md] |
| DS-13 | Text inks on the page: every palette text ink clears 3:1 on `PageBg`. | [code rsx-term/ui/palette_test.go] |
| DS-14 | The colour gate is a matrix test over ink × background × theme. Image goldens that strip or ignore colour can never see a colour regression; the matrix is the test that sees colour. | [doc rsx-term/VISUALS.md "Rules when you touch the UI"] |

```go
// rsx-term/ui/stream.go:436-464 (exact; WCAG 2.1 relative luminance and ratio)
func glyphInk(bg lipgloss.Color) lipgloss.Color {
	if contrastHex(string(ColorTextBright), string(bg)) >= contrastHex(string(ColorPageBg), string(bg)) {
		return ColorTextBright
	}
	return ColorPageBg
}

func contrastHex(a, b string) float64 {
	la, lb := relLuma(a), relLuma(b)
	if la < lb {
		la, lb = lb, la
	}
	return (la + 0.05) / (lb + 0.05)
}

func relLuma(h string) float64 {
	r, g, b := hexRGB(h)
	lin := func(v int) float64 {
		s := float64(v) / 255
		if s <= 0.03928 {
			return s / 12.92
		}
		return math.Pow((s+0.055)/1.055, 2.4)
	}
	return 0.2126*lin(r) + 0.7152*lin(g) + 0.0722*lin(b)
}
```

Measured matrix (computed at the pin with the functions above):

| Pair | Default | Colourblind |
|---|---|---|
| reserved ink on ramp tier 0 / 1 / 2 / 3 / 4 | 17.09 / 9.61 / 4.31 / 7.93 / 14.05 | 17.09 / 12.18 / 7.01 / 4.25 / 6.96 |
| buy / sell / own overlay on tier 1 | 7.90 / 4.10 / 4.45 | 4.96 / 6.98 / 5.64 |
| `Muted` on page | 3.54 | 3.54 |
| `Ring` on page | 3.53 | 3.53 |
| armed banner `TextBright` on `Ask` | 2.35 | 1.74 |

The rsx-term docs quote 3.97:1 (worst reserved-ink pair) and 3.91:1 (worst
overlay pair) [doc rsx-term/VISUALS.md], [code rsx-term/ui/stream.go:416-420];
the recomputation above does not reproduce those two figures. The gate is the
matrix test, not either quoted number.

### Known contrast debt (deliberate, owned upstream)

- `Muted #586b62` passes WCAG 3:1 on the page (3.54:1) but sits at APCA
  Lc ≈ 23, under the Lc 30 any-text floor. It carries labels, help, the time
  gutter and every `~` derived value. A bump is a coordinated palette revision
  with the dashboard and spec 55, never a glass-only fork. [doc rsx-term/VISUALS.md
  "Known design debt"], [doc rsx-glass/SPEC.md "Muted ink is APCA-sub-floor"]
- The ARMED banner (bright text on the ask red) is 2.35:1 by WCAG, Lc ≈ 46 by
  APCA; it is redundant (bold + ⚠ + the red field). A page-ink-on-red variant
  (7.28:1 default, 9.79:1 colourblind) clears both models; choosing it is a
  founder decision. In the colourblind theme the current banner is 1.74:1.
  [doc rsx-term/VISUALS.md]

## Style roles

[code rsx-term/ui/styles.go:55-99]

| Role | Ink / ground | Used for |
|---|---|---|
| Text / TextBright / Muted | body / focused / labels | all text |
| Live / Ask | positive, long, bid / negative, short, ask figures | numbers with a sign or a side |
| Heading | section titles, the badge ground, ⚡, session-open labels | |
| Accent | screen tag ground, own order, selection `▸`, sparkline, keycast | |
| Degraded | offline dot, "no live book", unpriceable uPnL, stale md | |
| Derived | Muted + italic, ALWAYS with a leading `~` | every client-computed value (mark = mid, uPnL off it) and every legitimately not-yet-real field (a pending latency leg, the dashed risk row) |
| Armed | TextBright, bold, on the Ask ground | the persistent confirm-off banner only: "the guardrail is down" |
| Panel | `Border` stroke, title first line in Muted | standard panel |
| Ring panel | `Ring` stroke | overlays that demand attention: confirm preview, trace HUD, help |

DS-20: a derived value is never drawn in a plain style; `~` and the derived
style always travel together [doc rsx-term/VISUALS.md "StyleDerived"].
DS-21: one accent per element — never a coloured ground plus a coloured border
plus coloured text on one element [doc rsx-term/VISUALS.md "Rules"].

## Shape language

Each shape means exactly one thing. Up / down means buy / sell everywhere: on
the trade markers, on the own-order ruler marks, and nowhere else.
[doc rsx-term/notes/trade-side.md], [doc rsx-term/VISUALS.md "The glyph vocabulary"]

The streaming glyph table is one data-driven struct, so a re-measured ramp
drops in without a render change [code rsx-term/ui/stream.go:61-105]:

```go
// rsx-term/ui/stream.go:91-105 (exact)
var glyphs = glyphSet{
	countRamp:     []rune{' ', '░', '▒', '▓', '█'},
	tradeBuyRamp:  []rune{'▵', '▴', '△', '▲'},
	tradeSellRamp: []rune{'▿', '▾', '▽', '▼'},
	microRamp:     []rune{'▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'},
	newsRamp:      []rune{'·', '►', '►', '‼'},
	persistent:    '▚',
	ownOrder:      '◇',
	ownBuy:        '▲',
	ownSell:       '▼',
	cursor:        '┃',
	touchTick:     '┼',
	rulerLine:     '─',
	railIdle:      '│',
}
```

Channel → terminal glyph → glass primitive. The channel is essential; the
terminal glyph is the encoding a cell grid forced. Glass keeps every channel
and MAY replace the encoding by a primitive with the same ordering and the
same meaning. Tags are from the prune list
[doc rsx-term/notes/element-inventory.md]; a "cuttable" or "mergeable" tag is a
candidate the founder decides, not a decision ([16-open-decisions.md](16-open-decisions.md)).

| Channel | Terminal glyph | Meaning | Glass primitive (allowed) | Tag |
|---|---|---|---|---|
| order count | ` ░▒▓█` | 0 · 1 (whale) · 2–3 · 4–7 · 8+ (wall) | five-step texture or stipple density, or an integer label at high zoom | channel essential, glyph cuttable |
| NOW-row live depth | `▁▂▃▄▅▆▇█` | exact live size, linear to the reference | a real bar of height ∝ size | cuttable |
| buy print magnitude | `▵▴△▲` | small → huge, up = aggressor lifts the offer | an up-pointing triangle, area by tier, aggressor hue | essential |
| sell print magnitude | `▿▾▽▼` | small → huge, down = aggressor hits the bid | a down-pointing triangle, area by tier | essential |
| news severity | `·►►‼` | routine · tagged · market-moving (amber) · critical (red, bold) | a rail marker; four distinct shapes | `►` repeats: redundant |
| persistence | `▚` | level held ≥ 30 s (client L2 proxy) | a hatch over the cell | essential |
| own order on the map | `◇` | your resting order | one outlined-cell treatment in Accent | mergeable with ruler marks |
| own order on the ruler | `▲` / `▼` | your buy / sell | same own-order treatment, side by shape | mergeable |
| price cursor | `┃` | the cursor `f` fires at | a vertical rule in TextBright | essential |
| touch columns | `┼` | the two touch columns | axis ticks | essential |
| ruler baseline | `─` | the fisheye axis | an axis line | cuttable |
| idle news rail | `│` | no headline in this row's window | blank | redundant |
| selection | `▸` | microscope row, feed / user / order cursor, DOM own-order row | a highlight bar in Accent | mergeable |
| tape rail separator | `┆` | trade feed column rule | spacing | cuttable |
| DOM depth bar | `▊` | per-level depth ∝ deepest visible level | a real bar | cuttable |
| solid fill | `█` | imbalance bar, latency hop bar | a real bar | cuttable |
| last print row | `‹` | the last trade's price row | a row highlight | essential channel |
| co-movement | `≡ = · ≠` | strong-with · with · independent · inverse | four distinct marks | essential |
| link state | `●` | `● live` green / `● offline` amber, always with the word | a dot plus the word | essential |
| axis legend | `◀ bids` `asks ▶` | side labels | text | `◀ ▶` redundant |
| speed motif | `⚡` | round-trip latency | an icon | essential |
| guardrail down | `⚠` | ARMED banner | an icon | essential |
| in flight | `⏳` | an assistant turn awaits a reply | a spinner state, never content | essential |
| keycast | `⌨` `␣` `↑↓←→` | the key just pressed | a chip | cuttable (founder: keep, see 15) |
| unknown margin health | `░░░░░░░░` | an empty, dashed meter | an empty meter outline | essential marker |
| estimate | `~` | client-derived value | the same character | essential |
| tape side | `B` / `S` | taker side without colour | a side word or the triangle | redundant |
| text cursor | `_` | focused field, chat caret | a caret | essential |
| divider / overflow | `──` / `…` | section rule / withheld remainder | spacing / truncation with `…` | `…` essential |

### Glyph calibration (terminal-only evidence for the ordering)

Measured ink coverage in DejaVuSansMono by the glyph-bank rasteriser
[doc rsx-term/notes/glyph-bank.md], [code rsx-term/tools/glyphbank/glyphbank.py]:

| Family | Coverage | Consequence |
|---|---|---|
| `░▒▓█` | 0.22 / 0.56 / 0.86 / 1.00 (uneven steps 0.22, 0.34, 0.30, 0.14) | carries only the coarse categorical count channel; fine intensity rides on the colour ramp |
| `▵▴△▲` | 0.079 / 0.110 / 0.144 / 0.263 | ascending ink; side is the shape |
| `▿▾▽▼` | 0.083 / 0.110 / 0.169 / 0.262 | mirrored ladder |
| `▁…█` | ~0.13 per step, 0.13 → 1.00 | the one linear family; bottom-weighted, bars only |
| braille U+2800–28FF | 0.23 for every codepoint (tofu) | excluded; a test forbids it [code rsx-term/ui/glyphs_test.go] |

Glass draws the non-ASCII glyphs as primitives in its atlas, so the font
coverage problem does not exist there [code rsx-glass/atlas/atlas.go package doc];
the ordering above (small → huge, sparse → solid) is what glass keeps.

## Typography and grid

| ID | Requirement | Source |
|---|---|---|
| DS-30 | The screen is a fixed grid of monospaced cells repainted in place every frame. History lives in the model's ring, never in scrollback; a frame is exactly the window height. | [code rsx-term/ui/stream.go:18-28], [code rsx-term/ui/stream_test.go TestStreamFixedGrid], [founder 2026-10-21 "the grid repaints in place; it is never a scrolling log"] |
| DS-31 | Glass rasterises ASCII from an embedded monospace font (Go Mono 14 pt, 72 dpi, full hinting) and draws the heatmap glyphs as primitives; the cell size comes from the font metrics. | [code rsx-glass/atlas/atlas.go New] |
| DS-32 | Numbers right-align in columns sized to their widest member (floor 6 for prices, 3 for quantities), so a column never goes ragged. | [code rsx-term/ui/format.go:88-98 strWidth], [code rsx-term/ui/view.go:227-242] |
| DS-33 | Over-long text truncates with `…`; a line never wraps into the next grid row (a wrap would shift the fixed grid). | [code rsx-term/ui/news_view.go:324-327], [code rsx-term/ui/keycast.go:174-188] |
| DS-34 | A positive signed figure carries a leading `+`; a negative one `-`. | [code rsx-term/ui/stream.go:925-943] |

### Number formatting (exact rules)

All values are fixed-point i64 on the wire; conversion to human decimals
happens only at display, with integer arithmetic, never floats.

```go
// rsx-term/ui/format.go:11-41 (exact)
func fmtNs(ns int64) string {
	if ns < 1_000 {
		return fmt.Sprintf("%d ns", ns)
	}
	if ns < 1_000_000 {
		return fmt.Sprintf("%d.%d µs", ns/1_000, (ns%1_000)/100)
	}
	return fmt.Sprintf("%d.%02d ms", ns/1_000_000, (ns%1_000_000)/10_000)
}

func fmtDec(raw int64, dec int) string {
	if dec <= 0 {
		return fmt.Sprintf("%d", raw)
	}
	neg := raw < 0
	if neg {
		raw = -raw
	}
	scale := int64(1)
	for i := 0; i < dec; i++ {
		scale *= 10
	}
	s := fmt.Sprintf("%d.%0*d", raw/scale, dec, raw%scale)
	if neg {
		s = "-" + s
	}
	return s
}
```

| Quantity | Rule | Source |
|---|---|---|
| price | `fmtDec(px, price_decimals)` of the active instrument | [code rsx-term/ui/model.go:122] |
| quantity | `fmtDec(qty, qty_decimals)` | [code rsx-term/ui/model.go:123] |
| money (notional, uPnL) | `fmtDec(raw / 10^qty_decimals, price_decimals)` — the quote's precision, truncating toward zero | [code rsx-term/ui/model.go:173-186] |
| latency | `fmtNs`: `340 ns`, `9.6 µs`, `1.28 ms` | [code rsx-term/ui/format.go:11-21] |
| wall-clock age ≥ 1 s | Go `Duration` rounded to 10 ms (`5.05s`) | [code rsx-term/ui/view.go:637-642] |
| time-axis span | `3s`, `1m`, `1h`, `24h` (`fmtSpan`) | [code rsx-term/ui/stream.go:778-787] |
| headline / handoff time | local `HH:MM:SS` | [code rsx-term/ui/news_view.go:319] |
| prompt time | UTC `2006-01-02 15:04:05 UTC` | [code rsx-term/assistant/prompt.go:62] |
| move | basis points shown as `+N.NN%`, sign from the bp sign | [code rsx-term/ui/news_view.go:118-133] |
| px×qty pair | `<px>×<qty>`; up to three levels joined by two spaces | [code rsx-term/ui/stream.go:896-910] |

## Honesty rules

The product's cardinal sin is a plausible-but-wrong number. These rules bind
every screen, every overlay, the assistant handoff and the MCP surface.
[doc rsx-term/notes/honesty.md], [doc rsx-term/CLAUDE.md "Keeper invariants"],
[founder 2026-10-21 "a latency is measured or it is `—`"]

| ID | Rule | Marker |
|---|---|---|
| H-1 | Dash the unknown: a field with no source shows `—`, never a plausible zero. | `—` in Muted |
| H-2 | Label the estimate: a client-derived value is prefixed `~`, drawn in the derived style, and names its basis where it fits (`~mark 0.010002 (mid)`). | `~` + derived style |
| H-3 | Aggregate ≠ exact: far heatmap rows are time-weighted windows, dimmed and labelled with their window, never dressed up as a book; a freeze of one is labelled `~<span> window (aggregate, not a restored book)`. | gutter label + microscope label |
| H-4 | Hard-block, never soft-warn: an order over the fat-finger caps is refused outright with `BLOCKED: …`; no dismissable dialog exists. ARMED removes the confirm, never the guard. | status `BLOCKED: …` |
| H-5 | Withhold on overflow: position and uPnL fold with checked i64; a fill that would overflow is rejected (state unchanged), a uPnL that would overflow is unavailable, a notional that would overflow dashes. | `—` |
| H-6 | Reject malformed input at the boundary: an unknown aggressor side is skipped, never coerced to buy; a level with an unparseable price or a non-positive size is dropped. | none (input skipped) |
| H-7 | A pending leg reads as pending: an unstamped latency leg shows `·· pending` in the derived style — never a bare dash, never a number. | `·· pending` |
| H-8 | Only received text renders as assistant content; waiting is a `⏳` status marker, failure is an error row `assistant unreachable — <err>`. | `⏳`, `err` row |
| H-9 | Read-only says so: an order on a venue with no order path is blocked with the reason `<venue> is read-only here (market data only)`. | status `BLOCKED: …` |
| H-10 | A capture-mode key is shown bare in the keycast; labelling a typed character with the verb it would otherwise fire is a lie. | bare key chip |
| H-11 | A latency with no real timestamp (frame `ts_ns == 0`) is not measured: it stays unknown rather than showing a multi-decade age; a negative age from clock skew clamps to 0. | `no frame yet` / `—` |
| H-12 | A raw protocol code is never the only thing a user sees: reason numbers are decoded to their names. | [BUGS TERM-FAILED-REASON-NOT-DECODED] |

### State → style

[doc rsx-term/VISUALS.md "State → style"]

| State | Drawn as |
|---|---|
| order link up / down | `● live` Live / `● offline` Degraded |
| marketdata down or empty book | Degraded row `no live book — market-data stream down` |
| client estimate | `~` + derived style |
| field with no server source | `—` |
| unpriceable uPnL (no mid) | Degraded `~uPnL —` + Muted `(needs live book)` |
| pending latency leg / risk row | `·· pending` / `░░░░░░░░` in the derived style |
| positive / negative figure | Live / Ask |
| overlay needing attention | Ring border |
| confirm off | Armed banner, persistent |
| far heatmap row | dimmed, gutter `−3s` … `−24h` |
| far row window crosses a session open | gutter shows the open's label in Heading |
| microscope row selected | rail `▸` in Accent; footer shows the honest window label |

## Degradation (terminal-only)

The terminal picked a render tier from the environment: `RSX_TERM_COLOR=plain|
shade|true`, else `NO_COLOR` / `TERM` / `COLORTERM`
[code rsx-term/ui/stream.go:166-193]. `shade` dropped the ramp ground and
carried size on the density glyph; `plain` emitted glyphs only. Glass is
always full colour and does not implement the ladder; it keeps the colourblind
swap, and it keeps the property the ladder protected: **side and magnitude of
a print stay readable with colour removed** (the shape channel), which glass
verifies by rendering a desaturated frame in a test
([17-acceptance.md](17-acceptance.md) AC-DS-04). See
[18-terminal-mapping.md](18-terminal-mapping.md).
