package ui

import (
	"fmt"
	"strings"
	"time"

	"github.com/charmbracelet/lipgloss"

	"rsx-term/book"
	"rsx-term/wire"
)

// Layout constants. Column content widths are sized so the widest honest row
// fits without wrapping (the degraded book message, the confirm preview box).
const (
	bookWidth  = 38
	orderWidth = 36
	rightWidth = 34
	// traceWidth is wider than rightWidth: the F3 HUD carries the pending-leg
	// legend line (bug IDs), which doesn't fit rightWidth without an ugly
	// mid-word wrap. Only the trace panel uses it — it swaps the whole right
	// column rather than sitting beside the other two (see viewMain).
	traceWidth = 46

	depthBar      = "▊"
	maxBarLen     = int64(24) // depth-bar cap, per specs/2/55-terminal.md
	maxLadderRows = 8         // rows shown per book side, before a WindowSizeMsg
	maxTapeRows   = 10        // trade prints shown, before a WindowSizeMsg
	sparkSamples  = 32        // how much of the rolling window the sparkline shows

	// mdStaleThreshold is how long since the last marketdata frame before the
	// book is flagged stale (amber) rather than merely "not the newest".
	mdStaleThreshold = 2 * time.Second

	degradedBookMsg = "no live book — market-data stream down"

	// helpText is the keybinding legend. Verbatim per specs/2/55-terminal.md
	// except for "x flatten", added ahead of the spec's next revision.
	helpText = " q quit  b/s side  t tif  r ro  p po  tab field  0-9 type  ⌫ del  enter submit  ↑↓ sel  c cancel  X all  x flatten  F3 trace "
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
		// positions, then working orders (only when you have some — no dead
		// panel when flat), then the tape.
		panels := []string{m.viewPositions()}
		if len(m.openOrders) > 0 {
			panels = append(panels, m.viewOpenOrders())
		}
		panels = append(panels, m.viewTrades())
		right = lipgloss.JoinVertical(lipgloss.Left, panels...)
	}
	return lipgloss.JoinHorizontal(lipgloss.Top, left, mid, right)
}

// ladderRows derives how many levels to show per book side from the
// terminal height, so a tall terminal shows a deeper book instead of a wall
// of empty space and a short one clips gracefully instead of a fixed 8.
// m.height == 0 means no WindowSizeMsg has landed yet — fall back to the
// spec default rather than guess.
func (m Model) ladderRows() int {
	if m.height <= 0 {
		return maxLadderRows
	}
	// Chrome outside the book panel's own level rows: status bar, the two
	// speed-strip lines, status line, help line, plus the book panel's own
	// title + top/bottom border + spread-divider row.
	const chrome = 1 + 2 + 1 + 1 + 4
	return clamp((m.height-chrome)/2, 1, 20)
}

// tapeRows derives how many trade prints to show from the terminal height,
// the trades-panel sibling of ladderRows. Same before-first-resize fallback.
func (m Model) tapeRows() int {
	if m.height <= 0 {
		return maxTapeRows
	}
	// Chrome outside the trades panel's own rows: the same top-level chrome
	// as ladderRows, plus the positions panel above trades (title + border +
	// header + one row) and the trades panel's own title + border.
	const chrome = 1 + 2 + 1 + 1 + 4 + 3
	return clamp(m.height-chrome, 1, 20)
}

// narrow reports whether the terminal is known to be too narrow for the full
// three-column layout. m.width == 0 (no WindowSizeMsg yet) is never treated
// as narrow — there's nothing to degrade against yet.
func (m Model) narrow() bool {
	const layoutWidth = bookWidth + orderWidth + rightWidth
	return m.width > 0 && m.width < layoutWidth
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

	rowsWanted := m.ladderRows()
	asks := m.book.Asks
	an := len(asks)
	if an > rowsWanted {
		an = rowsWanted
	}
	bids := m.book.Bids
	bn := len(bids)
	if bn > rowsWanted {
		bn = rowsWanted
	}

	// Right-align every level's px and qty to the widest value currently on
	// screen, so the ladder reads as rigid columns instead of going ragged
	// the moment a price crosses a digit boundary.
	var pxs, qtys []int64
	for _, l := range asks[:an] {
		pxs, qtys = append(pxs, l.Px), append(qtys, l.Qty)
	}
	for _, l := range bids[:bn] {
		pxs, qtys = append(pxs, l.Px), append(qtys, l.Qty)
	}
	pxW, qtyW := colWidth(5, pxs...), colWidth(2, qtys...)
	showBar := !m.narrow()

	// Draw the trader's own resting orders and the last print into the ladder
	// (research: orders-in-the-ladder is the highest-value pro-DOM feature).
	ownBids, ownAsks := m.ownOrderLevels()
	lastPx, hasLast := int64(0), false
	if e, ok := m.tape.Last(); ok {
		lastPx, hasLast = e.Px, true
	}

	// Best an asks, printed worst-first so the best ask sits above the spread.
	for i := an - 1; i >= 0; i-- {
		mark := levelMarker(asks[i].Px, ownAsks, lastPx, hasLast)
		rows = append(rows, levelRow(asks[i], ColorAsk, pxW, qtyW, showBar, mark))
	}

	// The spread row is the highest-frequency read on a ladder — bright/bold
	// so a trader's eye snaps to it, same StyleTextBright as a focused field
	// (no new colour meaning, just more weight).
	rows = append(rows, StyleTextBright.Bold(true).Render(fmt.Sprintf(" — %d —", m.book.Spread())))

	for i := 0; i < bn; i++ {
		mark := levelMarker(bids[i].Px, ownBids, lastPx, hasLast)
		rows = append(rows, levelRow(bids[i], ColorBid, pxW, qtyW, showBar, mark))
	}

	return panel(" book ", bookWidth, rows...)
}

// levelRow renders "<marker><px> <qty> <bar>" with px, qty right-aligned to
// pxW/qtyW and the px + bar coloured by side. `marker` is a 1-wide column
// (own-order / last-trade cue, or a space). The bar is dropped (not just
// blanked) when showBar is false — a narrow terminal degrades by shedding the
// widest element rather than wrapping mid-row.
func levelRow(l wire.Level, color lipgloss.Color, pxW, qtyW int, showBar bool, marker string) string {
	side := lipgloss.NewStyle().Foreground(color)
	row := fmt.Sprintf("%s%s %*d",
		marker,
		side.Render(fmt.Sprintf("%*d", pxW, l.Px)),
		qtyW, l.Qty,
	)
	if !showBar {
		return row
	}
	n := max(int64(0), min(l.Qty, maxBarLen))
	bar := strings.Repeat(depthBar, int(n))
	return row + " " + side.Render(bar)
}

// ownOrderLevels splits this session's working orders into the set of bid /
// ask prices at which the trader has a resting order, so the ladder can mark
// "my order is here" — the single highest-value pro-DOM cue.
func (m Model) ownOrderLevels() (bids, asks map[int64]bool) {
	bids, asks = map[int64]bool{}, map[int64]bool{}
	for _, o := range m.openOrders {
		if o.Side == wire.Buy {
			bids[o.Px] = true
		} else {
			asks[o.Px] = true
		}
	}
	return
}

// levelMarker is the 1-wide ladder cue for a price: a violet ▸ where the
// trader has a working order (priority), else a bright ‹ at the last-trade
// price, else blank. No new colour meaning — accent = your marker, bright =
// the last print.
func levelMarker(px int64, own map[int64]bool, lastPx int64, hasLast bool) string {
	if own[px] {
		return StyleAccent.Render("▸")
	}
	if hasLast && px == lastPx {
		return StyleTextBright.Render("‹")
	}
	return " "
}

// viewOpenOrders draws the trader's working orders — side, price, qty —
// right-aligned. Only rendered when there are orders (viewMain). The newest
// (what `c` cancels today) is the last row; each row is also marked in the
// ladder by ownOrderLevels.
func (m Model) viewOpenOrders() string {
	header := StyleMuted.Render("side  px  qty")
	var pxs, qtys []int64
	for _, o := range m.openOrders {
		pxs, qtys = append(pxs, o.Px), append(qtys, o.Qty)
	}
	pxW, qtyW := colWidth(5, pxs...), colWidth(2, qtys...)
	sel := m.clampSel(m.orderSel)
	rows := []string{header}
	for i, o := range m.openOrders {
		word, st := "BUY ", StyleLive
		if o.Side == wire.Sell {
			word, st = "SELL", StyleAsk
		}
		// ▸ marks the order `c` will cancel (↑/↓ move it); no blind cancel.
		cursor := "  "
		if i == sel {
			cursor = StyleAccent.Render("▸ ")
		}
		rows = append(rows, fmt.Sprintf("%s%s %*d %*d", cursor, st.Render(word), pxW, o.Px, qtyW, o.Qty))
	}
	return panel(" orders ", rightWidth, rows...)
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
	notional := o.Px * o.Qty
	notionalW := colWidth(8, notional)
	line1 := fmt.Sprintf("confirm %s %d @ %d", side.Render(o.Side.Label()), o.Qty, o.Px)
	line2 := fmt.Sprintf("notional %*d  %s  ro:%s po:%s", notionalW, notional, o.Tif.Label(), onOff(o.ReduceOnly), onOff(o.PostOnly))
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
	entries := m.tape.Entries()
	n := len(entries)
	if rowsWanted := m.tapeRows(); n > rowsWanted {
		n = rowsWanted
	}
	entries = entries[:n]

	var pxs, qtys []int64
	for _, e := range entries {
		pxs, qtys = append(pxs, e.Px), append(qtys, e.Qty)
	}
	pxW, qtyW := colWidth(5, pxs...), colWidth(2, qtys...)

	var rows []string
	for _, e := range entries {
		glyph := "B"
		if e.Side == wire.Sell {
			glyph = "S"
		}
		style := lipgloss.NewStyle().Foreground(sideColor(e.Side))
		rows = append(rows, style.Render(fmt.Sprintf("%s %*d %*d", glyph, pxW, e.Px, qtyW, e.Qty)))
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
// legValue renders one RTT leg's value: the real duration in StyleText, or a
// dim-italic "·· pending" (StyleDerived) for a leg the live server doesn't
// stamp yet (book.NsUnknown). Never a bare dash and never a fabricated
// number — a placeholder reads as "coming", not "broken" or "zero".
func legValue(ns int64) string {
	if ns == book.NsUnknown {
		return StyleDerived.Render("·· pending")
	}
	return StyleText.Render(fmtNs(ns))
}

// legLabel is legValue prefixed with the leg's name, for the one-line ⚡ strip.
func legLabel(name string, ns int64) string {
	if ns == book.NsUnknown {
		return StyleDerived.Render(fmt.Sprintf("%s ·· pending", name))
	}
	return StyleText.Render(fmt.Sprintf("%s %s", name, fmtNs(ns)))
}

// windowStats reads p50 / p99 / best off a book.Window as display strings,
// "—" for a leg with no samples yet. Shared by the speed strip and the trace
// HUD so both read the same numbers off the same window.
func windowStats(w book.Window) (p50, p99, best string) {
	p50, p99, best = "—", "—", "—"
	if v, ok := w.P50(); ok {
		p50 = fmtNs(v)
	}
	if v, ok := w.P99(); ok {
		p99 = fmtNs(v)
	}
	if v, ok := w.Min(); ok {
		best = fmtNs(v)
	}
	return p50, p99, best
}

// fmtAge renders a wall-clock duration: fmtNs's ns/µs/ms ladder for anything
// under a second, else a plain "5.05s" (fmtNs has no seconds rung — md
// staleness routinely runs into seconds, RTT legs never do).
func fmtAge(age time.Duration) string {
	if age >= time.Second {
		return age.Round(10 * time.Millisecond).String()
	}
	return fmtNs(age.Nanoseconds())
}

// mdAgeRow renders the current marketdata-path latency (client_now minus the
// frame's server ts_ns — real and client-measured, not a placeholder: every
// md frame carries ts_ns). "no frame yet" before the first one arrives.
func (m Model) mdAgeRow() string {
	if m.lastMdAgeNs == book.NsUnknown {
		return StyleMuted.Render("no frame yet")
	}
	return fmtNs(m.lastMdAgeNs)
}

// mdAgeStatsRow renders the rolling p50/p99/best of the md-path latency —
// the same rolling-window treatment as the RTT legs, over the md age window.
func (m Model) mdAgeStatsRow() string {
	if m.lastMdAgeNs == book.NsUnknown {
		return StyleMuted.Render("—")
	}
	p50, p99, best := windowStats(m.mdAgeWindow)
	return fmt.Sprintf("p50 %s  p99 %s  best %s", p50, p99, best)
}

// mdStaleRow renders how long since the last marketdata frame arrived —
// amber past mdStaleThreshold, pairing with the degraded "no live book" row.
func (m Model) mdStaleRow() string {
	if m.lastMdAt.IsZero() {
		return StyleMuted.Render("no frame yet")
	}
	age := time.Since(m.lastMdAt)
	txt := fmtAge(age) + " ago"
	if age > mdStaleThreshold {
		return StyleDegraded.Render(txt + " — STALE")
	}
	return StyleText.Render(txt)
}

func (m Model) viewTrace() string {
	row := func(k, v string) string {
		return StyleMuted.Render(fmt.Sprintf("%-9s", k)) + v
	}
	section := func(name string) string {
		return StyleHeading.Render(name)
	}

	netLeg, intLeg, engLeg := legValue(book.NsUnknown), legValue(book.NsUnknown), legValue(book.NsUnknown)
	pendingLegs := true
	if m.lastLat != nil {
		l := *m.lastLat
		netLeg, intLeg, engLeg = legValue(l.NetNs), legValue(l.InternalNs), legValue(l.EngineNs)
		pendingLegs = l.InternalNs == book.NsUnknown || l.EngineNs == book.NsUnknown
	}
	p50, p99, best := windowStats(m.latWindow)
	spark := StyleMuted.Render("(no samples yet)")
	if s := sparkline(m.latWindow.Recent(sparkSamples)); s != "" {
		spark = StyleAccent.Render(s)
	}

	rows := []string{
		StyleHeading.Bold(true).Render("TRACE — F3 to hide"),
		"",
		section("LINKS"),
		row("gw", fmt.Sprintf("%s  %s", linkWord(m.gwConnected), m.cfg.Endpoint)),
		row("md", fmt.Sprintf("%s  %s", linkWord(m.mdConnected), m.cfg.MdEndpoint)),
		"",
		section("LATENCY  (round-trip)"),
		row("net", netLeg),
		row("internal", intLeg),
		row("engine", engLeg),
		row("p50", fmt.Sprintf("%s   p99 %s   best %s", p50, p99, best)),
		row("samples", fmt.Sprintf("%d", m.latWindow.Len())),
		row("recent", spark),
		"",
		section("LATENCY  (marketdata)"),
		row("age", m.mdAgeRow()),
		row("since", m.mdStaleRow()),
		"",
		section("FLOW"),
		row("open", fmt.Sprintf("%d", len(m.openOrders))),
		row("fills", fmt.Sprintf("%d", m.fills)),
		row("spread", fmt.Sprintf("%d", m.book.Spread())),
		row("depth", fmt.Sprintf("%d bid / %d ask", len(m.book.Bids), len(m.book.Asks))),
		row("last", m.status),
	}
	if pendingLegs {
		rows = append(rows, "",
			StyleMuted.Render("pending: internal→GW-STAMP-LATENCY-LIVE"),
			StyleMuted.Render("         engine→ME-ENGINE-LATENCY-NOT-REPORTED"))
	}
	body := lipgloss.JoinVertical(lipgloss.Left, rows...)
	return RingPanelStyle.Width(traceWidth).Render(body)
}

func linkWord(up bool) string {
	if up {
		return "connected"
	}
	return "down"
}

// viewSpeed is the ⚡ round-trip strip: last RTT split into net / internal /
// engine (a leg the live server doesn't stamp yet renders as a dim-italic
// "·· pending", never a fabricated number or a bare dash — see legValue),
// plus rolling p50 / p99 / best and a sparkline of the recent RTT window —
// the "watch the latency live" touch. A trailing marketdata-path indicator
// (client-measured frame age, real not placeholder) rides on the same line
// when a frame has arrived. Dim until the first RTT sample arrives.
func (m Model) viewSpeed() string {
	if m.lastLat == nil {
		return StyleMuted.Render(" ⚡ latency: waiting for first round-trip…")
	}
	l := *m.lastLat
	rtt := StyleHeading.Bold(true).Render(fmt.Sprintf(" ⚡ RTT %s ", fmtNs(l.TotalNs)))
	legs := StyleText.Render("= ") +
		legLabel("net", l.NetNs) + StyleText.Render(" + ") +
		legLabel("internal", l.InternalNs) + StyleText.Render(" + ") +
		legLabel("engine", l.EngineNs)

	p50, p99, best := windowStats(m.latWindow)
	spark := sparkline(m.latWindow.Recent(sparkSamples))
	stats := StyleMuted.Render(fmt.Sprintf("   p50 %s · p99 %s · best %s  ", p50, p99, best)) +
		StyleAccent.Render(spark) +
		mdIndicator(m)

	return lipgloss.JoinVertical(lipgloss.Left, rtt+legs, stats)
}

// mdIndicator is the compact marketdata-freshness tag appended to the speed
// strip's stats line: the current frame age when fresh, or an amber "md
// stale" flag past mdStaleThreshold. Empty before the first md frame arrives
// (nothing to report yet, not a placeholder).
func mdIndicator(m Model) string {
	if m.lastMdAt.IsZero() {
		return ""
	}
	if age := time.Since(m.lastMdAt); age > mdStaleThreshold {
		return StyleDegraded.Render(fmt.Sprintf("· md stale %s", fmtNs(age.Nanoseconds())))
	}
	if m.lastMdAgeNs == book.NsUnknown {
		return ""
	}
	return StyleMuted.Render("· md " + fmtNs(m.lastMdAgeNs))
}

func (m Model) viewStatusLine() string {
	return StyleTextBright.Render(" " + m.status)
}

func viewHelp() string {
	return StyleMuted.Render(helpText)
}
