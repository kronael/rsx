---
status: draft
---

# 03 — Heatmap encoding

The heatmap is the BOOK screen's body: the whole order book and its recent
history drawn in one fixed grid, time running up on a log cadence, price
running across on a fisheye, each cell's colour giving resting size and its
shape giving order count, persistence, own orders and trade prints. This file
owns the model-to-cell encoding; the screen around it is
[04-screen-book.md](04-screen-book.md), the palette and shapes are
[02-design-system.md](02-design-system.md).

Source pin: `rsx-term/…` and `rsx-core/…` citations are at
`5d175b6ff04755e02fcca5ebbb918b01b929c23b`. `rsx-core` survives the deletion of
`rsx-term`; the folds below stay importable. The render-side rules (cell
precedence, ramps, gutter) lived in `rsx-term/ui/stream.go` and are mirrored in
`rsx-glass/heat.go`; the excerpts here are the reference.

## Requirements

| ID | Requirement | Source |
|---|---|---|
| HM-01 | Time runs up: the bottom row is NOW (the current book, repainted every frame, never scrolls); above it a ring of exact live bins; above those far rows on a fixed multi-scale schedule. A fixed row count spans now → days. | [code rsx-core/book/heatmap.go:10-35], [doc rsx-term/notes/compression.md] |
| HM-02 | Live bins are 100 ms. | [code rsx-term/ui/stream.go:115-116] |
| HM-03 | Far rows hold time-weighted (ms-weighted) liquidity profiles of the whole book — mean size and count per price bucket, dominant side, summed trade flow — and are labelled as windows, never as a book. | [code rsx-core/book/heatmap.go:296-328], [founder 2026-10-21 "Far rows show aggregated liquidity labelled as such"] |
| HM-04 | The far schedule is 3 s, 60 s, 1 h, then 24 h for every further row; at most 12 far rows; far rows take a third of the body. | [code rsx-core/book/heatmap.go:69-94], [code rsx-term/ui/stream.go:118-120], [code rsx-term/ui/update.go:743] |
| HM-05 | Coarse far rows (span ≥ 1 h) whose wall-clock window contains a session open carry that open's label instead of the horizon; four opens, configurable, priority-ordered; default `day=00:00,Asia=01:00,EU=08:00,US=14:30` UTC. A malformed config is a fatal startup error. | [code rsx-core/book/session.go], [code rsx-term/main.go:716-726], [founder 2026-10-21 "a session-open reference with four configurable opens"] |
| HM-06 | Price runs across a mid-centred fisheye: bids left, asks right of a spread gap; 1 tick per column at the touch, a triangular schedule beyond; the deepest levels clamp into the edge columns so the whole book lands on screen. | [code rsx-core/book/heatmap.go:400-521] |
| HM-07 | The axis anchors on the mid in doubled price units and re-centres only when the mid drifts more than 8 ticks; rows store price-space profiles and fold to columns at render against the current anchor, so a re-anchor re-aligns all history and a resize preserves it. | [code rsx-core/book/heatmap.go:378-398], [code rsx-term/ui/stream.go:282-310] |
| HM-08 | Size maps to colour on ONE sequential ramp for both sides (`blend(PageBg, Live, tier/4)`), log-scaled against a stable basis that rises instantly and decays 1/256 per bin; any nonzero size is at least tier 1. Side is position, never hue. | [code rsx-term/ui/stream.go:195-260], [code rsx-core/book/basis.go] |
| HM-09 | Order count maps to a five-step shape channel (0, 1, 2–3, 4–7, 8+); a whale (one big order) and a wall (many small) read differently. | [code rsx-term/ui/stream.go:238-253], [code rsx-term/ui/stream_test.go TestCellCountChannel] |
| HM-10 | A level held ≥ 30 s carries the persistence mark (client L2 proxy behind `book.AgeSource`; an exchange order-age feed replaces it with no UI change). | [code rsx-term/ui/stream.go:110-113], [code rsx-core/book/persist.go] |
| HM-11 | Trades are a co-equal layer: a print draws at its price column in the row whose window it fell in, as a side-shaped magnitude marker in the aggressor hue over the overlay-capped ground; the same prints also list on the tape rail. | [code rsx-term/ui/stream.go:348-365], [code rsx-term/ui/stream.go:659-687] |
| HM-12 | Your resting orders on the active symbol mark the NOW row at their live column every frame (and the ruler); they never drift up with history. | [code rsx-term/ui/stream.go:695-706] |
| HM-13 | Exact numbers never appear on the map; the touch ladder and tape carry them. | [doc rsx-term/VISUALS.md "Size"] |
| HM-14 | The model is pure and clock-free: the caller supplies bin timestamps; the composer is a pure function of the model and the frame time. | [code rsx-core/book/heatmap.go:27-28], [doc rsx-core/SPEC.md §2] |

## Time axis (log cadence)

```go
// rsx-core/book/heatmap.go:74-94 (exact)
var farSpans = []time.Duration{
	3 * time.Second,
	60 * time.Second,
	time.Hour,
	24 * time.Hour,
}

const farSpanMax = 24 * time.Hour

func FarSpan(i int) time.Duration {
	if i < 0 {
		i = 0
	}
	if i < len(farSpans) {
		return farSpans[i]
	}
	return farSpanMax
}
```

**Why this schedule.** The founder asked (2026-07-23 06:07): "i need 3x shoeter
the shortest window on screen and then next window should be 20x as kuch time
and the third 100x more... e.g. tje second is minites and the other is an hour
and the agg is days reference open". The proposal 100 ms live / 3 s / 60 s /
~1 h / 1 day, with "literal x100 of the minute = 100 min ≈ 1.7h; I'd round to
1h — your call", was approved with "ofc like go" and "also uodate tg glass in
this direction too!" (2026-07-23 07:02). So 1 h, not 100 min, is the decided
third window; the 2026-10-21 wording in `rsx-term/CLAUDE.md` ("windows 20×
and 100× longer (minutes, an hour …)") names the same decision. The
superseded schedules and their dates are in the ledger,
[15-founder-requirements.md](15-founder-requirements.md) F-BOOK-03.

The cascade: when a live bin falls off the ring it folds into far tier 0,
weighted by its own milliseconds; a tier that has covered its span seals into
a row, promotes into the next tier and restarts. A bin wider than a tier's
remaining capacity (a scheduler stall) is split across full-span fills, with
its trades attributed to the first chunk only, so no time is dropped and no
print is double-counted [code rsx-core/book/heatmap.go:198-244].

```go
// rsx-core/book/heatmap.go:301-328 (exact) — a far row is a time-weighted mean
func (t *tier) snapshot() Row {
	row := Row{FromNs: t.fromNs, ToNs: t.toNs, Span: t.span}
	if t.coveredMs == 0 {
		return row
	}
	for px, acc := range t.levels {
		size := acc.sizeMs / t.coveredMs
		if size == 0 {
			continue // present too briefly to register at this resolution
		}
		count := int32(acc.countMs / t.coveredMs)
		if count < 1 {
			count = 1
		}
		side := int8(-1)
		if acc.askMs > acc.bidMs {
			side = 1
		}
		row.Levels = append(row.Levels, LevelSample{Px: px, Size: size, Count: count, Side: side})
	}
	for px, qty := range t.buys {
		row.Trades = append(row.Trades, TradeSample{Px: px, Qty: qty, Side: wire.Buy})
	}
	for px, qty := range t.sells {
		row.Trades = append(row.Trades, TradeSample{Px: px, Qty: qty, Side: wire.Sell})
	}
	return row
}
```

Row order for display: `Heatmap.Rows()` returns far tiers farthest first, then
the live ring oldest → newest; the NOW row is `LiveFold` of the current book
and the trades since the last bin, built per frame and not part of `Rows()`
[code rsx-core/book/heatmap.go:330-373].

Grid sizing from the window [code rsx-term/ui/update.go:737-765]:

```go
// rsx-term/ui/update.go:755-765 (exact)
func streamDims(width, height int) (int, int) {
	w := width - 1 - gutterWidth - tapeRailWidth
	if w%2 != 0 {
		w--
	}
	rows := height - (2 + 1 + 5)
	if rows > 72 {
		rows = 72
	}
	return w, rows
}
```

Horizontal budget: news rail 1 + heat columns (forced even so the mid splits
the axis) + time gutter 6 + tape rail 14. Vertical: header 2 + body
(far + live + NOW, capped at 72) + ruler 1 + footer 5. Far rows =
`clamp((rows-1)/3, 0, 12)`, live capacity = `rows - 1 - far`. Below 8 columns
or 3 rows the screen reports `streaming heatmap — sizing…` instead of drawing
[code rsx-term/ui/update.go:737-748], [code rsx-term/ui/stream.go:516-518].
Before the live ring fills, blank rows pad between the far block and the live
block so live rows stay bottom-anchored and the grid stays fixed
[code rsx-term/ui/stream.go:533-535].

Session-open test: the first daily UTC instance of an open at or after the
row's start lies before its end.

```go
// rsx-core/book/session.go:65-75 (exact)
func (session SessionOpen) inWindow(fromNs, toNs int64) bool {
	if toNs <= fromNs || fromNs < 0 {
		return false
	}
	ofDay := int64(session.Hour)*int64(time.Hour) + int64(session.Minute)*int64(time.Minute)
	open := (fromNs/dayNs)*dayNs + ofDay
	if open < fromNs {
		open += dayNs
	}
	return open < toNs
}
```

## Price axis (fisheye)

```go
// rsx-core/book/heatmap.go:406-432 (exact)
func FisheyeCol(px, anchor, tick int64, width int) (int, bool) {
	if anchor == 0 || width < 2 {
		return 0, false
	}
	if tick <= 0 {
		tick = 1
	}
	half := width / 2
	num := 2*px - anchor
	if num == 0 {
		return 0, false
	}
	if num > 0 { // ask side, right of centre
		ticks := (num + 2*tick - 1) / (2 * tick) // ceil to whole ticks
		col := half + fisheyeOffset(ticks) - 1
		if col > width-1 {
			col = width - 1
		}
		return col, true
	}
	ticks := (-num + 2*tick - 1) / (2 * tick)
	col := half - fisheyeOffset(ticks)
	if col < 0 {
		col = 0
	}
	return col, true
}
```

```go
// rsx-core/book/heatmap.go:483-521 (exact; closed form, bounded for absurd prices)
func fisheyeOffset(ticks int64) int {
	if ticks < 1 {
		ticks = 1
	}
	if ticks <= linearTicks {
		return int(ticks)
	}
	return linearTicks + compressTicks(ticks-linearTicks)
}

func compressTicks(extra int64) int {
	if extra <= 1 {
		return 1
	}
	n := int64(math.Ceil((math.Sqrt(8*float64(extra)+1) - 1) / 2))
	if n < 1 {
		n = 1
	}
	for n > 1 && (n-1)*n/2 >= extra {
		n--
	}
	for n*(n+1)/2 < extra {
		n++
	}
	return int(n)
}
```

`linearTicks = 8`, `hysteresisTicks = 8` [code rsx-core/book/heatmap.go:127-133].
Resulting column widths from the touch outward (computed from the code):

| Column offset from mid | Ticks covered |
|---|---|
| 1 … 9 | 1 each |
| 10 | 2 |
| 11 | 3 |
| 12 | 4 |
| k ≥ 10 | k − 8 |

`FisheyePx` is the inverse for cursor placement: an aggregated deep column
resolves to its inner (touch-side) price, the least surprising choice; it works
in doubled units so an odd anchor (1-tick spread) does not drop a half tick
[code rsx-core/book/heatmap.go:434-461]. A price exactly on the mid has no
column (the spread gap). A cursor placed through a deep column lands on the
inner edge: requirement HM-15, from [doc rsx-term/notes/honesty.md rule 3].

## Size ramp and the stable basis

```go
// rsx-term/ui/stream.go:198-236 (exact)
func sizeTier(size, refMax int64) int {
	return logTier(size, refMax, sizeTiers)
}

func tradeTier(qty, refMax int64) int {
	t := logTier(qty, refMax, len(glyphs.tradeBuyRamp))
	if t < 1 {
		t = 1
	}
	return t
}

func tradeGlyph(side wire.Side, tier int) rune {
	if side == wire.Sell {
		return glyphs.tradeSellRamp[tier-1]
	}
	return glyphs.tradeBuyRamp[tier-1]
}

func logTier(v, refMax int64, steps int) int {
	if v <= 0 || refMax <= 0 {
		return 0
	}
	ratio := math.Log(float64(v)+1) / math.Log(float64(refMax)+1)
	t := int(ratio * float64(steps))
	if t < 1 {
		t = 1
	}
	if t > steps {
		t = steps
	}
	return t
}
```

```go
// rsx-core/book/basis.go:7-22 (exact)
const BasisDecayShift = 8

func FoldBasis(basis, observed int64) int64 {
	if observed > basis {
		return observed
	}
	basis -= basis >> BasisDecayShift
	if basis < 1 {
		basis = 1
	}
	return basis
}
```

Per bin, each market folds its size basis against the largest visible level
and its trade basis against the largest print of the bin
[code rsx-term/ui/update.go:703-725]; per frame the references are the bases
floored by the NOW row's own maxima, so a fresh spike registers between bins
and the field never renormalises downward frame to frame
[code rsx-term/ui/stream.go:553-570]. Half-life ≈ 18 s at 100 ms bins
[code rsx-core/book/basis.go:3-6].

## Cell precedence (the render stack)

A heat cell is the fisheye fold of one row's price-space profile: sizes and
counts summed per column, the oldest age kept, trade quantity summed with the
side of the largest single print [code rsx-term/ui/stream.go:282-310].

```go
// rsx-term/ui/stream.go:316-346 (exact)
func cellStr(c cell, refMax, tradeMax int64, mode renderMode) string {
	sz := sizeTier(c.size, refMax)
	if c.tradeQty > 0 {
		return tradeCellStr(c, sz, tradeMax, mode)
	}
	if c.own {
		return ownCellStr(sz, mode)
	}
	if sz == 0 {
		return " "
	}
	glyph := glyphs.countRamp[countTier(c.count)]
	if c.ageNs >= int64(persistThreshold) {
		glyph = glyphs.persistent
	}
	switch mode {
	case modePlain:
		return string(glyphs.countRamp[sz])
	case modeShade:
		g := glyphs.countRamp[sz]
		if c.ageNs >= int64(persistThreshold) {
			g = glyphs.persistent
		}
		return StyleText.Render(string(g))
	default:
		return lipgloss.NewStyle().
			Foreground(glyphInk(rampColor(sz))).
			Background(rampColor(sz)).
			Render(string(glyph))
	}
}
```

Precedence, top wins:

1. **Trade print** — side-shaped magnitude marker (`tradeTier` of the summed
   quantity against the trade reference), aggressor hue, bold, on the ramp
   capped at tier 1.
2. **Own order** — the own-order mark in Accent, bold, on the capped ramp
   (NOW row only; see HM-12).
3. **Book** — ground = `rampColor(sizeTier)`; shape = count step, or the
   persistence mark when the column's oldest level is ≥ 30 s; ink = the
   reserved glyph ink for that ground.
4. **Empty** — page ground, no mark.

The NOW row differs only in step 3: instead of the count step it draws a
linear bar `idx = clamp(size/refMax * 8, 1, 8)` on the page ground in the
reserved ink — an exact miniature histogram of the live book; persistence,
trades and own orders still override [code rsx-term/ui/stream.go:382-414].

Far rows are drawn dimmed relative to live rows [doc rsx-term/VISUALS.md
"State → style"]. At the pin the terminal's `renderRow` applies the same cell
function to far and live rows, and the dimming is carried by the time-weighted
means being smaller plus the gutter label (no separate dim style is applied in
`stream.go`). Glass MUST make far rows visibly distinct from live rows
(HM-16, from the founder rule "aggregated liquidity labelled as such"), and the
choice of treatment (reduced opacity, a band, a divider) is OD-02.

## Gutter, rails and ruler

| Element | Rule | Source |
|---|---|---|
| time gutter (right, 6 wide) | NOW row: `now` in TextBright; far row: session label in Heading if HM-05 fires, else `−<span>` in Muted; live rows blank | [code rsx-term/ui/stream.go:761-776] |
| news rail (col 0) | worst-severity marker of the headlines whose time falls in the row's window, else the idle rail; the microscope's selected row shows `▸` in Accent | [code rsx-term/ui/stream.go:715-759] |
| tape rail (right, 14 wide) | one print per body row, newest top: separator, side-shaped magnitude marker, exact price (truncated to fit), aggressor hue; empty rows show only the separator | [code rsx-term/ui/stream.go:659-687] |
| ruler (under NOW) | baseline across, the two centre columns as touch ticks, own orders as up/down marks in Accent, the cursor in TextBright (cursor wins a shared column) | [code rsx-term/ui/stream.go:789-830] |

News severity grading per row [code rsx-term/ui/stream.go:740-759]: tier 0
Muted `·`, tier 1 Text `►`, tier 2 Degraded `►`, tier 3 Ask bold `‼`.

## Microscope (freeze, not replay)

The microscope is a row cursor over the rows the heatmap already holds
(`Rows()`: far + live, not NOW). It creates no replay buffer; a freeze copies
one held row. A far row freezes as `~<span> window (aggregate, not a restored
book)`; a live row as `exact ~100ms bin` [code rsx-term/ui/news_view.go:485-493].
A symbol or venue switch resets the cursor (it indexed the old book)
[code rsx-term/ui/update.go:355-361, 367-388]. Key grammar:
[10-keymap.md](10-keymap.md); handoff: [07-screen-assistant.md](07-screen-assistant.md).

## What glass already has

`rsx-glass/heat.go` mirrors: the size ramp, count step, persistence, the
per-side trade ramps with the overlay cap, the NOW bars, the reserved glyph
ink, the gutter with session labels, the ruler ticks, the touch ladder and the
stable basis. It cuts (v1): the news rail, own-order marks and cursor, the
tape rail, keycast [code rsx-glass/heat.go:18-30]. Its layout is fixed at
72×24 with 6 far rows, not derived from the window
[code rsx-glass/cmd/glass/main.go:43-48], [code rsx-glass/heat.go:59-73].
Status of every element: [19-glass-status.md](19-glass-status.md).
