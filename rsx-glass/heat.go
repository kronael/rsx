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
//   KEEP  size ramp (bg), count density, persistence ▚, trade layer ▵▴△▲/▿▾▽▼
//         (side by shape + aggressor hue), NOW-row live bars, honesty markers
//         — and ~, the touch ladder, gutter time labels + session-open markers,
//         the fisheye ruler, the link dot.
//   CUT   news rail, own-order glyphs / cursor (no input in v1), keycast, the
//         classic DOM view, the truecolour→plain degradation ladder (a GPU is
//         always full-colour), the tape rail.

// gutterWidth is the right time-axis label column ("  −24h"). Matches rsx-term.
const gutterWidth = 6

// sessionOpens is the active session-open schedule the gutter labels its coarse
// (>= 1h) far rows with (book.SessionOpenLabel) — the UTC day roll plus the
// Asia/Europe/US cash opens by default. UseSessionOpens rebinds it once at
// startup (RSX_SESSION_OPENS), the same set-once-before-Compose seam as UseTheme.
var sessionOpens = book.DefaultSessionOpens()

// UseSessionOpens rebinds the session-open schedule. Call once at startup,
// before the first Compose (the render reads the var per frame).
func UseSessionOpens(opens []book.SessionOpen) {
	if len(opens) == 0 {
		opens = book.DefaultSessionOpens()
	}
	sessionOpens = opens
}

// sizeTiers is the number of non-empty steps on the size ramp (VISUALS.md).
const sizeTiers = 4

// farRows is the number of aggregated far tiers above the live ring (the
// fixed layout Compose assumes: header 1 + body + ruler 1 + footer 2).
const farRows = 6

// heatmapCap turns a grid height into the (live, far) capacities to
// Configure a book.Heatmap with, so it always fits Compose's row math exactly
// (Compose's body/ruler/footer split assumes the same height). NewFeed and
// NewSynth both size their heatmap this way and must agree on the split.
func heatmapCap(h int) (live, far int) {
	body := h - 4 // header(1) + body + ruler(1) + footer(2)
	live = body - 1 - farRows
	if live < 1 {
		live = 1
	}
	return live, farRows
}

// persistThreshold is how long a level must hold before it reads as
// long-standing liquidity (the ▚ mark). Matches rsx-term.
const persistThreshold = 30 * time.Second

// glyph vocabulary, pruned to v1 (rsx-term/ui/stream.go glyphs). Each rune
// means exactly one thing; the software atlas draws the non-ASCII ones as
// primitives (real rectangles/shapes), which is what the GPU layer is for.
var (
	countRamp     = []rune{' ', '░', '▒', '▓', '█'} // resting order-count density
	tradeBuyRamp  = []rune{'▵', '▴', '△', '▲'}      // buy magnitude, up = lifts the offer
	tradeSellRamp = []rune{'▿', '▾', '▽', '▼'}      // sell magnitude, down = hits the bid
	microRamp     = []rune{'▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'}
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
	// RefBasis/TradeBasis are the slow-decaying ramp references (book.FoldBasis),
	// folded per bin by the model. bases() floors them by the live frame so the
	// map never renormalizes downward frame-to-frame — the anti-strobe rule.
	RefBasis   int64
	TradeBasis int64
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
	refMax, tradeMax := bases(s.RefBasis, s.TradeBasis, now)
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
	} else if s, ok := row.SessionOpenLabel(sessionOpens); ok {
		label, col = s, Heading // a coarse far row's window crosses a session open
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
		// Side is a SHAPE (up = buy, down = sell) with the aggressor hue as
		// the redundant channel, over the overlay background — the size ramp
		// capped at overlayBgTier so the ink always clears 3:1 (uncapped, a
		// buy print on a tier-4 wall was fg==bg — invisible). rsx-term rule.
		return Cell{
			Bg:    rampColor(overlayTier(sz)),
			Fg:    aggressor(c.tradeSide),
			Glyph: tradeGlyph(c.tradeSide, tradeTier(c.tradeQty, tradeMax)),
		}
	}
	if isNow {
		if c.size <= 0 {
			return Cell{Bg: Page, Fg: Text, Glyph: ' '}
		}
		return Cell{Bg: Page, Fg: glyphFg(Page), Glyph: microRamp[microIdx(c.size, refMax)]}
	}
	if sz == 0 {
		return Cell{Bg: Page, Fg: Text, Glyph: ' '}
	}
	glyph := countRamp[countTier(c.count)]
	if c.ageNs >= int64(persistThreshold) {
		glyph = persistGlyph
	}
	return Cell{Bg: bg, Fg: glyphFg(bg), Glyph: glyph}
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

// bases returns the size and trade ramp references: the slow-decaying stable
// bases (book.FoldBasis, folded per bin by the model) floored by the live
// frame's max, so a fresh spike still registers between bins while the view
// never renormalizes downward frame-to-frame. This mirrors rsx-term's
// stableBases; scanning every visible row instead re-brightened the whole field
// whenever one large historical order entered or left (the autoscale strobe).
func bases(refBasis, tradeBasis int64, now book.Row) (refMax, tradeMax int64) {
	refMax, tradeMax = refBasis, tradeBasis
	for _, l := range now.Levels {
		if l.Size > refMax {
			refMax = l.Size
		}
	}
	for _, t := range now.Trades {
		if t.Qty > tradeMax {
			tradeMax = t.Qty
		}
	}
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

// glyphFg picks the higher-contrast of the two reserved glyph inks — TextBright
// on a dark background, Page on a bright one — so a density / persistence / live
// glyph is always legible on its size-ramp cell and is NEVER fg==bg. Because the
// two inks sit near the luminance extremes, the pick clears 3:1 against every
// ramp background (asserted by the palette test in heat_test.go). This is the
// reserved-contrast-pair rule: fg is a palette colour, never "the background one
// tier up" (which collapsed to fg==bg at the top tier — the invisible wall glyph).
func glyphFg(bg color.RGBA) color.RGBA {
	if contrastRatio(TextBright, bg) >= contrastRatio(Page, bg) {
		return TextBright
	}
	return Page
}

// contrastRatio is the WCAG 2.1 contrast ratio between two opaque colours (1..21).
func contrastRatio(a, b color.RGBA) float64 {
	la, lb := relLuma(a), relLuma(b)
	if la < lb {
		la, lb = lb, la
	}
	return (la + 0.05) / (lb + 0.05)
}

// relLuma is the WCAG 2.1 relative luminance of an sRGB colour (0..1).
func relLuma(c color.RGBA) float64 {
	lin := func(v uint8) float64 {
		s := float64(v) / 255
		if s <= 0.03928 {
			return s / 12.92
		}
		return math.Pow((s+0.055)/1.055, 2.4)
	}
	return 0.2126*lin(c.R) + 0.7152*lin(c.G) + 0.0722*lin(c.B)
}

func sizeTier(size, refMax int64) int { return logTier(size, refMax, sizeTiers) }

func tradeTier(qty, refMax int64) int {
	t := logTier(qty, refMax, len(tradeBuyRamp))
	if t < 1 {
		t = 1
	}
	return t
}

// tradeGlyph picks the side's shape ladder (rsx-term VISUALS.md): buys point
// up, sells point down — side survives colour-vision loss; hue is redundant.
func tradeGlyph(side wire.Side, tier int) rune {
	if side == wire.Sell {
		return tradeSellRamp[tier-1]
	}
	return tradeBuyRamp[tier-1]
}

// overlayBgTier is the highest size-ramp tier a trade overlay may use as its
// background (see overlayTier).
const overlayBgTier = 1

// overlayTier caps a trade overlay's background at overlayBgTier so both
// aggressor inks keep >= 3:1 against it in both themes (asserted by
// TestGlyphContrast). Measured worst pair after the cap: 3.91:1.
func overlayTier(sz int) int {
	if sz > overlayBgTier {
		return overlayBgTier
	}
	return sz
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
