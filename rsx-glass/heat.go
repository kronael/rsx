package glass

import (
	"fmt"
	"image/color"
	"math"
	"strconv"
	"strings"
	"time"

	"rsx-core/book"
	"rsx-core/wire"
)

// The Grid builder. It REUSES rsx-term's model wholesale — the fisheye, the
// log-time heatmap ring, the live fold, the cascade all live in rsx-term/book
// (a pure, clock-free, UI-agnostic package) and are imported, never
// re-implemented. What lives here is only the ENCODING: model → resolved
// colour + glyph per cell, faithful to rsx-term/VISUALS.md. Per the GPU-rebuild
// prune (rsx-term/notes/element-inventory.md) the encoding is exactly the layer
// a desktop renderer owns — the channel survives, the terminal glyph does not.
//
// v1 is a read-only heatmap viewer. The pruned element set (vs rsx-term):
//   KEEP  size ramp (bg), count density, persistence ▚, trade layer ○◆●■,
//         NOW-row live bars, honesty markers — and ~, the touch ladder, gutter
//         time labels, the fisheye ruler, the link dot.
//   CUT   news rail, own-order glyphs / cursor (no input in v1), keycast, the
//         classic DOM view, the truecolour→plain degradation ladder (a GPU is
//         always full-colour), the tape rail.

// gutterWidth is the right time-axis label column ("  −10m"). Matches rsx-term.
const gutterWidth = 6

// sizeTiers is the number of non-empty steps on the size ramp (VISUALS.md).
const sizeTiers = 4

// persistThreshold is how long a level must hold before it reads as
// long-standing liquidity (the ▚ mark). Matches rsx-term.
const persistThreshold = 30 * time.Second

// glyph vocabulary, pruned to v1 (rsx-term/ui/stream.go glyphs). Each rune
// means exactly one thing; the software atlas draws the non-ASCII ones as
// primitives (real rectangles/shapes), which is what the GPU layer is for.
var (
	countRamp = []rune{' ', '░', '▒', '▓', '█'} // resting order-count density
	tradeRamp = []rune{'○', '◆', '●', '■'}      // trade magnitude, aggressor-hued
	microRamp = []rune{'▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'}
)

const persistGlyph = '▚'

// Scene is everything the builder needs for one frame — the reused book state
// plus the current live book and the frame's clock. It is plain data the caller
// (a feed adapter) fills; the builder never touches a network or a wall clock.
type Scene struct {
	Symbol    string
	Connected bool
	Heat      *book.Heatmap
	Bids      []wire.Level
	Asks      []wire.Level
	Trades    []book.TapeEntry
	Ages      book.AgeSource
	FromNs    int64
	ToNs      int64
}

// cellAgg is one fisheye column's accumulated liquidity + trade flow for a row.
type cellAgg struct {
	size      int64
	count     int32
	ageNs     int64
	tradeQty  int64
	tradeSide wire.Side
}

// Fixed screen layout, top→bottom: header(1) + body + ruler(1) + footer(2).
// The NOW row is the last body row; far rows pin to the top, live rows sit just
// above NOW, blanks pad the middle before the ring fills. The caller Configures
// the heatmap so far + liveCap == body-1 and it fits exactly.
func Compose(g *Grid, s Scene) {
	g.Clear()
	if g.W < gutterWidth+4 || g.H < 5 {
		return
	}
	heatW := g.W - gutterWidth
	nowY := g.H - 4 // last body row
	rulerY := g.H - 3
	askY, bidY := g.H-2, g.H-1

	now := book.LiveFold(s.Bids, s.Asks, s.Trades, s.Ages, s.FromNs, s.ToNs)
	refMax, tradeMax := bases(s.Heat, now)
	anchor := s.Heat.Anchor()
	tick := s.Heat.Tick()

	composeHeader(g, s, heatW)

	rows := s.Heat.Rows() // far (top) then live, oldest→newest
	far := s.Heat.FarRows()
	if far > len(rows) {
		far = len(rows)
	}
	y := 1
	for i := 0; i < far && y < nowY; i++ { // far rows pinned to the top
		composeRow(g, y, heatW, rows[i], anchor, tick, refMax, tradeMax, false)
		y++
	}
	liveRows := rows[far:]
	for pad := (nowY - y) - len(liveRows); pad > 0 && y < nowY; pad-- {
		y++ // blank middle before the live ring fills
	}
	for _, row := range liveRows {
		if y >= nowY {
			break
		}
		composeRow(g, y, heatW, row, anchor, tick, refMax, tradeMax, false)
		y++
	}
	composeRow(g, nowY, heatW, now, anchor, tick, refMax, tradeMax, true)

	composeRuler(g, rulerY, heatW)
	composeFooter(g, s, askY, bidY)
}

// composeHeader draws the top strip: the RSX badge, the link dot, and the
// anchored mid (— until the book anchors).
func composeHeader(g *Grid, s Scene, heatW int) {
	x := g.text(0, 0, " RSX  "+s.Symbol+" ", Page, Heading)
	x = g.text(x+1, 0, "●", linkColor(s.Connected), color.RGBA{})
	if s.Connected {
		g.text(x+1, 0, " live", Live, color.RGBA{})
	} else {
		g.text(x+1, 0, " offline", Degraded, color.RGBA{})
	}
	mid := "—"
	midCol := Muted
	if p := s.Heat.MidPx(); p > 0 {
		mid = fmtPx(p)
		midCol = Text
	}
	g.text(heatW-len("mid "+mid), 0, "mid "+mid, midCol, color.RGBA{})
}

// composeRow draws one heatmap body line: the fisheye cells and the gutter's
// time label. isNow uses the live micro-bar glyphs.
func composeRow(g *Grid, y, heatW int, row book.Row, anchor, tick, refMax, tradeMax int64, isNow bool) {
	cells := foldRow(row, anchor, tick, heatW)
	for x := 0; x < heatW; x++ {
		if c := g.At(x, y); c != nil {
			*c = bookCell(cells[x], refMax, tradeMax, isNow)
		}
	}
	label := ""
	col := Muted
	if isNow {
		label, col = "now", TextBright
	} else if row.Span > 0 {
		label = "−" + fmtSpan(row.Span)
	}
	if label != "" {
		g.text(g.W-len(label), y, label, col, color.RGBA{})
	}
}

// composeRuler draws the fisheye baseline under the NOW row: — across, with the
// two touch columns ticked ┼.
func composeRuler(g *Grid, y, heatW int) {
	for x := 0; x < heatW; x++ {
		if c := g.At(x, y); c != nil {
			c.Glyph = '─'
			c.Fg = Muted
		}
	}
	half := heatW / 2
	for _, x := range []int{half - 1, half} {
		if x >= 0 {
			if c := g.At(x, y); c != nil {
				c.Glyph = '┼'
				c.Fg = Muted
			}
		}
	}
}

// composeFooter draws the two ladder lines: up to three exact levels per side
// (the only exact numbers on the screen), — when a side is empty, plus the
// derived ~mark. This is where the honesty markers live.
func composeFooter(g *Grid, s Scene, askY, bidY int) {
	line := func(y int, label, lad string, col color.RGBA) int {
		x := g.text(0, y, " "+label+" ", Muted, color.RGBA{})
		if lad == "" {
			return g.text(x, y, "—", Muted, color.RGBA{})
		}
		return g.text(x, y, lad, col, color.RGBA{})
	}
	line(askY, "ask", ladder(s.Asks), Ask)
	line(bidY, "bid", ladder(s.Bids), Live)
	mark := "~mark —"
	if p := s.Heat.MidPx(); p > 0 {
		mark = "~mark " + fmtPx(p)
	}
	g.text(g.W-len(mark), askY, mark, Muted, color.RGBA{})
}

// bookCell resolves one fisheye column into a Cell (VISUALS.md precedence:
// trade over own over book; the NOW row draws live micro-bars).
func bookCell(c cellAgg, refMax, tradeMax int64, isNow bool) Cell {
	sz := sizeTier(c.size, refMax)
	bg := rampColor(sz)
	if c.tradeQty > 0 {
		return Cell{Bg: bg, Fg: aggressor(c.tradeSide), Glyph: tradeRamp[tradeTier(c.tradeQty, tradeMax)-1]}
	}
	if isNow {
		if c.size <= 0 {
			return Cell{Bg: Page, Fg: Text, Glyph: ' '}
		}
		return Cell{Bg: Page, Fg: rampColor(clampTier(sz + 1)), Glyph: microRamp[microIdx(c.size, refMax)]}
	}
	if sz == 0 {
		return Cell{Bg: Page, Fg: Text, Glyph: ' '}
	}
	glyph := countRamp[countTier(c.count)]
	if c.ageNs >= int64(persistThreshold) {
		glyph = persistGlyph
	}
	return Cell{Bg: bg, Fg: rampColor(clampTier(sz + 1)), Glyph: glyph}
}

// foldRow maps a row's price-space levels and trades onto fisheye columns
// against the current anchor — via book.FisheyeCol (the fisheye is reused, not
// re-implemented; this only buckets the results).
func foldRow(row book.Row, anchor, tick int64, width int) []cellAgg {
	cells := make([]cellAgg, width)
	for _, l := range row.Levels {
		col, ok := book.FisheyeCol(l.Px, anchor, tick, width)
		if !ok {
			continue
		}
		c := &cells[col]
		c.size += l.Size
		c.count += l.Count
		if l.AgeNs > c.ageNs {
			c.ageNs = l.AgeNs
		}
	}
	for _, tr := range row.Trades {
		col, ok := book.FisheyeCol(tr.Px, anchor, tick, width)
		if !ok {
			continue
		}
		c := &cells[col]
		if tr.Qty >= c.tradeQty {
			c.tradeSide = tr.Side
		}
		c.tradeQty += tr.Qty
	}
	return cells
}

// bases returns the size and trade ramp references: the largest resting size
// and print across all rows plus the live frame. rsx-term uses a slow-decaying
// stable basis to stop the map strobing; the viewer takes the per-frame max
// (deterministic, no clock) — the stable basis is a v2 refinement on the seam.
func bases(h *book.Heatmap, now book.Row) (refMax, tradeMax int64) {
	scan := func(row book.Row) {
		for _, l := range row.Levels {
			if l.Size > refMax {
				refMax = l.Size
			}
		}
		for _, t := range row.Trades {
			if t.Qty > tradeMax {
				tradeMax = t.Qty
			}
		}
	}
	for _, row := range h.Rows() {
		scan(row)
	}
	scan(now)
	return refMax, tradeMax
}

// ladder renders up to three exact levels of one side, nearest the touch first,
// or "" when the side is empty.
func ladder(levels []wire.Level) string {
	if len(levels) == 0 {
		return ""
	}
	n := len(levels)
	if n > 3 {
		n = 3
	}
	parts := make([]string, 0, n)
	for _, l := range levels[:n] {
		parts = append(parts, fmtPx(l.Px)+"x"+fmtQty(l.Qty))
	}
	return strings.Join(parts, " ")
}

func linkColor(connected bool) color.RGBA {
	if connected {
		return Live
	}
	return Degraded
}

func aggressor(side wire.Side) color.RGBA {
	if side == wire.Sell {
		return Ask
	}
	return Live
}

// rampColor is the single sequential size ramp: the page background blended
// toward the live hue, dim (tier 0) → full (tier sizeTiers). One ramp, both
// sides — side is position, colour is magnitude (VISUALS.md).
func rampColor(tier int) color.RGBA { return blend(Page, Live, float64(tier)/float64(sizeTiers)) }

func clampTier(t int) int {
	if t > sizeTiers {
		return sizeTiers
	}
	return t
}

func sizeTier(size, refMax int64) int { return logTier(size, refMax, sizeTiers) }

func tradeTier(qty, refMax int64) int {
	t := logTier(qty, refMax, len(tradeRamp))
	if t < 1 {
		t = 1
	}
	return t
}

// microIdx maps a NOW-row size to a linear eighth-block bar 0..len-1.
func microIdx(size, refMax int64) int {
	steps := len(microRamp)
	if refMax <= 0 {
		return 0
	}
	idx := int(float64(size) / float64(refMax) * float64(steps))
	if idx < 1 {
		idx = 1
	}
	if idx > steps {
		idx = steps
	}
	return idx - 1
}

// countTier maps a resting order count to a density-glyph step 0..4 (VISUALS.md).
func countTier(count int32) int {
	switch {
	case count <= 0:
		return 0
	case count == 1:
		return 1
	case count <= 3:
		return 2
	case count <= 7:
		return 3
	default:
		return 4
	}
}

// logTier is the shared log scale: 0 for empty, else 1..steps (VISUALS.md).
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

func fmtSpan(d time.Duration) string {
	if d >= time.Hour {
		return fmt.Sprintf("%dh", int(d/time.Hour))
	}
	if d >= time.Minute {
		return fmt.Sprintf("%dm", int(d/time.Minute))
	}
	return fmt.Sprintf("%ds", int(d/time.Second))
}

// fmtPx / fmtQty render raw i64 fixed-point at a fixed display precision. The
// viewer has no per-symbol config, so it shows the raw integer — exact, honest,
// and enough to read the touch. A live adapter can supply real tick/lot scaling.
func fmtPx(px int64) string   { return strconv.FormatInt(px, 10) }
func fmtQty(qty int64) string { return strconv.FormatInt(qty, 10) }
