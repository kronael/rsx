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
	bookWidth  = 44
	orderWidth = 36
	rightWidth = 34
	// traceWidth is wider than rightWidth: the F3 HUD carries the pending-leg
	// legend line (bug IDs), which doesn't fit rightWidth without an ugly
	// mid-word wrap. Only the trace panel uses it — it swaps the whole right
	// column rather than sitting beside the other two (see viewMain).
	traceWidth = 46

	depthGlyph    = "▊" // per-level depth-bar cell, per specs/2/55-terminal.md
	ladderBarW    = 6   // depth-bar cells per side (bid grows left, ask right)
	maxLadderRows = 8   // rows shown per book side, before a WindowSizeMsg
	maxTapeRows   = 10  // trade prints shown, before a WindowSizeMsg
	sparkSamples  = 32  // how much of the rolling window the sparkline shows

	// mdStaleThreshold is how long since the last marketdata frame before the
	// book is flagged stale (amber) rather than merely "not the newest".
	mdStaleThreshold = 2 * time.Second

	degradedBookMsg = "no live book — market-data stream down"

	// helpText is the keybinding legend. Verbatim per specs/2/55-terminal.md
	// except for "x flatten", added ahead of the spec's next revision.
	helpText = " q quit  b/s side  t tif  r ro  p po  +/- tick  j/k join  tab field  0-9 type  ⌫ del  enter submit  m mkt  ↑↓ sel  c cancel  X all  x flatten  R reverse  F2 armed  F3 trace  ? help "
)

// View renders the whole terminal: status bar / three-column main / speed
// strip / status line / help line. Mirrors rsx-tui/src/render.rs.
func (m Model) View() string {
	if m.showHelp {
		return m.viewHelpOverlay()
	}
	rows := []string{m.viewStatusBar()}
	if b := m.viewArmedBanner(); b != "" {
		rows = append(rows, b)
	}
	rows = append(rows, m.viewMain(), m.viewSpeed(), m.viewStatusLine(), viewHelp())
	return lipgloss.JoinVertical(lipgloss.Left, rows...)
}

// viewArmedBanner is the persistent confirm-off warning line: empty when the
// two-enter confirm is on (the safe default), a loud red banner when ARMED so
// the trader can never forget the guardrail is down. F2 toggles it.
func (m Model) viewArmedBanner() string {
	if !m.armed {
		return ""
	}
	return StyleArmed.Render(" ⚠ ARMED — confirm OFF, orders fire on one enter · F2 to re-arm safety ")
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
		last = m.fmtPx(e.Px)
	}
	// mark is book-mid-derived, not exchange-authoritative; index and
	// funding have no source (always —). The "~" prefix + dim/italic style
	// (StyleDerived) mark it as a client-side estimate, never truth.
	markLabel := "~mark — (mid)"
	if mid, ok := m.book.Mid(); ok {
		markLabel = fmt.Sprintf("~mark %s (mid)", m.fmtPx(mid))
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
	if m.narrow() {
		// Too narrow for three columns side by side: stacking them keeps every
		// panel on-screen (each is <= bookWidth), instead of the terminal
		// clipping the right column off the edge. Order form first — it's what
		// you act with — then the book, then positions/orders/tape.
		return lipgloss.JoinVertical(lipgloss.Left, mid, left, right)
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

// resolvedCenter is the ladder's centre price: the sticky ladderCenter, else
// the current mid, else whichever side has liquidity, else 0. Shared by viewBook
// and the mouse click-to-price mapping so the two never drift apart.
func (m Model) resolvedCenter() int64 {
	if m.ladderCenter != 0 {
		return m.ladderCenter
	}
	if mid, ok := m.book.Mid(); ok {
		return mid
	}
	if len(m.book.Asks) > 0 {
		return m.book.Asks[0].Px
	}
	if len(m.book.Bids) > 0 {
		return m.book.Bids[0].Px
	}
	return 0
}

// priceAtY maps a screen row to the ladder price rendered on it, for mouse
// click-to-price. It mirrors viewBook's row layout exactly: status bar (1) +
// optional ARMED banner (1) + the book panel's top border (1) + title (1),
// then one row per level from center+half down to center-half. Returns false
// off the ladder, in the stacked (narrow) layout where these offsets differ,
// or with no live book.
func (m Model) priceAtY(y int) (int64, bool) {
	if m.narrow() || m.book.Empty() || !m.mdConnected {
		return 0, false
	}
	firstLevelY := 3 // status bar + book top border + title
	if m.armed {
		firstLevelY++
	}
	i := y - firstLevelY
	half := m.ladderRows()
	if i < 0 || i > 2*half {
		return 0, false
	}
	price := m.resolvedCenter() + int64(half) - int64(i)
	if price <= 0 {
		return 0, false
	}
	return price, true
}

// viewBook draws a STATIC price ladder: a fixed price axis centred on
// ladderCenter (recentred only on drift — recenterLadder), bid qty on the
// left of the price column, ask qty on the right, so the axis does not
// reshuffle on every tick (TT/Sierra pattern). Empty prices are gaps — the
// spread reads as the blank band between best bid and best ask. The trader's
// own orders (▸) and the last print (‹) are marked on their rows. Degrades to
// an amber row when the ladder is empty or marketdata is down.
func (m Model) viewBook() string {
	if m.book.Empty() || !m.mdConnected {
		return panel(" book ", bookWidth, StyleDegraded.Render(degradedBookMsg))
	}

	center := m.resolvedCenter()
	half := int64(m.ladderRows())

	askByPx := map[int64]int64{}
	for _, l := range m.book.Asks {
		askByPx[l.Px] = l.Qty
	}
	bidByPx := map[int64]int64{}
	for _, l := range m.book.Bids {
		bidByPx[l.Px] = l.Qty
	}
	ownBids, ownAsks := m.ownOrderLevels()
	lastPx, hasLast := int64(0), false
	if e, ok := m.tape.Last(); ok {
		lastPx, hasLast = e.Px, true
	}

	pxW := strWidth(6, m.fmtPx(center+half), m.fmtPx(center-half))
	var qtyStrs []string
	var maxQty int64
	for _, q := range askByPx {
		qtyStrs = append(qtyStrs, m.fmtQty(q))
		if q > maxQty {
			maxQty = q
		}
	}
	for _, q := range bidByPx {
		qtyStrs = append(qtyStrs, m.fmtQty(q))
		if q > maxQty {
			maxQty = q
		}
	}
	qtyW := strWidth(3, qtyStrs...)

	var rows []string
	for p := center + half; p >= center-half; p-- {
		aQ, aOk := askByPx[p]
		bQ, bOk := bidByPx[p]
		mark := " "
		if ownBids[p] || ownAsks[p] {
			mark = StyleAccent.Render("▸")
		} else if hasLast && p == lastPx {
			mark = StyleTextBright.Render("‹")
		}
		rows = append(rows, m.ladderRow(p, bQ, aQ, maxQty, bOk, aOk, pxW, qtyW, mark))
	}

	// Top-of-book imbalance: bid vs ask share of the visible depth — a fast
	// "where's the pressure" read (research: DOM imbalance drives scalping).
	var tb, ta int64
	for _, q := range bidByPx {
		tb += q
	}
	for _, q := range askByPx {
		ta += q
	}
	if tb+ta > 0 {
		rows = append(rows, imbalanceBar(tb, ta, 20))
	}
	return panel(" book ", bookWidth, rows...)
}

// imbalanceBar renders a green(bid)|red(ask) split bar of the visible depth
// plus the bid share, so a trader sees buying vs selling pressure at a glance.
func imbalanceBar(bid, ask int64, width int) string {
	total := bid + ask
	bw := int(int64(width) * bid / total)
	if bw > width {
		bw = width
	}
	bar := StyleLive.Render(strings.Repeat("█", bw)) + StyleAsk.Render(strings.Repeat("█", width-bw))
	return bar + StyleMuted.Render(fmt.Sprintf(" %d%% bid", int(int64(100)*bid/total)))
}

// ladderRow renders one static-ladder price row:
// "<mark> <bidBar> <bidQ> <price> <askQ> <askBar>" — a depth bar (▊, scaled to
// the deepest visible level) then bid qty (green) left of the decimal price,
// ask qty (red) then its depth bar right, the price coloured by whichever side
// rests there (muted in the empty spread band). Bid depth grows leftward and
// ask depth rightward, so the two bars form a histogram pointing at the spread
// (the DOM/Bookmap depth read). Right-aligned to string widths so the axis
// stays rigid.
func (m Model) ladderRow(p, bidQty, askQty, maxQty int64, bidOk, askOk bool, pxW, qtyW int, mark string) string {
	blank := strings.Repeat(" ", qtyW)
	barBlank := strings.Repeat(" ", ladderBarW)
	bidCol, askCol := blank, blank
	bidBar, askBar := barBlank, barBlank
	if bidOk {
		bidCol = StyleLive.Render(fmt.Sprintf("%*s", qtyW, m.fmtQty(bidQty)))
		bidBar = depthBar(bidQty, maxQty, ladderBarW, StyleLive, true)
	}
	if askOk {
		askCol = StyleAsk.Render(fmt.Sprintf("%*s", qtyW, m.fmtQty(askQty)))
		askBar = depthBar(askQty, maxQty, ladderBarW, StyleAsk, false)
	}
	pxStyle := StyleMuted
	if bidOk {
		pxStyle = StyleLive
	} else if askOk {
		pxStyle = StyleAsk
	}
	return fmt.Sprintf("%s %s %s %s %s %s", mark, bidBar, bidCol,
		pxStyle.Render(fmt.Sprintf("%*s", pxW, m.fmtPx(p))), askCol, askBar)
}

// depthBar renders a proportional depth bar `width` cells wide: qty relative to
// maxQty in ▊ cells, coloured by the side's style, so each level reads as a
// horizontal histogram. leftGrow right-aligns the bar (bid side, grows toward
// the edge); otherwise it's left-aligned (ask side). Any nonzero qty shows at
// least one cell so a thin level never vanishes; an empty level is all spaces.
func depthBar(qty, maxQty int64, width int, style lipgloss.Style, leftGrow bool) string {
	n := 0
	if maxQty > 0 && qty > 0 {
		n = int(int64(width) * qty / maxQty)
		if n == 0 {
			n = 1
		}
		if n > width {
			n = width
		}
	}
	bar := style.Render(strings.Repeat(depthGlyph, n))
	pad := strings.Repeat(" ", width-n)
	if leftGrow {
		return pad + bar
	}
	return bar + pad
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

// viewHelpOverlay is the modal `?` key reference, grouped by function with
// destructive keys (cancel / flatten / quit) in the ask/red style so they're
// visibly the dangerous ones. Any key closes it (handleKey).
func (m Model) viewHelpOverlay() string {
	grp := func(h string) string { return StyleHeading.Render(h) }
	key := func(k, d string) string {
		return StyleTextBright.Render(fmt.Sprintf("  %-8s", k)) + StyleMuted.Render(d)
	}
	danger := func(k, d string) string {
		return StyleAsk.Render(fmt.Sprintf("  %-8s", k)) + StyleMuted.Render(d)
	}
	rows := []string{
		StyleHeading.Bold(true).Render("KEYS — ? or any key to close"),
		"",
		grp("order entry"),
		key("b / s", "buy / sell side"),
		key("0-9  ⌫", "type / delete the focused field"),
		key("tab", "switch price / qty field"),
		key("t", "cycle time-in-force (GTC / IOC / FOK)"),
		key("r / p", "toggle reduce-only / post-only"),
		key("+ / -", "nudge price one tick (seeds from mid)"),
		key("j / k", "join best bid / ask"),
		key("click", "left-click a ladder row to set its price"),
		key("enter", "preview → enter again to send · esc cancels"),
		key("m", "market — IOC at the far touch"),
		"",
		grp("orders & position"),
		key("↑ / ↓", "move the working-order cursor"),
		danger("c", "cancel the selected order"),
		danger("X", "cancel ALL working orders"),
		danger("x", "flatten — reduce-only close the position"),
		danger("R", "reverse — flip the position (crosses zero)"),
		"",
		grp("view"),
		key("F3", "latency / telemetry trace"),
		danger("F2", "ARMED — toggle confirm OFF (single-enter fire)"),
		key("?", "this help"),
		danger("q", "quit"),
	}
	return RingPanelStyle.Render(lipgloss.JoinVertical(lipgloss.Left, rows...))
}

// viewOpenOrders draws the trader's working orders — side, price, qty —
// right-aligned. Only rendered when there are orders (viewMain). The newest
// (what `c` cancels today) is the last row; each row is also marked in the
// ladder by ownOrderLevels.
func (m Model) viewOpenOrders() string {
	header := StyleMuted.Render("side  px  qty")
	var pxs, qtys []string
	for _, o := range m.openOrders {
		pxs, qtys = append(pxs, m.fmtPx(o.Px)), append(qtys, m.fmtQty(o.Qty))
	}
	pxW, qtyW := strWidth(6, pxs...), strWidth(3, qtys...)
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
		rows = append(rows, fmt.Sprintf("%s%s %*s %*s", cursor, st.Render(word), pxW, m.fmtPx(o.Px), qtyW, m.fmtQty(o.Qty)))
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
// viewConfirm renders the two-enter confirm preview inline within the order
// panel. It is NOT its own bordered box: a nested border wider than orderWidth
// wraps and shatters (the order panel's own border already frames it). A violet
// divider gives it weight; every line fits orderWidth. liq is the one
// server-gated field, marked n/a inline.
func (m Model) viewConfirm() string {
	o := *m.pendingConfirm
	side := lipgloss.NewStyle().Foreground(sideColor(o.Side))
	div := lipgloss.NewStyle().Foreground(ColorRing).Render("── confirm ──────────────────")
	l1 := fmt.Sprintf("%s %s @ %s", side.Render(o.Side.Label()), m.fmtQty(o.Qty), m.fmtPx(o.Px))
	notional := "—"
	if n, ok := safeMul(o.Px, o.Qty); ok {
		notional = m.fmtNotional(n)
	}
	l2 := fmt.Sprintf("notional %s  %s", notional, o.Tif.Label())
	l3 := StyleMuted.Render(fmt.Sprintf("ro:%s  po:%s   liq n/a", onOff(o.ReduceOnly), onOff(o.PostOnly)))
	l4 := StyleHeading.Bold(true).Render("enter again to SEND") + StyleMuted.Render(" · esc")
	return lipgloss.JoinVertical(lipgloss.Left, div, l1, l2, l3, l4)
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
	if m.position.Flat() {
		return panel(" positions (mark=mid) ", rightWidth, StyleMuted.Render("no position — fills build it"))
	}

	net := m.position.Net
	word, wordStyle := "LONG", StyleLive
	if net < 0 {
		word, wordStyle = "SHORT", StyleAsk
	}
	netStr := m.fmtQty(net)
	if net > 0 {
		netStr = "+" + netStr
	}

	entry := "—"
	if e, ok := m.position.Entry(); ok {
		entry = m.fmtPx(e)
	}

	// Identity + size on one line, money on the next, risk on the last — the
	// narrow (rightWidth) panel can't hold them side by side without wrapping,
	// so they stack. Each row is short enough to never wrap.
	sizeRow := fmt.Sprintf("%s  %s @ %s",
		wordStyle.Render(word), netStr, entry)

	upnl := StyleDegraded.Render("~uPnL —") + StyleMuted.Render(" (needs live book)")
	if mid, ok := m.book.Mid(); ok {
		if u, ok := m.position.Upnl(mid); ok {
			v := m.fmtNotional(u)
			st := StyleLive
			if u > 0 {
				v = "+" + v
			} else if u < 0 {
				st = StyleAsk
			}
			upnl = StyleMuted.Render("~uPnL ") + st.Render(v)
		}
	}

	return panel(" positions (mark=mid) ", rightWidth, sizeRow, upnl, m.viewRiskRow())
}

// viewRiskRow is the position's risk surface — liquidation price, return-on-
// equity, and a margin-health bar. Every figure needs the risk engine's
// margin / leverage state, which the terminal has no feed for yet, so each is
// honestly dashed in StyleDerived (the established "not real yet" marking, same
// as a pending latency leg) rather than fabricated. It gives these new-trader
// must-haves a fixed home for the moment the backend lands — the terminal
// shows the whole risk picture's shape, and marks what's still pending.
func (m Model) viewRiskRow() string {
	dash := StyleDerived.Render
	// An all-empty bar: margin health is unknown, so no segment is filled.
	// Dashed, not a fake reading — the trader must not read a health level
	// off a number the terminal doesn't have.
	bar := StyleDerived.Render("░░░░░░░░")
	return dash("liq —") + "   " + dash("ROE —") + "   " +
		StyleMuted.Render("mgn ") + bar + "   " + dash("(needs risk engine)")
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

	var pxs, qtys []string
	for _, e := range entries {
		pxs, qtys = append(pxs, m.fmtPx(e.Px)), append(qtys, m.fmtQty(e.Qty))
	}
	pxW, qtyW := strWidth(6, pxs...), strWidth(3, qtys...)

	var rows []string
	for _, e := range entries {
		glyph := "B"
		if e.Side == wire.Sell {
			glyph = "S"
		}
		style := lipgloss.NewStyle().Foreground(sideColor(e.Side))
		rows = append(rows, style.Render(fmt.Sprintf("%s %*s %*s", glyph, pxW, m.fmtPx(e.Px), qtyW, m.fmtQty(e.Qty))))
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
	return fmt.Sprintf("%s   p99 %s   best %s", p50, p99, best)
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
		row("p50", m.mdAgeStatsRow()),
		row("since", m.mdStaleRow()),
		"",
		section("FLOW"),
		row("open", fmt.Sprintf("%d", len(m.openOrders))),
		row("fills", fmt.Sprintf("%d", m.fills)),
		row("spread", m.fmtPx(m.book.Spread())),
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
// Internal round-trip SLA thresholds (ns): a µs-class GW→ME→GW path. Over
// warn → the RTT reads amber, over crit → red, so the strip is a live health
// light, not just a readout.
const (
	slaWarnNs = 50_000
	slaCritNs = 100_000
)

// hopBar draws a proportional stacked bar of the *known* latency legs
// (net│internal│engine), each coloured by leg — so a trader *sees* where the
// time goes, not just reads three numbers. Pending legs contribute nothing
// (honest): on the mock all three show, live it's the net leg until the
// gateway stamps the rest.
func hopBar(l book.Sample, width int) string {
	type seg struct {
		ns int64
		st lipgloss.Style
	}
	var segs []seg
	add := func(ns int64, st lipgloss.Style) {
		if ns != book.NsUnknown && ns > 0 {
			segs = append(segs, seg{ns, st})
		}
	}
	add(l.NetNs, StyleHeading)
	add(l.InternalNs, StyleAccent)
	add(l.EngineNs, StyleLive)
	var total int64
	for _, s := range segs {
		total += s.ns
	}
	if total <= 0 {
		return ""
	}
	var b strings.Builder
	used := 0
	for i, s := range segs {
		w := int(int64(width) * s.ns / total)
		if i == len(segs)-1 {
			w = width - used // last segment fills the remainder exactly
		}
		if w < 0 {
			w = 0
		}
		b.WriteString(s.st.Render(strings.Repeat("█", w)))
		used += w
	}
	return b.String()
}

func (m Model) viewSpeed() string {
	if m.lastLat == nil {
		return StyleMuted.Render(" ⚡ latency: waiting for first round-trip…")
	}
	l := *m.lastLat
	rttStyle := StyleHeading
	if l.TotalNs > slaCritNs {
		rttStyle = StyleAsk
	} else if l.TotalNs > slaWarnNs {
		rttStyle = StyleDegraded
	}
	rtt := rttStyle.Bold(true).Render(fmt.Sprintf(" ⚡ RTT %s ", fmtNs(l.TotalNs)))
	legs := StyleText.Render("= ") +
		legLabel("net", l.NetNs) + StyleText.Render(" + ") +
		legLabel("internal", l.InternalNs) + StyleText.Render(" + ") +
		legLabel("engine", l.EngineNs)
	bar := hopBar(l, 18)
	if bar != "" {
		legs += StyleText.Render("  ") + bar
	}

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
