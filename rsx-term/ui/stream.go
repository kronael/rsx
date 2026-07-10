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
)

// Streaming "text Bookmap" heatmap (RSX_TERM_STREAM=1). Time flows top→bottom,
// one row per ~100ms bin, newest at the bottom; price runs left→right through a
// mid-centred fisheye (bids left, asks right, near-touch at 1 tick/cell). The
// ring, fisheye, and binning live in book.Heatmap; this file only renders.
//
// CELL ENCODING — two independent channels per cell:
//   - COLOUR (background) = resting SIZE, log-scaled (sizes are heavy-tailed):
//     the side's hue, dim → saturated as size grows (sizeTier). Bid hue and ask
//     hue are the palette's bid/ask pair (colorblind theme swaps them for
//     blue/orange automatically, since they are the same ColorBid/ColorAsk vars).
//   - GLYPH (` ░▒▓█`) = resting ORDER COUNT (countTier): one whale (huge size,
//     1 order) shows a bright cell with a faint ░; a wall of many small orders
//     shows a fuller ▓/█. The glyph's foreground is the size hue one tier
//     brighter than the background, so the count texture stays legible even
//     under a solid █ while the size colour still reads.
//
// TRADES overlay the resting map: a bin's prints paint a bright ◆ in the
// aggressor's hue at the trade column, brightest on the newest row and decaying
// over the next couple of rows as they age upward.
//
// DEGRADES gracefully by terminal colour (detectMode): full two-channel RGB on
// 256/true-colour; shades-only (size via glyph, side via 16-colour hue) on a
// plain colour terminal; a colourless glyph ladder when colour is unavailable.

// shades is the ` ░▒▓█` intensity ramp, shared by the size and count channels.
var shades = []rune{' ', '░', '▒', '▓', '█'}

// sizeTiers is the number of non-empty intensity steps (ramp index 1..4).
const sizeTiers = 4

// tradeGlyph marks a cell where trades printed this bin.
const tradeGlyph = '◆'

// binInterval is the heatmap time-bin width — one ring row per tick.
const binInterval = 100 * time.Millisecond

// renderMode is the graceful-degradation tier chosen from terminal capability.
type renderMode int

const (
	// modePlain is colourless: a glyph ladder (size via ` ░▒▓█`), side by column.
	modePlain renderMode = iota
	// modeShade is 16-colour: side hue + glyph by size (count channel dropped).
	modeShade
	// modeTrue is full two-channel: RGB background by size, glyph by count.
	modeTrue
)

// binTickMsg fires once per bin interval to fold a fresh heatmap row.
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
	if size <= 0 || refMax <= 0 {
		return 0
	}
	ratio := math.Log(float64(size)+1) / math.Log(float64(refMax)+1)
	t := int(ratio * float64(sizeTiers))
	if t < 1 {
		t = 1
	}
	if t > sizeTiers {
		t = sizeTiers
	}
	return t
}

// countTier maps a resting order count to a glyph step 0..4: one order is a
// whale (faint ░), many orders read as a solid wall (█).
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

// hueFor returns the side's palette hue as a hex string (theme-aware).
func hueFor(side int8) string {
	if side > 0 {
		return string(ColorAsk)
	}
	return string(ColorBid)
}

// sizeBg is the cell background for a size tier: the side hue blended out of the
// page background, dim (tier 0) → full hue (tier sizeTiers).
func sizeBg(side int8, tier int) lipgloss.Color {
	return lipgloss.Color(blendHex(string(ColorPageBg), hueFor(side), float64(tier)/float64(sizeTiers)))
}

// cellStr renders one heatmap cell (one glyph wide). Trades take visual
// priority; otherwise resting liquidity renders per the mode's channels. decay
// (1 newest → ~0 oldest) dims a trade mark as its row ages.
func cellStr(c book.Cell, refMax int64, decay float64, mode renderMode) string {
	if c.BuyTrade > 0 || c.SellTrade > 0 {
		return tradeCell(c, decay, mode)
	}
	if c.Size <= 0 || c.Side == 0 {
		return " "
	}
	sz := sizeTier(c.Size, refMax)
	switch mode {
	case modePlain:
		return string(shades[sz])
	case modeShade:
		return sideStyle(c.Side).Render(string(shades[sz]))
	default:
		fgTier := sz + 1
		if fgTier > sizeTiers {
			fgTier = sizeTiers
		}
		return lipgloss.NewStyle().
			Foreground(sizeBg(c.Side, fgTier)).
			Background(sizeBg(c.Side, sz)).
			Render(string(shades[countTier(c.Count)]))
	}
}

// tradeCell renders a bright aggressor-coloured ◆, decayed by row age.
func tradeCell(c book.Cell, decay float64, mode renderMode) string {
	side := int8(-1) // buy aggressor -> bid hue
	if c.SellTrade > c.BuyTrade {
		side = 1
	}
	switch mode {
	case modePlain:
		return string(tradeGlyph)
	case modeShade:
		return sideStyle(side).Bold(true).Render(string(tradeGlyph))
	default:
		bg := lipgloss.Color(blendHex(string(ColorPageBg), hueFor(side), 0.35+0.65*decay))
		return lipgloss.NewStyle().
			Foreground(ColorTextBright).
			Background(bg).
			Bold(true).
			Render(string(tradeGlyph))
	}
}

// sideStyle is the 16-colour side foreground (bid green / ask red, theme-aware).
func sideStyle(side int8) lipgloss.Style {
	if side > 0 {
		return StyleAsk
	}
	return StyleLive
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

// streamLegend is the one-line control hint pinned under the heatmap.
const streamLegend = " q quit  b/s side  +/- tick  enter submit  F3 trace  ? help  · streaming heatmap (RSX_TERM_STREAM) "

// viewStream renders the whole streaming heatmap screen: header (symbol / link /
// mid axis), the scrolling ring body, and a pinned footer (touch, position,
// latency, an LLM placeholder, the control legend). Selected by RSX_TERM_STREAM;
// the DOM View is untouched when the flag is off.
func (m Model) viewStream() string {
	if m.heat == nil {
		return StyleMuted.Render("streaming heatmap — sizing… (waiting for terminal size / first frame)")
	}
	mode := detectMode()
	lines := []string{m.streamHeader()}
	lines = append(lines, m.streamBody(mode)...)
	lines = append(lines, m.streamFooter()...)
	return strings.Join(lines, "\n")
}

// streamHeader is the top strip: symbol badge, link dot, and the price axis
// legend with the anchored mid.
func (m Model) streamHeader() string {
	badge := lipgloss.NewStyle().
		Foreground(ColorPageBg).
		Background(ColorHeading).
		Bold(true).
		Render(fmt.Sprintf(" RSX  %s ", m.cfg.Symbol))
	link := StyleLive.Render("● live")
	if !m.gwConnected {
		link = StyleDegraded.Render("● offline")
	}
	mid := "—"
	if p := m.heat.MidPx(); p > 0 {
		mid = m.fmtPx(p)
	}
	axis := StyleMuted.Render("◀ bids") +
		StyleText.Render(fmt.Sprintf("  mid %s  ", mid)) +
		StyleMuted.Render("asks ▶")
	return lipgloss.JoinHorizontal(lipgloss.Top, badge, "  ", link, "  ", axis)
}

// streamBody renders the ring: a left news rail plus one heatmap line per row,
// newest at the bottom. Empty leading rows (before the ring fills) are padded so
// the newest bin stays anchored to the bottom. Trades decay in brightness as
// their row ages upward.
func (m Model) streamBody(mode renderMode) []string {
	rows := m.heat.Rows()
	refMax := refMaxOf(rows)
	height := m.heat.Height()
	out := make([]string, 0, height)
	for i := 0; i < height-len(rows); i++ {
		out = append(out, StyleMuted.Render("│"))
	}
	n := len(rows)
	for j, row := range rows {
		decay := 1.0 - 0.3*float64(n-1-j)
		if decay < 0.15 {
			decay = 0.15
		}
		var sb strings.Builder
		sb.WriteString(m.railChar(row.BinTs))
		for _, c := range row.Cells {
			sb.WriteString(cellStr(c, refMax, decay, mode))
		}
		out = append(out, sb.String())
	}
	return out
}

// railChar renders the news rail gutter for one row: a bright ► when an enabled
// source has a headline in this bin's window, otherwise a faint gutter. The
// default source is off, so offline the rail is a plain gutter.
func (m Model) railChar(binTs int64) string {
	half := int64(binInterval / 2)
	if m.news != nil && m.news.Enabled() && len(m.news.Markers(binTs-half, binTs+half)) > 0 {
		return StyleDegraded.Render("►")
	}
	return StyleMuted.Render("│")
}

// refMaxOf is the largest resting size across the visible ring, the reference
// the size channel log-scales against (floored at 1 so an empty map is safe).
func refMaxOf(rows []book.Row) int64 {
	max := int64(1)
	for _, r := range rows {
		for _, c := range r.Cells {
			if c.Size > max {
				max = c.Size
			}
		}
	}
	return max
}

// streamFooter is the pinned (non-scrolling) status block under the heatmap.
func (m Model) streamFooter() []string {
	return []string{
		m.streamTouchLine(),
		m.streamPosLine(),
		m.streamLatLine(),
		StyleMuted.Render("? assistant — context ready (placeholder)"),
		StyleMuted.Render(streamLegend),
	}
}

// streamTouchLine shows the exact live touch: best bid / ask price × size and
// the spread, using the symbol's display precision.
func (m Model) streamTouchLine() string {
	bid := StyleMuted.Render("—")
	if b, ok := m.book.BestBid(); ok {
		bid = StyleLive.Render(fmt.Sprintf("%s × %s", m.fmtPx(b.Px), m.fmtQty(b.Qty)))
	}
	ask := StyleMuted.Render("—")
	if a, ok := m.book.BestAsk(); ok {
		ask = StyleAsk.Render(fmt.Sprintf("%s × %s", m.fmtPx(a.Px), m.fmtQty(a.Qty)))
	}
	spread := StyleMuted.Render(fmt.Sprintf("spread %d", m.book.Spread()))
	return StyleMuted.Render("bid ") + bid + StyleMuted.Render("   ask ") + ask + "   " + spread
}

// streamPosLine shows the client-derived position and mid-marked uPnL, mirroring
// the DOM positions panel's honesty (dashed uPnL until a mid exists).
func (m Model) streamPosLine() string {
	if m.position.Flat() {
		return StyleMuted.Render("pos flat — fills build it")
	}
	net := m.position.Net
	word, st := "LONG", StyleLive
	if net < 0 {
		word, st = "SHORT", StyleAsk
	}
	netStr := m.fmtQty(net)
	if net > 0 {
		netStr = "+" + netStr
	}
	entry := "—"
	if e, ok := m.position.Entry(); ok {
		entry = m.fmtPx(e)
	}
	upnl := StyleDegraded.Render("~uPnL —") + StyleMuted.Render(" (needs live book)")
	if mid, ok := m.book.Mid(); ok {
		if u, ok := m.position.Upnl(mid); ok {
			v, us := m.fmtNotional(u), StyleLive
			if u > 0 {
				v = "+" + v
			} else if u < 0 {
				us = StyleAsk
			}
			upnl = StyleMuted.Render("~uPnL ") + us.Render(v)
		}
	}
	return StyleMuted.Render("pos ") + st.Render(word) + " " +
		StyleText.Render(netStr+" @ "+entry) + "   " + upnl
}

// streamLatLine shows the round-trip latency (⚡) plus the rolling window, or a
// waiting note before the first round-trip.
func (m Model) streamLatLine() string {
	zap := StyleHeading.Render("⚡")
	if m.lastLat == nil {
		return zap + StyleMuted.Render(" latency: waiting for first round-trip…")
	}
	p50, p99, best := windowStats(m.latWindow)
	return zap + StyleText.Render(fmt.Sprintf(" RTT %s", fmtNs(m.lastLat.TotalNs))) +
		StyleMuted.Render(fmt.Sprintf("   p50 %s · p99 %s · best %s", p50, p99, best))
}
