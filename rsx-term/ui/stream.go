package ui

import (
	"fmt"
	"math"
	"os"
	"strconv"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	"rsx-term/book"
	"rsx-term/wire"
)

// Streaming "text Bookmap" (RSX_TERM_STREAM=1). The screen is a FIXED grid the
// model repaints in place every frame (Bubble Tea altscreen line-diffs it —
// history lives in the heatmap's ring, never in terminal scrollback):
//
//	header                          symbol · link · mid
//	far rows   (top)                log-time windows: hours … 10m 5m 2m 1m 10s
//	live rows                       one row per ~100ms bin
//	NOW row    (bottom)             the current book, repainted every frame
//	ruler                           fisheye axis: cursor ┃, your orders ▲▼
//	footer                          touch ladder · position · latency · legend
//
// TIME is multi-resolution (book.Heatmap): the bottom rows are live and roll
// fast; each far row aggregates an exponentially longer window (book.FarSpan)
// so the top barely moves and a fixed row count spans now→hours. The right
// gutter labels each far row's horizon.
//
// PRICE is a mid-centred fisheye (book.FisheyeCol): bids left, asks right,
// 1 tick/cell at the touch, deep levels aggregating — so every row reads both
// near-touch concentration (centre, fine) and whole-book depth (edges,
// aggregated). Far rows lean on the whole-book reading.
//
// Each cell carries up to three channels:
//   - COLOUR (background) — resting SIZE on a SINGLE sequential ramp
//     (dim → bright), identical for both sides. Side is POSITION (left/right
//     of the mid gap), never colour.
//   - GLYPH — shape is a channel of its own (see glyphs): density ramp ░▒▓█ =
//     order count; ▚ = long-standing liquidity; ◉ = your resting order;
//     ▁▂▃▄▅▆▇█ micro-bars = the NOW row's exact live depth.
//   - TRADE-FLOW — a co-equal layer over the book: prints render in the
//     AGGRESSOR's hue with a magnitude ramp · • ◆ █ (how big) at their price
//     column (where), over the resting-size background, so liquidity and
//     executed flow read together.
//
// DEGRADES by terminal colour (detectMode): full ramp on 256/true-colour;
// glyph-density + trade hues on 16-colour; pure glyphs when colourless.

// glyphSet is the terminal's glyph vocabulary — ONE data-driven table so a
// calibrated ramp (measured ink-coverage ordering from the glyph-bank tool)
// can drop in by swapping these values, with no render change. Keep it small
// and principled: each glyph means exactly one thing (legend in VISUALS.md).
type glyphSet struct {
	countRamp  []rune // resting order-count density, index 0 = empty
	tradeRamp  []rune // trade-flow magnitude, small → huge
	microRamp  []rune // NOW-row sub-cell depth bars, shallow → deep
	newsRamp   []rune // news rail marker per severity, low → critical
	persistent rune   // long-standing (persistent) liquidity
	ownOrder   rune   // your resting order, on the map
	ownBuy     rune   // your resting buy, on the ruler
	ownSell    rune   // your resting sell, on the ruler
	cursor     rune   // the price cursor, on the ruler
	touchTick  rune   // the two touch columns, on the ruler
	rulerLine  rune   // ruler baseline
	railIdle   rune   // news rail with nothing in the window
}

// glyphs is the active vocabulary, calibrated against DejaVuSansMono by the
// glyph-bank rasterizer (measured ink coverage; re-run per terminal font):
//   - countRamp ░▒▓█ measures 0.22/0.56/0.86/1.00 — NOT evenly spaced, so it
//     only carries the COARSE categorical count channel; fine intensity rides
//     on the colour ramp.
//   - tradeRamp ○◆●■ measures 0.16/0.26/0.40/0.51 — ascending-ink distinct
//     SHAPES, so trade-flow never reads as resting liquidity.
//   - microRamp eighth-blocks are an even ~0.13/step ladder (measured), the
//     one family safe for a linear magnitude bar.
//   - Braille is unusable (renders as tofu in DejaVuSansMono) — quadrants
//     like ▚ are the sanctioned sub-cell family instead.
var glyphs = glyphSet{
	countRamp:  []rune{' ', '░', '▒', '▓', '█'},
	tradeRamp:  []rune{'○', '◆', '●', '■'},
	microRamp:  []rune{'▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'},
	newsRamp:   []rune{'·', '►', '►', '‼'},
	persistent: '▚',
	ownOrder:   '◇',
	ownBuy:     '▲',
	ownSell:    '▼',
	cursor:     '┃',
	touchTick:  '┼',
	rulerLine:  '─',
	railIdle:   '│',
}

// sizeTiers is the number of non-empty intensity steps on the size ramp.
const sizeTiers = 4

// persistThreshold is how long a level must hold before it renders as
// long-standing liquidity (the ▚ mark). Client-side L2 proxy for now — see
// book.AgeSource for the seam a real order-age feed plugs into.
const persistThreshold = 30 * time.Second

// binInterval is the live-row time-bin width — one ring row per tick.
const binInterval = 100 * time.Millisecond

// maxFarRows caps the aggregated block so live detail keeps the majority of
// a very tall terminal.
const maxFarRows = 12

// gutterWidth is the right time-axis gutter ("  −10m").
const gutterWidth = 6

// renderMode is the graceful-degradation tier chosen from terminal capability.
type renderMode int

const (
	// modePlain is colourless: glyphs only.
	modePlain renderMode = iota
	// modeShade is 16-colour: glyph density for size, hues only for trades.
	modeShade
	// modeTrue is the full encoding: ramp background + glyph channels.
	modeTrue
)

// binTickMsg fires once per bin interval to seal a live heatmap row.
type binTickMsg time.Time

// binTickCmd schedules the next bin tick.
func binTickCmd() tea.Cmd { return tea.Tick(binInterval, toBinTick) }

// toBinTick wraps a tick time as a binTickMsg (named, per the no-inline-closure
// convention).
func toBinTick(t time.Time) tea.Msg { return binTickMsg(t) }

// detectMode picks the render tier from the environment. RSX_TERM_COLOR
// (plain|shade|true) forces it; otherwise NO_COLOR / TERM / COLORTERM decide.
// lipgloss further maps RGB down to the real terminal's palette, so modeTrue is
// safe to request on a 256-colour terminal.
func detectMode() renderMode {
	switch os.Getenv("RSX_TERM_COLOR") {
	case "plain":
		return modePlain
	case "shade":
		return modeShade
	case "true":
		return modeTrue
	}
	if os.Getenv("NO_COLOR") != "" {
		return modePlain
	}
	term := os.Getenv("TERM")
	if term == "" || term == "dumb" {
		return modePlain
	}
	if ct := os.Getenv("COLORTERM"); ct == "truecolor" || ct == "24bit" {
		return modeTrue
	}
	if strings.Contains(term, "256") || strings.Contains(term, "truecolor") {
		return modeTrue
	}
	return modeShade
}

// sizeTier maps a resting size to a log-scaled intensity step 0..sizeTiers,
// relative to the frame's largest bucket (sizes are heavy-tailed, so log). Any
// nonzero size shows at least tier 1 so a thin level never vanishes.
func sizeTier(size, refMax int64) int {
	return logTier(size, refMax, sizeTiers)
}

// tradeTier maps a print's quantity to a magnitude step 1..len(tradeRamp)
// relative to the frame's largest print — the "how big" of the trade layer.
func tradeTier(qty, refMax int64) int {
	t := logTier(qty, refMax, len(glyphs.tradeRamp))
	if t < 1 {
		t = 1
	}
	return t
}

// logTier is the shared log scale: 0 for empty, else 1..steps.
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

// countTier maps a resting order count to a density-glyph step 0..4: one
// order is a whale (faint ░), many orders read as a solid wall (█).
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

// rampColor is the single sequential size ramp: the page background blended
// toward the live hue, dim (tier 0) → full (tier sizeTiers). One ramp for
// BOTH sides — side is position, colour is magnitude.
func rampColor(tier int) lipgloss.Color {
	return lipgloss.Color(blendHex(string(ColorPageBg), string(ColorLive), float64(tier)/float64(sizeTiers)))
}

// aggressorStyle is the trade layer's hue: buy prints in the bid hue, sell
// prints in the ask hue (theme-aware; the resting book never uses these).
func aggressorStyle(side wire.Side) lipgloss.Style {
	if side == wire.Sell {
		return StyleAsk
	}
	return StyleLive
}

// cell is one rendered bucket: the fisheye fold of a row's price-space
// profile plus the trade-flow that printed there.
type cell struct {
	size      int64
	count     int32
	ageNs     int64
	tradeQty  int64
	tradeSide wire.Side
	own       bool
}

// foldCells maps a row's price-space levels and trades onto fisheye columns
// against the CURRENT anchor — so a recenter re-aligns history too.
func foldCells(row book.Row, anchor, tick int64, width int) []cell {
	cells := make([]cell, width)
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
		if tr.Qty >= c.tradeQty { // biggest print wins the cell's side
			c.tradeSide = tr.Side
		}
		c.tradeQty += tr.Qty
	}
	return cells
}

// cellStr renders one heatmap cell (one glyph wide). The trade layer draws
// over the resting-size background so both read together; otherwise the cell
// is the book: single-ramp background (size), density glyph (count), with ▚
// marking long-standing liquidity and ◉ marking your own resting order.
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
		fgTier := sz + 1
		if fgTier > sizeTiers {
			fgTier = sizeTiers
		}
		return lipgloss.NewStyle().
			Foreground(rampColor(fgTier)).
			Background(rampColor(sz)).
			Render(string(glyph))
	}
}

// tradeCellStr renders the trade layer: an aggressor-hued magnitude glyph
// (· • ◆ █ by print size) over the resting-size background.
func tradeCellStr(c cell, sz int, tradeMax int64, mode renderMode) string {
	glyph := string(glyphs.tradeRamp[tradeTier(c.tradeQty, tradeMax)-1])
	switch mode {
	case modePlain:
		return glyph
	case modeShade:
		return aggressorStyle(c.tradeSide).Bold(true).Render(glyph)
	default:
		return aggressorStyle(c.tradeSide).
			Bold(true).
			Background(rampColor(sz)).
			Render(glyph)
	}
}

// ownCellStr marks your own resting order on the map (◉, accent) over the
// level's resting-size background so it never hides the book under it.
func ownCellStr(sz int, mode renderMode) string {
	glyph := string(glyphs.ownOrder)
	switch mode {
	case modePlain:
		return glyph
	case modeShade:
		return StyleAccent.Bold(true).Render(glyph)
	default:
		return StyleAccent.Bold(true).Background(rampColor(sz)).Render(glyph)
	}
}

// microCellStr renders one NOW-row cell: an eighth-block micro-bar scaled to
// the frame's deepest bucket — the exact live book as a miniature histogram.
func microCellStr(c cell, refMax, tradeMax int64, mode renderMode) string {
	if c.tradeQty > 0 {
		return tradeCellStr(c, sizeTier(c.size, refMax), tradeMax, mode)
	}
	if c.own {
		return ownCellStr(sizeTier(c.size, refMax), mode)
	}
	if c.size <= 0 {
		return " "
	}
	steps := len(glyphs.microRamp)
	idx := int(float64(c.size) / float64(refMax) * float64(steps))
	if idx < 1 {
		idx = 1
	}
	if idx > steps {
		idx = steps
	}
	glyph := string(glyphs.microRamp[idx-1])
	if c.ageNs >= int64(persistThreshold) {
		glyph = string(glyphs.persistent)
	}
	switch mode {
	case modePlain:
		return glyph
	case modeShade:
		return StyleText.Render(glyph)
	default:
		return lipgloss.NewStyle().Foreground(rampColor(sizeTier(c.size, refMax))).Render(glyph)
	}
}

// blendHex linearly interpolates two "#rrggbb" colours, t in [0,1].
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

// hexRGB parses "#rrggbb" into its channels (0 on a malformed value).
func hexRGB(h string) (int, int, int) {
	if len(h) != 7 || h[0] != '#' {
		return 0, 0, 0
	}
	r, _ := strconv.ParseInt(h[1:3], 16, 0)
	g, _ := strconv.ParseInt(h[3:5], 16, 0)
	b, _ := strconv.ParseInt(h[5:7], 16, 0)
	return int(r), int(g), int(b)
}

// viewStream renders the active streaming screen as a fixed grid (see the
// file header): the depth BOOK (default), the multi-pair chase grid, the
// news overview, or the LLM assistant. Selected by RSX_TERM_STREAM; the DOM
// View is untouched when the flag is off.
func (m Model) viewStream() string {
	if m.showHelp {
		return m.viewStreamHelp()
	}
	switch m.screen {
	case screenPair:
		return m.viewPair()
	case screenNews:
		return m.viewNews()
	case screenLLM:
		return m.viewLLM()
	default:
		return m.viewBookScreen()
	}
}

// viewBookScreen is the depth heatmap: hand market-making on ONE symbol
// (hop symbols with x + letter code; cover breadth in the pair view).
func (m Model) viewBookScreen() string {
	if m.heatW == 0 {
		return StyleMuted.Render("streaming heatmap — sizing… (waiting for terminal size / first frame)")
	}
	mk := m.mkt()
	mode := detectMode()
	nowNs := time.Now().UnixNano()
	now := book.LiveFold(mk.book.Bids, mk.book.Asks, mk.pending, mk.persist, mk.lastBinNs, nowNs)
	refMax, tradeMax := stableBases(mk, now)
	anchor := mk.heat.Anchor()
	tick := mk.heat.Tick()

	rows := mk.heat.Rows()
	far := mk.heat.FarRows()
	body := make([]string, 0, far+mk.heat.LiveCap()+1)
	for _, row := range rows[:far] {
		body = append(body, m.renderRow(row, anchor, tick, refMax, tradeMax, mode, false))
	}
	for i := 0; i < mk.heat.LiveCap()-mk.heat.LiveLen(); i++ {
		body = append(body, m.blankRow())
	}
	for _, row := range rows[far:] {
		body = append(body, m.renderRow(row, anchor, tick, refMax, tradeMax, mode, false))
	}
	body = append(body, m.renderRow(now, anchor, tick, refMax, tradeMax, mode, true))
	tape := m.tapeRail(len(body), tradeMax)
	for i := range body {
		body[i] += tape[i]
	}

	lines := make([]string, 0, m.height)
	lines = append(lines, m.streamHeader(), m.modeLine())
	lines = append(lines, body...)
	lines = append(lines, m.rulerLine(anchor, tick))
	lines = append(lines, m.streamFooter()...)
	return strings.Join(lines, "\n")
}

// stableBases returns the ramp references: the slow-decaying bases (foldBases)
// floored by whatever the live frame shows right now, so a fresh spike still
// registers between bin ticks while the view never renormalises downward
// frame-to-frame.
func stableBases(mk *market, now book.Row) (refMax, tradeMax int64) {
	refMax, tradeMax = mk.sizeBasis, mk.tradeBasis
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

// modeLine is the persistent status strip under the header: the active
// screen, the RO/PO modifier toggles (always visible — the trader must know
// what mode the next keystroke fires in), the armed size, the watchlist, and
// screen-specific state (the armed pair symbol, the switcher buffer).
func (m Model) modeLine() string {
	tag := lipgloss.NewStyle().
		Foreground(ColorPageBg).
		Background(ColorAccent).
		Bold(true).
		Render(" " + m.screen.label() + " ")
	mod := func(name string, on bool) string {
		if on {
			return StyleArmed.Render(" " + name + " ")
		}
		return StyleMuted.Render(" " + name + " off ")
	}
	venue := m.activeVenue
	if m.screen == screenPair || m.screen == screenNews {
		venue = m.pairVenue()
	}
	parts := []string{
		tag,
		StyleHeading.Render(" " + venue + " "),
		mod("RO", m.reduceOnly),
		mod("PO", m.postOnly),
		StyleMuted.Render(fmt.Sprintf(" size %s [%d] ", m.fmtQty(m.sizePreset()), m.sizeSel+1)),
		StyleMuted.Render(" list " + m.lists[m.listSel].name + " "),
	}
	if m.switching {
		parts = append(parts, StyleTextBright.Bold(true).Render(" switch: "+m.switchBuf+"_ ")+m.switchCandidates())
	}
	if m.venuePicking {
		var vs []string
		for _, v := range m.venues {
			vs = append(vs, v.Code+" "+v.Name)
		}
		parts = append(parts, StyleTextBright.Bold(true).Render(" venue? ")+StyleMuted.Render(strings.Join(vs, " · ")))
	}
	if m.screen == screenPair {
		if m.armedSym != 0 {
			armed := m.instrumentFor(m.pairVenue(), m.armedSym).Name
			if m.countBuf != "" {
				armed += " ×" + m.countBuf
			}
			parts = append(parts, StyleArmed.Render(" ARMED "+armed+" "))
		} else {
			parts = append(parts, StyleMuted.Render(" press a letter to arm "))
		}
	}
	return lipgloss.JoinHorizontal(lipgloss.Top, parts...)
}

// switchCandidates lists the active venue's code → name pairs that still
// match the switcher buffer (capped — a 150-symbol venue must not overflow
// the mode line).
func (m Model) switchCandidates() string {
	venue, _ := m.venueByName(m.activeVenue)
	var parts []string
	for _, ins := range venue.Instruments {
		if strings.HasPrefix(ins.Code, m.switchBuf) {
			parts = append(parts, ins.Code+" "+ins.Name)
		}
		if len(parts) == 8 {
			parts = append(parts, "…")
			break
		}
	}
	if len(parts) == 0 {
		return StyleAsk.Render("no match")
	}
	return StyleMuted.Render(strings.Join(parts, " · "))
}

// renderRow renders one body line: news rail + heatmap cells + time gutter.
// The NOW row (isNow) uses micro-bar glyphs and carries your resting orders.
func (m Model) renderRow(row book.Row, anchor, tick, refMax, tradeMax int64, mode renderMode, isNow bool) string {
	cells := foldCells(row, anchor, tick, m.heatW)
	if isNow {
		m.markOwnOrders(cells, anchor, tick)
	}
	var sb strings.Builder
	sb.WriteString(m.railChar(row))
	for _, c := range cells {
		if isNow {
			sb.WriteString(microCellStr(c, refMax, tradeMax, mode))
		} else {
			sb.WriteString(cellStr(c, refMax, tradeMax, mode))
		}
	}
	sb.WriteString(m.gutterLabel(row, isNow))
	return sb.String()
}

// tapeRailWidth is the adjacent trade-tape column: separator + side glyph +
// space + price (quantized size rides on the glyph, never an exact number).
const tapeRailWidth = 14

// tapeRail renders the compact trade FEED beside the heatmap: one print per
// body row, newest at the top — aggressor hue, magnitude glyph (the same
// tradeRamp as the map), exact price. Trades therefore appear BOTH at their
// level in the heatmap AND in this feed.
func (m Model) tapeRail(rows int, tradeMax int64) []string {
	out := make([]string, rows)
	entries := m.mkt().tape.Entries()
	blank := StyleMuted.Render(" ┆") + strings.Repeat(" ", tapeRailWidth-2)
	for i := 0; i < rows; i++ {
		if i >= len(entries) {
			out[i] = blank
			continue
		}
		e := entries[i]
		glyph := string(glyphs.tradeRamp[tradeTier(e.Qty, tradeMax)-1])
		txt := fmt.Sprintf("%s %s", glyph, m.fmtPx(e.Px))
		if len(txt) > tapeRailWidth-3 {
			txt = txt[:tapeRailWidth-3]
		}
		out[i] = StyleMuted.Render(" ┆") + aggressorStyle(e.Side).Render(fmt.Sprintf(" %-*s", tapeRailWidth-3, txt))
	}
	return out
}

// viewStreamHelp is the streaming view's modal key reference (any key closes).
func (m Model) viewStreamHelp() string {
	grp := func(h string) string { return StyleHeading.Render(h) }
	key := func(k, d string) string {
		return StyleTextBright.Render(fmt.Sprintf("  %-9s", k)) + StyleMuted.Render(d)
	}
	danger := func(k, d string) string {
		return StyleAsk.Render(fmt.Sprintf("  %-9s", k)) + StyleMuted.Render(d)
	}
	rows := []string{
		StyleHeading.Bold(true).Render("KEYS — streaming · any key to close"),
		"",
		grp("game order entry (fires on ONE key — size cap still hard-blocks)"),
		key("b / s", "side buy / sell"),
		key("1-5", "arm a size preset"),
		key("h l ← →", "move the price cursor a tick"),
		key("j / k", "snap cursor to best bid / ask"),
		key("click", "set the cursor from the map"),
		danger("f", "place resting limit at the cursor"),
		danger("⇧1-5", "cross NOW — IOC at the far touch, preset size"),
		danger("d", "cancel own order nearest the cursor"),
		"",
		grp("view"),
		key("?", "this help"),
		danger("q / esc", "quit"),
	}
	return RingPanelStyle.Render(lipgloss.JoinVertical(lipgloss.Left, rows...))
}

// markOwnOrders flags the cells where this session's orders rest on the
// ACTIVE symbol, so the map itself shows them (◇) — they are re-marked every
// frame at their live price column, never lost as history drifts up.
func (m Model) markOwnOrders(cells []cell, anchor, tick int64) {
	for _, o := range m.ownOrdersFor(m.activeVenue, m.active) {
		col, ok := book.FisheyeCol(o.Px, anchor, tick, len(cells))
		if !ok {
			continue
		}
		cells[col].own = true
	}
}

// blankRow pads the live block before the ring fills, keeping the grid fixed.
func (m Model) blankRow() string {
	return StyleMuted.Render(string(glyphs.railIdle)) +
		strings.Repeat(" ", m.heatW) +
		strings.Repeat(" ", gutterWidth)
}

// railChar renders the news rail for one row: a severity-graded marker when
// an enabled source has a headline inside the row's window — low reads quiet,
// critical stands out — otherwise a faint gutter. The full headline lives in
// context mode (progressive disclosure), never inline here.
func (m Model) railChar(row book.Row) string {
	if m.news == nil || !m.news.Enabled() {
		return StyleMuted.Render(string(glyphs.railIdle))
	}
	from, to := row.FromNs, row.ToNs
	if from == 0 && to == 0 {
		return StyleMuted.Render(string(glyphs.railIdle))
	}
	markers := m.news.Markers(from, to)
	if len(markers) == 0 {
		return StyleMuted.Render(string(glyphs.railIdle))
	}
	worst := 0
	for _, mk := range markers {
		if mk.Tier > worst {
			worst = mk.Tier
		}
	}
	return newsMarker(worst)
}

// newsMarker renders one rail marker for a severity tier (see glyphs.newsRamp).
func newsMarker(tier int) string {
	if tier < 0 {
		tier = 0
	}
	if tier >= len(glyphs.newsRamp) {
		tier = len(glyphs.newsRamp) - 1
	}
	glyph := string(glyphs.newsRamp[tier])
	switch tier {
	case 0:
		return StyleMuted.Render(glyph)
	case 1:
		return StyleText.Render(glyph)
	case 2:
		return StyleDegraded.Render(glyph)
	default:
		return StyleAsk.Bold(true).Render(glyph)
	}
}

// gutterLabel is the right time-axis label: each far row shows its horizon
// (its aggregation window), the NOW row says "now", live rows stay blank.
func (m Model) gutterLabel(row book.Row, isNow bool) string {
	if isNow {
		return StyleTextBright.Render(fmt.Sprintf("%*s", gutterWidth, "now"))
	}
	if row.Span <= 0 {
		return strings.Repeat(" ", gutterWidth)
	}
	return StyleMuted.Render(fmt.Sprintf("%*s", gutterWidth, "−"+fmtSpan(row.Span)))
}

// fmtSpan renders a schedule span compactly: 10s, 1m, 2m, 10m, 1h.
func fmtSpan(d time.Duration) string {
	if d >= time.Hour {
		return fmt.Sprintf("%dh", int(d/time.Hour))
	}
	if d >= time.Minute {
		return fmt.Sprintf("%dm", int(d/time.Minute))
	}
	return fmt.Sprintf("%ds", int(d/time.Second))
}

// rulerLine is the interaction axis under the NOW row: the fisheye baseline
// with the touch columns ticked (┼), your resting orders as side-shaped ▲/▼,
// and the price cursor ┃ (game order entry moves it; f fires at it).
func (m Model) rulerLine(anchor, tick int64) string {
	marks := make([]rune, m.heatW)
	for i := range marks {
		marks[i] = glyphs.rulerLine
	}
	styles := make([]lipgloss.Style, m.heatW)
	for i := range styles {
		styles[i] = StyleMuted
	}
	half := m.heatW / 2
	if half >= 1 {
		marks[half-1], marks[half] = glyphs.touchTick, glyphs.touchTick
	}
	for _, o := range m.ownOrdersFor(m.activeVenue, m.active) {
		col, ok := book.FisheyeCol(o.Px, anchor, tick, m.heatW)
		if !ok {
			continue
		}
		if o.Side == wire.Buy {
			marks[col] = glyphs.ownBuy
		} else {
			marks[col] = glyphs.ownSell
		}
		styles[col] = StyleAccent.Bold(true)
	}
	if cursor := m.mkt().cursorPx; cursor > 0 {
		if col, ok := book.FisheyeCol(cursor, anchor, tick, m.heatW); ok {
			marks[col] = glyphs.cursor
			styles[col] = StyleTextBright.Bold(true)
		}
	}
	var sb strings.Builder
	sb.WriteString(StyleMuted.Render(string(glyphs.railIdle)))
	for i, r := range marks {
		sb.WriteString(styles[i].Render(string(r)))
	}
	sb.WriteString(strings.Repeat(" ", gutterWidth))
	return sb.String()
}

// streamHeader is the top strip: symbol badge, link dot, and the price axis
// legend with the anchored mid.
func (m Model) streamHeader() string {
	badge := lipgloss.NewStyle().
		Foreground(ColorPageBg).
		Background(ColorHeading).
		Bold(true).
		Render(fmt.Sprintf(" RSX  %s ", m.ins().Name))
	link := StyleLive.Render("● live")
	if !m.gwConnected {
		link = StyleDegraded.Render("● offline")
	}
	mid := "—"
	if p := m.mkt().heat.MidPx(); p > 0 {
		mid = m.fmtPx(p)
	}
	axis := StyleMuted.Render("◀ bids") +
		StyleText.Render(fmt.Sprintf("  mid %s  ", mid)) +
		StyleMuted.Render("asks ▶")
	return lipgloss.JoinHorizontal(lipgloss.Top, badge, "  ", link, "  ", axis)
}

// streamLegend is the one-line control hint pinned under the heatmap.
const streamLegend = " q quit  tab view  x symbol  n news  b/s side  1-5 size  ⇧1-5 cross  h/l j/k cursor  f place  d cancel  r/p RO/PO  ? help "

// newsLegend / llmLegend are the news and assistant screens' hint lines.
const newsLegend = " q quit  tab view  / search  j/k select  enter → assistant  letter → book  esc back "
const llmLegend = " q quit  tab view  esc → news "

// hintLine is the persistent context-sensitive hint for the active screen
// (the k9s pattern: the keys that matter right now, ? for the full map).
func (m Model) hintLine() string {
	switch m.screen {
	case screenPair:
		return StyleMuted.Render(pairLegend)
	case screenNews:
		return StyleMuted.Render(newsLegend)
	case screenLLM:
		return StyleMuted.Render(llmLegend)
	default:
		return StyleMuted.Render(streamLegend)
	}
}

// streamFooter is the pinned status block: the exact touch ladder (two
// lines), position, latency, and the control legend.
func (m Model) streamFooter() []string {
	mk := m.mkt()
	return []string{
		m.touchLadderLine(mk.book.Asks, "ask ", StyleAsk),
		m.touchLadderLine(mk.book.Bids, "bid ", StyleLive),
		m.streamPosLine(),
		m.streamLatLine(),
		m.hintLine(),
	}
}

// touchLadderLine renders up to three exact levels of one side — precise
// px×size at display precision, nearest the touch first — or a dash when
// that side is empty.
func (m Model) touchLadderLine(levels []wire.Level, label string, style lipgloss.Style) string {
	out := StyleMuted.Render(" " + label)
	if len(levels) == 0 {
		return out + StyleMuted.Render("—")
	}
	n := len(levels)
	if n > 3 {
		n = 3
	}
	parts := make([]string, 0, n)
	for _, l := range levels[:n] {
		parts = append(parts, fmt.Sprintf("%s×%s", m.fmtPx(l.Px), m.fmtQty(l.Qty)))
	}
	return out + style.Render(strings.Join(parts, "  "))
}

// streamPosLine shows the ACTIVE symbol's client-derived position and
// mid-marked uPnL, mirroring the DOM positions panel's honesty (dashed uPnL
// until a mid exists).
func (m Model) streamPosLine() string {
	mk := m.mkt()
	if mk.position.Flat() {
		return StyleMuted.Render(" pos flat — fills build it")
	}
	net := mk.position.Net
	word, st := "LONG", StyleLive
	if net < 0 {
		word, st = "SHORT", StyleAsk
	}
	netStr := m.fmtQty(net)
	if net > 0 {
		netStr = "+" + netStr
	}
	entry := "—"
	if e, ok := mk.position.Entry(); ok {
		entry = m.fmtPx(e)
	}
	upnl := StyleDegraded.Render("~uPnL —") + StyleMuted.Render(" (needs live book)")
	if mid, ok := mk.book.Mid(); ok {
		if u, ok := mk.position.Upnl(mid); ok {
			v, us := m.fmtNotional(u), StyleLive
			if u > 0 {
				v = "+" + v
			} else if u < 0 {
				us = StyleAsk
			}
			upnl = StyleMuted.Render("~uPnL ") + us.Render(v)
		}
	}
	return StyleMuted.Render(" pos ") + st.Render(word) + " " +
		StyleText.Render(netStr+" @ "+entry) + "   " + upnl
}

// streamLatLine shows the round-trip latency (⚡) plus the rolling window, or a
// waiting note before the first round-trip.
func (m Model) streamLatLine() string {
	zap := StyleHeading.Render(" ⚡")
	if m.lastLat == nil {
		return zap + StyleMuted.Render(" latency: waiting for first round-trip…")
	}
	p50, p99, best := windowStats(m.latWindow)
	return zap + StyleText.Render(fmt.Sprintf(" RTT %s", fmtNs(m.lastLat.TotalNs))) +
		StyleMuted.Render(fmt.Sprintf("   p50 %s · p99 %s · best %s", p50, p99, best))
}
