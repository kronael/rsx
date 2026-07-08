package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"

	"rsx-term/wire"
)

// Layout constants. Column content widths are sized so the widest honest row
// fits without wrapping (the degraded book message, the confirm preview box).
const (
	bookWidth  = 38
	orderWidth = 36
	rightWidth = 34

	depthBar      = "▊"
	maxBarLen     = int64(24) // depth-bar cap, per specs/2/55-terminal.md
	maxLadderRows = 8         // rows shown per book side
	maxTapeRows   = 10        // trade prints shown

	degradedBookMsg = "no live book — market-data stream down"

	// helpText is the keybinding legend, verbatim per specs/2/55-terminal.md.
	helpText = " q quit  b/s side  t tif  r ro  p po  tab field  0-9 type  ⌫ del  enter submit  c cancel  F3 trace "
)

// View renders the whole terminal: status bar / three-column main / speed
// strip / status line / help line. Mirrors rsx-tui/src/render.rs.
func (m Model) View() string {
	return lipgloss.JoinVertical(
		lipgloss.Left,
		m.viewStatusBar(),
		m.viewMain(),
		m.viewSpeed(),
		m.viewStatusLine(),
		viewHelp(),
	)
}

// panel wraps a title (first line, muted) and body rows in the standard
// bordered panel at the given content width.
func panel(title string, width int, rows ...string) string {
	lines := append([]string{StyleMuted.Render(title)}, rows...)
	body := lipgloss.JoinVertical(lipgloss.Left, lines...)
	return PanelStyle.Width(width).Render(body)
}

func (m Model) viewStatusBar() string {
	badge := lipgloss.NewStyle().
		Foreground(ColorPageBg).
		Background(ColorHeading).
		Bold(true).
		Render(fmt.Sprintf(" RSX  %s ", m.cfg.Symbol))

	link := StyleLive.Render("● live")
	if !m.gwConnected {
		link = StyleDegraded.Render("● offline")
	}

	counts := StyleMuted.Render(fmt.Sprintf("open %d  fills %d", len(m.openOrders), m.fills))

	last := "—"
	if e, ok := m.tape.Last(); ok {
		last = fmt.Sprintf("%d", e.Px)
	}
	// mark is book-mid-derived, not exchange-authoritative; index and
	// funding have no source (always —). The "~" prefix + dim/italic style
	// (StyleDerived) mark it as a client-side estimate, never truth.
	markLabel := "~mark — (mid)"
	if mid, ok := m.book.Mid(); ok {
		markLabel = fmt.Sprintf("~mark %d (mid)", mid)
	}
	market := StyleMuted.Render(fmt.Sprintf("last %s  ", last)) +
		StyleDerived.Render(markLabel) +
		StyleMuted.Render("  index —  funding —")

	return lipgloss.JoinHorizontal(lipgloss.Top, badge, "  ", link, "  ", counts, "  ", market)
}

func (m Model) viewMain() string {
	left := m.viewBook()
	mid := m.viewOrder()
	var right string
	if m.showTrace {
		right = m.viewTrace()
	} else {
		right = lipgloss.JoinVertical(lipgloss.Left, m.viewPositions(), m.viewTrades())
	}
	return lipgloss.JoinHorizontal(lipgloss.Top, left, mid, right)
}

// viewBook draws the ladder: asks (red) worst-first down to the spread row,
// then bids (green) best-first. Degraded to an amber row when the ladder is
// empty or the marketdata link is down (specs/2/55-terminal.md).
func (m Model) viewBook() string {
	var rows []string
	if m.book.Empty() || !m.mdConnected {
		rows = append(rows, StyleDegraded.Render(degradedBookMsg))
		return panel(" book ", bookWidth, rows...)
	}

	asks := m.book.Asks
	an := len(asks)
	if an > maxLadderRows {
		an = maxLadderRows
	}
	// Best an asks, printed worst-first so the best ask sits above the spread.
	for i := an - 1; i >= 0; i-- {
		rows = append(rows, levelRow(asks[i], ColorAsk))
	}

	rows = append(rows, StyleMuted.Render(fmt.Sprintf("— %d —", m.book.Spread())))

	bids := m.book.Bids
	bn := len(bids)
	if bn > maxLadderRows {
		bn = maxLadderRows
	}
	for i := 0; i < bn; i++ {
		rows = append(rows, levelRow(bids[i], ColorBid))
	}

	return panel(" book ", bookWidth, rows...)
}

// levelRow renders "<px> <qty> <bar>" with the px and bar coloured by side.
func levelRow(l wire.Level, color lipgloss.Color) string {
	n := max(int64(0), min(l.Qty, maxBarLen))
	bar := strings.Repeat(depthBar, int(n))
	side := lipgloss.NewStyle().Foreground(color)
	return fmt.Sprintf("%s %s %s",
		side.Render(fmt.Sprintf("%d", l.Px)),
		fmt.Sprintf("%d", l.Qty),
		side.Render(bar),
	)
}

func (m Model) viewOrder() string {
	buy := lipgloss.NewStyle().Foreground(ColorBid)
	sell := lipgloss.NewStyle().Foreground(ColorAsk)
	if m.side == wire.Sell {
		sell = sell.Reverse(true)
	} else {
		buy = buy.Reverse(true)
	}
	sideLine := buy.Render("  BUY  ") + "  " + sell.Render("  SELL  ")

	pxLine := fieldLine("price", m.pxBuf, m.focus == FocusPx)
	qtyLine := fieldLine("qty  ", m.qtyBuf, m.focus == FocusQty)
	tifLine := fmt.Sprintf("time-in-force: %s", m.tif.Label())
	flagLine := fmt.Sprintf("reduce-only: %s   post-only: %s", onOff(m.reduceOnly), onOff(m.postOnly))

	tail := StyleMuted.Render("enter → preview, enter again to send")
	if m.pendingConfirm != nil {
		tail = m.viewConfirm()
	}

	return panel(" order ", orderWidth, sideLine, "", pxLine, qtyLine, tifLine, flagLine, "", tail)
}

// fieldLine renders one labelled form field. The focused one is bold + bright
// with a trailing "_" cursor; the other is muted.
func fieldLine(label, value string, focused bool) string {
	if focused {
		return StyleTextBright.Bold(true).Render(fmt.Sprintf("%s: %s_", label, value))
	}
	return StyleMuted.Render(fmt.Sprintf("%s: %s", label, value))
}

func onOff(b bool) string {
	if b {
		return "on"
	}
	return "off"
}

// viewConfirm renders the ring-bordered submit preview (specs/2/55-terminal.md
// order lifecycle & confirmation). This is always the first thing enter shows
// for a fresh order — handleEnter only ever sets pendingConfirm here, never
// submits without it having rendered first. liq has no source yet: shown as a
// deliberate "n/a", with the reason in one shared legend line rather than
// repeated inline per field.
func (m Model) viewConfirm() string {
	o := *m.pendingConfirm
	side := lipgloss.NewStyle().Foreground(sideColor(o.Side))
	line1 := fmt.Sprintf("confirm %s %d @ %d", side.Render(o.Side.Label()), o.Qty, o.Px)
	line2 := fmt.Sprintf("notional %d  %s  ro:%s po:%s", o.Px*o.Qty, o.Tif.Label(), onOff(o.ReduceOnly), onOff(o.PostOnly))
	line3 := StyleMuted.Render("liq  n/a")
	legend := StyleMuted.Render("n/a fields need server support, not yet wired")
	line4 := StyleHeading.Bold(true).Render("enter again to SEND") + StyleMuted.Render(" · esc cancel")
	body := lipgloss.JoinVertical(lipgloss.Left, line1, line2, line3, legend, line4)
	return RingPanelStyle.Render(body)
}

func sideColor(s wire.Side) lipgloss.Color {
	if s == wire.Sell {
		return ColorAsk
	}
	return ColorBid
}

// viewPositions draws the client-derived position (mark=mid, labelled). uPnL
// is derived from that same client-side mark, so its header carries the "~"
// derived-value marker too. With no mid to price it against, it shows the
// reason rather than a bare dash (mirrors the amber "no live book" caption).
func (m Model) viewPositions() string {
	header := StyleMuted.Render("sym  side  net  entry  ~uPnL")
	if m.position.Flat() {
		return panel(" positions (mark=mid) ", rightWidth, header, StyleMuted.Render("no position — fills build it"))
	}

	net := m.position.Net
	word, wordStyle := "LONG", StyleLive
	if net < 0 {
		word, wordStyle = "SHORT", StyleAsk
	}

	entry := "—"
	if e, ok := m.position.Entry(); ok {
		entry = fmt.Sprintf("%d", e)
	}

	upnl := "—  (needs live book)"
	upnlStyle := StyleDegraded
	if mid, ok := m.book.Mid(); ok {
		if u, ok := m.position.Upnl(mid); ok {
			upnl = signed(u)
			upnlStyle = StyleLive
			if u < 0 {
				upnlStyle = StyleAsk
			}
		}
	}

	row := fmt.Sprintf("%s  %s  %s  %s  %s",
		m.cfg.Symbol,
		wordStyle.Render(word),
		signed(net),
		entry,
		upnlStyle.Render(upnl),
	)
	return panel(" positions (mark=mid) ", rightWidth, header, row)
}

// signed prefixes a "+" for positive values; negatives already carry "-".
func signed(v int64) string {
	if v > 0 {
		return fmt.Sprintf("+%d", v)
	}
	return fmt.Sprintf("%d", v)
}

// viewTrades draws the public tape, newest first, price coloured by taker
// side. Each print is also prefixed with a B/S glyph so the side reads
// without relying on colour.
func (m Model) viewTrades() string {
	var rows []string
	for i, e := range m.tape.Entries() {
		if i >= maxTapeRows {
			break
		}
		glyph := "B"
		if e.Side == wire.Sell {
			glyph = "S"
		}
		style := lipgloss.NewStyle().Foreground(sideColor(e.Side))
		rows = append(rows, style.Render(fmt.Sprintf("%s %d %d", glyph, e.Px, e.Qty)))
	}
	if len(rows) == 0 {
		rows = append(rows, StyleMuted.Render("no trades yet"))
	}
	return panel(" trades ", rightWidth, rows...)
}

// viewTrace is the diagnostic HUD. lipgloss has no Clear-style overlay
// compositing like ratatui's Clear widget, so we swap the whole right column
// for the trace panel — the honest simple equivalent of the Rust trace overlay
// (rsx-tui/src/render.rs draw_trace_hud). This is an intentional, noted
// deviation.
func (m Model) viewTrace() string {
	row := func(k, v string) string {
		return StyleMuted.Render(fmt.Sprintf("%-9s", k)) + v
	}
	p50 := "—"
	if v, ok := m.latWindow.P50(); ok {
		p50 = fmtNs(v)
	}
	best := "—"
	if v, ok := m.latWindow.Min(); ok {
		best = fmtNs(v)
	}

	rows := []string{
		StyleHeading.Bold(true).Render("TRACE — F3 to hide"),
		row("endpoint", m.cfg.Endpoint),
		row("md", m.cfg.MdEndpoint),
		row("link", linkWord(m.gwConnected)),
		row("md link", linkWord(m.mdConnected)),
		row("rtt p50", p50),
		row("rtt min", best),
		row("open", fmt.Sprintf("%d", len(m.openOrders))),
		row("fills", fmt.Sprintf("%d", m.fills)),
		row("spread", fmt.Sprintf("%d", m.book.Spread())),
		row("depth", fmt.Sprintf("%d bid / %d ask", len(m.book.Bids), len(m.book.Asks))),
		row("last", m.status),
	}
	body := lipgloss.JoinVertical(lipgloss.Left, rows...)
	return RingPanelStyle.Width(rightWidth).Render(body)
}

func linkWord(up bool) string {
	if up {
		return "connected"
	}
	return "down"
}

// viewSpeed is the ⚡ round-trip strip: last RTT split into net / internal /
// engine (each leg "—" when unmeasured), plus rolling p50 / best. Dim until
// the first sample arrives.
func (m Model) viewSpeed() string {
	if m.lastLat == nil {
		return StyleMuted.Render(" ⚡ latency: waiting for first round-trip…")
	}
	l := *m.lastLat
	rtt := StyleHeading.Bold(true).Render(fmt.Sprintf(" ⚡ RTT %s ", fmtNs(l.TotalNs)))
	legs := StyleText.Render(fmt.Sprintf("= net %s + internal %s + engine %s",
		fmtNsOrDash(l.NetNs), fmtNsOrDash(l.InternalNs), fmtNsOrDash(l.EngineNs)))

	p50 := "—"
	if v, ok := m.latWindow.P50(); ok {
		p50 = fmtNs(v)
	}
	best := "—"
	if v, ok := m.latWindow.Min(); ok {
		best = fmtNs(v)
	}
	stats := StyleMuted.Render(fmt.Sprintf("   p50 %s · best %s", p50, best))
	return rtt + legs + stats
}

func (m Model) viewStatusLine() string {
	return StyleTextBright.Render(" " + m.status)
}

func viewHelp() string {
	return StyleMuted.Render(helpText)
}
