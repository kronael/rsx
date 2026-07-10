package ui

import (
	"fmt"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/book"
	"rsx-term/feed"
	"rsx-term/wire"
)

// digitCap bounds each digit buffer so it always parses into an int64.
const digitCap = 18

// Update folds one message into the model. Key messages drive the order form;
// wire.* / feed.* messages fold market data, private events, and link state.
func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch v := msg.(type) {
	case tea.KeyMsg:
		return m.handleKey(v)
	case tea.MouseMsg:
		return m.handleMouse(v)

	case feed.GwUp:
		m.gwConnected = true
		m.status = "connected"
	case feed.GwDown:
		m.gwConnected = false
		m.status = "disconnected"
	case feed.MdUp:
		m.mdConnected = true
	case feed.MdDown:
		m.mdConnected = false

	case wire.Snapshot:
		m.seq.ResetTo(v.Seq)
		m.book.ApplySnapshot(v)
		m.recenterLadder()
		m.foldMdFrame(v.TsNs)
	case wire.Delta:
		m.seq.Observe(v.Seq)
		m.book.ApplyDelta(v)
		m.recenterLadder()
		m.foldMdFrame(v.TsNs)
	case wire.Bbo:
		m.seq.Observe(v.Seq)
		m.book.ApplyBbo(v)
		m.recenterLadder()
		m.foldMdFrame(v.TsNs)
	case wire.MdTrade:
		m.seq.Observe(v.Seq)
		side := wire.Buy
		if v.TakerSide != 0 {
			side = wire.Sell
		}
		entry := book.TapeEntry{Side: side, Px: v.Px, Qty: v.Qty}
		m.tape.Push(entry)
		if m.cfg.Stream {
			m.pendingTrades = append(m.pendingTrades, entry)
		}
		m.foldMdFrame(v.TsNs)

	case wire.Accepted:
		m.openOrders = append(m.openOrders, OpenOrder{
			Oid:  v.Oid,
			Cid:  v.Cid,
			Side: v.Order.Side,
			Px:   v.Order.Px,
			Qty:  v.Order.Qty,
		})
		m.status = fmt.Sprintf("order %d accepted", v.Oid)
		m.foldRtt(v.RttNs)
	case wire.Done:
		m.removeOrder(v.Oid)
		m.status = fmt.Sprintf("order %d done", v.Oid)
		m.foldRtt(v.RttNs)
	case wire.Rejected:
		m.status = "rejected: " + v.Reason
	case wire.Fill:
		m.fills++
		m.position.ApplyFill(v.Side, v.Px, v.Qty)
		m.status = fmt.Sprintf("fill %d: %s @ %s", v.Oid, m.fmtQty(v.Qty), m.fmtPx(v.Px))

	case feed.Latency:
		s := v.Sample
		m.lastLat = &s
		m.latWindow.Add(v.Sample.TotalNs)

	case tea.WindowSizeMsg:
		m.width = v.Width
		m.height = v.Height
		if m.cfg.Stream {
			m.resizeHeat()
		}

	case binTickMsg:
		if m.cfg.Stream {
			m.foldBin(time.Time(v).UnixNano())
			return m, binTickCmd()
		}
	}
	return m, nil
}

// foldBin closes the current heatmap time bin: it folds the live book snapshot
// and the trades accumulated since the last tick into a fresh ring row, then
// clears the pending-trade buffer. No-op until the ring is sized.
func (m *Model) foldBin(binTs int64) {
	if m.heat == nil {
		return
	}
	m.heat.Ingest(m.book.Bids, m.book.Asks, m.pendingTrades, binTs)
	m.pendingTrades = m.pendingTrades[:0]
}

// resizeHeat (re)builds the heatmap ring to fit the terminal. It reallocates on
// a size change (dropping history — acceptable for a live stream), and clears
// the ring when the terminal is too small to render.
func (m *Model) resizeHeat() {
	w, h := streamDims(m.width, m.height)
	if w < 2 || h < 1 {
		m.heat = nil
		return
	}
	if m.heat == nil || m.heat.Width() != w || m.heat.Height() != h {
		tick := m.cfg.Tick
		if tick <= 0 {
			tick = 1
		}
		m.heat = book.NewHeatmap(w, h, tick)
	}
}

// streamDims derives the heatmap's column and row counts from the terminal size:
// one column of news rail on the left, and reserved rows for the header + the
// pinned footer. Width is forced even so the mid splits the axis cleanly.
func streamDims(width, height int) (int, int) {
	const rail = 1
	const reserved = 1 + 5 // header line + 5 footer lines
	w := width - rail
	if w%2 != 0 {
		w--
	}
	h := height - reserved
	if h > 60 {
		h = 60
	}
	return w, h
}

// foldRtt records a measured round-trip from a private event. A negative RttNs
// (RttUnknown) is never measured — it must not fabricate a latency figure.
func (m *Model) foldRtt(rttNs int64) {
	if rttNs < 0 {
		return
	}
	m.latWindow.Add(rttNs)
	m.lastLat = &book.Sample{
		TotalNs:    rttNs,
		NetNs:      book.NsUnknown,
		InternalNs: book.NsUnknown,
		EngineNs:   book.NsUnknown,
	}
}

// foldMdFrame records the client-measured age of an inbound marketdata frame
// and marks its arrival for staleness tracking. Age is wall-clock now minus
// the frame's server ts_ns (Unix epoch nanoseconds — rsx-types::time_utils,
// SystemTime::now().duration_since(UNIX_EPOCH)). tsNs == 0 means the frame
// carries no real timestamp (the offline demo script doesn't stamp one) —
// that's not measurable, so it's left as book.NsUnknown rather than showing
// a fabricated multi-decade age. Arrival time (for staleness) is recorded
// either way, since it just answers "did a frame land."
func (m *Model) foldMdFrame(tsNs uint64) {
	m.lastMdAt = time.Now()
	if tsNs == 0 {
		m.lastMdAgeNs = book.NsUnknown
		return
	}
	age := time.Now().UnixNano() - int64(tsNs)
	if age < 0 {
		age = 0 // clock skew guard — never show a negative age
	}
	m.lastMdAgeNs = age
	m.mdAgeWindow.Add(age)
}

// recenterLadder keeps the static price ladder's centre stable: it moves only
// when the mid drifts outside a small band, so the price axis doesn't
// reshuffle on every tick — the classic DOM anti-pattern (TT/Sierra keep the
// ladder stationary and recenter on demand).
func (m *Model) recenterLadder() {
	mid, ok := m.book.Mid()
	if !ok {
		return
	}
	const band = 6
	d := mid - m.ladderCenter
	if d < 0 {
		d = -d
	}
	if m.ladderCenter == 0 || d > band {
		m.ladderCenter = mid
	}
}

// removeOrder drops any open order with a matching oid (absent = no-op). It
// allocates a fresh slice so it never mutates a retained earlier model's
// backing array.
func (m *Model) removeOrder(oid uint64) {
	var out []OpenOrder
	for _, o := range m.openOrders {
		if o.Oid != oid {
			out = append(out, o)
		}
	}
	m.openOrders = out
}

// handleMouse maps a left-click on a ladder row to that row's price (the mouse
// analog of j/k / +/-). Only a left button-press inside the book column counts;
// motion, other buttons, and clicks outside the ladder are ignored. It sets the
// price and focuses it — it never submits (click-to-trade would bypass the
// two-enter confirm), and it clears a stale preview since the form changed.
func (m Model) handleMouse(e tea.MouseMsg) (tea.Model, tea.Cmd) {
	if e.Action != tea.MouseActionPress || e.Button != tea.MouseButtonLeft {
		return m, nil
	}
	if e.X > bookWidth+1 { // outside the book column (border + content)
		return m, nil
	}
	px, ok := m.priceAtY(e.Y)
	if !ok {
		return m, nil
	}
	m.pxBuf = m.fmtPx(px)
	m.focus = FocusPx
	m.pendingConfirm = nil
	m.status = fmt.Sprintf("price %s (click)", m.fmtPx(px))
	return m, nil
}

// handleKey applies one key. Digits / backspace / tab / b / s / t / r / p edit
// the form (and always invalidate a stale confirm preview); the rest are
// commands. Mirrors rsx-tui/src/input.rs plus specs/2/55-terminal.md.
func (m Model) handleKey(k tea.KeyMsg) (tea.Model, tea.Cmd) {
	key := k.String()
	// The help overlay is modal: any key dismisses it (q still quits), so it
	// never traps the trader.
	if m.showHelp {
		if key == "q" || key == "ctrl+c" {
			return m, tea.Quit
		}
		m.showHelp = false
		return m, nil
	}
	switch key {
	case "q", "ctrl+c":
		return m, tea.Quit
	case "?":
		m.showHelp = true
		return m, nil
	case "esc":
		if m.pendingConfirm != nil {
			m.pendingConfirm = nil
			m.status = "order not sent"
			return m, nil
		}
		return m, tea.Quit
	case "enter":
		return m.handleEnter()
	case "c":
		return m.handleCancel()
	case "X":
		return m.handleCancelAll()
	case "x":
		return m.handleFlatten()
	case "R":
		return m.handleReverse()
	case "m":
		return m.handleMarket()
	case "up":
		m.orderSel = m.clampSel(m.orderSel - 1)
		return m, nil
	case "down":
		m.orderSel = m.clampSel(m.orderSel + 1)
		return m, nil
	case "f3":
		m.showTrace = !m.showTrace
		return m, nil
	case "f2":
		m.armed = !m.armed
		m.pendingConfirm = nil
		if m.armed {
			m.status = "ARMED — orders fire on one enter (no confirm)"
		} else {
			m.status = "confirm on — two-enter preview restored"
		}
		return m, nil
	}

	// Editing keys. Each also clears any pending confirm — its preview would
	// be stale once the form changes.
	edited := true
	switch {
	case len(key) == 1 && key[0] >= '0' && key[0] <= '9':
		m.appendDigit(rune(key[0]))
	case key == ".":
		m.appendDot()
	case key == "backspace":
		m.backspace()
	case key == "tab":
		if m.focus == FocusPx {
			m.focus = FocusQty
		} else {
			m.focus = FocusPx
		}
	case key == "b":
		m.side = wire.Buy
	case key == "s":
		m.side = wire.Sell
	case key == "t":
		m.tif = m.tif.Next()
	case key == "r":
		m.reduceOnly = !m.reduceOnly
	case key == "p":
		m.postOnly = !m.postOnly
	case key == "+" || key == "=":
		m.stepPx(1)
	case key == "-":
		m.stepPx(-1)
	case key == "j":
		m.joinBid()
	case key == "k":
		m.joinAsk()
	default:
		edited = false
	}
	if edited {
		m.pendingConfirm = nil
	}
	return m, nil
}

// tick is the configured price increment, floored at 1 so the nudge keys always
// move by something even when unset.
func (m Model) tick() int64 {
	if m.cfg.Tick > 0 {
		return m.cfg.Tick
	}
	return 1
}

// currentPx is the raw price the buffer holds (decimal input reconstructed to
// raw), or a seed (mid rounded down to the tick, else one tick) when it's
// empty/unparseable — so `+`/`-` work before a price is typed.
func (m Model) currentPx() int64 {
	if px, ok := parseRaw(m.pxBuf, m.cfg.PriceDec); ok && px > 0 {
		return px
	}
	if mid, ok := m.book.Mid(); ok && mid > 0 {
		t := m.tick()
		return (mid / t) * t
	}
	return m.tick()
}

// stepPx nudges the price buffer by n ticks (n may be negative), flooring at one
// tick so it never reaches zero or negative, and focuses the price field. The
// buffer is written back as the human decimal the trader reads.
func (m *Model) stepPx(n int64) {
	px := m.currentPx() + n*m.tick()
	if px < m.tick() {
		px = m.tick()
	}
	m.pxBuf = m.fmtPx(px)
	m.focus = FocusPx
}

// joinBid / joinAsk set the price buffer to the current best bid / ask (as the
// human decimal) — the keyboard analog of clicking that level to rest at the
// touch.
func (m *Model) joinBid() {
	b, ok := m.book.BestBid()
	if !ok {
		m.status = "no bid to join"
		return
	}
	m.pxBuf = m.fmtPx(b.Px)
	m.focus = FocusPx
}

func (m *Model) joinAsk() {
	a, ok := m.book.BestAsk()
	if !ok {
		m.status = "no ask to join"
		return
	}
	m.pxBuf = m.fmtPx(a.Px)
	m.focus = FocusPx
}

// appendDot adds a single decimal point to the focused buffer — ignored if one
// is already there, the field has no fractional precision, or the buffer is
// full. A leading dot is expanded to "0." so ".5" reads as "0.5".
func (m *Model) appendDot() {
	buf, dec := &m.pxBuf, m.cfg.PriceDec
	if m.focus == FocusQty {
		buf, dec = &m.qtyBuf, m.cfg.QtyDec
	}
	if dec <= 0 || strings.ContainsRune(*buf, '.') || len(*buf) >= digitCap {
		return
	}
	if *buf == "" {
		*buf = "0"
	}
	*buf += "."
}

// appendDigit adds r to the focused buffer, capping length so it always parses.
func (m *Model) appendDigit(r rune) {
	if m.focus == FocusPx {
		if len(m.pxBuf) < digitCap {
			m.pxBuf += string(r)
		}
		return
	}
	if len(m.qtyBuf) < digitCap {
		m.qtyBuf += string(r)
	}
}

// backspace pops the last digit off the focused buffer.
func (m *Model) backspace() {
	if m.focus == FocusPx {
		if len(m.pxBuf) > 0 {
			m.pxBuf = m.pxBuf[:len(m.pxBuf)-1]
		}
		return
	}
	if len(m.qtyBuf) > 0 {
		m.qtyBuf = m.qtyBuf[:len(m.qtyBuf)-1]
	}
}

// handleEnter is the two-step confirm gate: the first enter builds a preview
// and returns without submitting; only a second, separate enter — with
// pendingConfirm already set from the first call — reaches Submit. Since
// Bubble Tea delivers one KeyMsg per Update call, a rapid double-enter is
// still two distinct calls through this gate: the preview always renders
// (viewConfirm, driven by pendingConfirm) before the send branch can run.
func (m Model) handleEnter() (tea.Model, tea.Cmd) {
	if m.pendingConfirm == nil {
		o, ok := m.buildOrder()
		if !ok {
			m.status = "incomplete order (need price & qty)"
			return m, nil
		}
		return m.arm(o)
	}

	o := *m.pendingConfirm
	if err := m.cfg.Sub.Submit(o); err != nil {
		// Keep the preview set so a retry-enter resubmits.
		m.status = "submit failed: " + err.Error()
		return m, nil
	}
	m.status = m.sentStatus(o)
	m.clearForm()
	return m, nil
}

// clearForm resets the order-entry fields after a send, keeping side and tif
// for the next order (a trader usually works one side at a time).
func (m *Model) clearForm() {
	m.pxBuf = ""
	m.qtyBuf = ""
	m.focus = FocusPx
	m.reduceOnly = false
	m.postOnly = false
	m.pendingConfirm = nil
}

// buildOrder parses the form (human-decimal px/qty) into a raw-i64 OrderReq, or
// false if either buffer is empty / unparseable / non-positive.
func (m Model) buildOrder() (wire.OrderReq, bool) {
	px, okPx := parseRaw(m.pxBuf, m.cfg.PriceDec)
	qty, okQty := parseRaw(m.qtyBuf, m.cfg.QtyDec)
	if !okPx || !okQty || px <= 0 || qty <= 0 {
		return wire.OrderReq{}, false
	}
	return wire.OrderReq{
		Side:       m.side,
		Px:         px,
		Qty:        qty,
		Tif:        m.tif,
		ReduceOnly: m.reduceOnly,
		PostOnly:   m.postOnly,
	}, true
}

// sentStatus renders the confirmation line for a submitted order in human
// decimals, e.g. "sent BUY 5.0000 @ 0.010001 [GTC] ro po".
func (m Model) sentStatus(o wire.OrderReq) string {
	s := fmt.Sprintf("sent %s %s @ %s [%s]", o.Side.Label(), m.fmtQty(o.Qty), m.fmtPx(o.Px), o.Tif.Label())
	if o.ReduceOnly {
		s += " ro"
	}
	if o.PostOnly {
		s += " po"
	}
	return s
}

// handleFlatten begins closing the whole derived net position: 'x' builds
// the reduce-only, marketable order that flattens it (Sell at the best bid
// to close a long, Buy at the best ask to close a short, qty = |net|) and
// routes it through the same pendingConfirm gate handleEnter's second
// press already drives — a flatten is never a single fat-fingered
// keystroke. A flat position, or a missing opposing best price (no live
// book to price against), is a no-op with a status explaining why; it
// never fabricates a price.
func (m Model) handleFlatten() (tea.Model, tea.Cmd) {
	if m.position.Flat() {
		m.status = "no position to flatten"
		return m, nil
	}
	o, ok := m.buildFlattenOrder()
	if !ok {
		m.status = "can't flatten: no live book to price against"
		return m, nil
	}
	return m.arm(o)
}

// handleMarket arms a marketable IOC at the far touch for the form's side +
// qty (Buy crosses the best ask, Sell the best bid) — take liquidity now,
// routed through the same confirm gate. Empty/bad qty or no opposing book to
// cross is a no-op with a reason; it never fabricates a price.
func (m Model) handleMarket() (tea.Model, tea.Cmd) {
	qty, ok := parseRaw(m.qtyBuf, m.cfg.QtyDec)
	if !ok || qty <= 0 {
		m.status = "market: enter a qty first"
		return m, nil
	}
	var px int64
	if m.side == wire.Buy {
		if len(m.book.Asks) == 0 {
			m.status = "market: no ask to cross"
			return m, nil
		}
		px = m.book.Asks[0].Px
	} else {
		if len(m.book.Bids) == 0 {
			m.status = "market: no bid to cross"
			return m, nil
		}
		px = m.book.Bids[0].Px
	}
	o := wire.OrderReq{Side: m.side, Px: px, Qty: qty, Tif: wire.Ioc, ReduceOnly: m.reduceOnly}
	return m.arm(o)
}

// maxOrderQty is the fat-finger hard cap (lots). An order over it is BLOCKED
// outright — a hard stop, not a dismissible soft warning (the Citi fat-finger
// lesson: soft warnings train click-through).
const maxOrderQty = 1_000_000

// arm applies the fat-finger guard, then either sets the confirm preview or —
// in ARMED (confirm-off) mode — submits straight away. Every order path (enter
// / market / flatten / reverse) goes through it, so nothing oversized can even
// reach the confirm, and ARMED uniformly skips the preview for all of them
// while the size guard still holds.
func (m Model) arm(o wire.OrderReq) (tea.Model, tea.Cmd) {
	if o.Qty > maxOrderQty {
		m.status = fmt.Sprintf("BLOCKED: qty %s exceeds max %s (fat-finger guard)", m.fmtQty(o.Qty), m.fmtQty(maxOrderQty))
		return m, nil
	}
	if m.armed {
		if err := m.cfg.Sub.Submit(o); err != nil {
			m.status = "submit failed: " + err.Error()
			return m, nil
		}
		m.status = m.sentStatus(o)
		m.clearForm()
		return m, nil
	}
	m.pendingConfirm = &o
	return m, nil
}

// handleReverse flips the net position: a marketable order of twice the net
// size on the opposite side (+N → -N), through the confirm gate. Not
// reduce-only — it deliberately crosses zero. Flat, or no book to cross, is a
// no-op with a reason.
func (m Model) handleReverse() (tea.Model, tea.Cmd) {
	net := m.position.Net
	if net == 0 {
		m.status = "no position to reverse"
		return m, nil
	}
	side, px := wire.Sell, int64(0)
	if net < 0 { // short → buy to flip long
		side = wire.Buy
		if len(m.book.Asks) == 0 {
			m.status = "reverse: no ask to cross"
			return m, nil
		}
		px = m.book.Asks[0].Px
	} else { // long → sell to flip short
		if len(m.book.Bids) == 0 {
			m.status = "reverse: no bid to cross"
			return m, nil
		}
		px = m.book.Bids[0].Px
	}
	qty := net
	if qty < 0 {
		qty = -qty
	}
	return m.arm(wire.OrderReq{Side: side, Px: px, Qty: 2 * qty, Tif: wire.Ioc})
}

// buildFlattenOrder builds the reduce-only IOC order that flattens the
// current derived position at the best opposing price: Sell at the best
// bid to close a long, Buy at the best ask to close a short. Reduce-only
// means the exchange never lets this order carry the position past flat
// (it can only shrink toward zero), so it can never flip the position.
// Returns false when that side of the book is empty.
func (m Model) buildFlattenOrder() (wire.OrderReq, bool) {
	net := m.position.Net
	if net == 0 {
		return wire.OrderReq{}, false
	}
	qty := net
	if qty < 0 {
		qty = -qty
	}
	if net > 0 {
		bid, ok := m.book.BestBid()
		if !ok {
			return wire.OrderReq{}, false
		}
		return wire.OrderReq{Side: wire.Sell, Px: bid.Px, Qty: qty, Tif: wire.Ioc, ReduceOnly: true}, true
	}
	ask, ok := m.book.BestAsk()
	if !ok {
		return wire.OrderReq{}, false
	}
	return wire.OrderReq{Side: wire.Buy, Px: ask.Px, Qty: qty, Tif: wire.Ioc, ReduceOnly: true}, true
}

// clampSel keeps an open-orders selection index in [0, len-1], or 0 when the
// list is empty, so the cursor is always valid as orders come and go.
func (m Model) clampSel(i int) int {
	n := len(m.openOrders)
	if n == 0 {
		return 0
	}
	return clamp(i, 0, n-1)
}

// handleCancel cancels the *selected* working order (up/down move the cursor,
// the panel + ladder show which one) — not a blind cancel-newest, so a trader
// can target a specific resting order.
func (m Model) handleCancel() (tea.Model, tea.Cmd) {
	if len(m.openOrders) == 0 {
		m.status = "no open order to cancel"
		return m, nil
	}
	o := m.openOrders[m.clampSel(m.orderSel)]
	if o.Cid == "" {
		m.status = "selected order has no cid yet"
		return m, nil
	}
	if err := m.cfg.Sub.Cancel(o.Cid); err != nil {
		m.status = "cancel failed: " + err.Error()
	} else {
		m.status = "cancel sent for order " + fmt.Sprint(o.Oid)
	}
	return m, nil
}

// handleCancelAll cancels every working order carrying a cid.
func (m Model) handleCancelAll() (tea.Model, tea.Cmd) {
	n := 0
	for _, o := range m.openOrders {
		if o.Cid == "" {
			continue
		}
		if err := m.cfg.Sub.Cancel(o.Cid); err != nil {
			m.status = "cancel-all failed: " + err.Error()
			return m, nil
		}
		n++
	}
	if n == 0 {
		m.status = "no open orders to cancel"
	} else {
		m.status = fmt.Sprintf("cancel-all sent (%d orders)", n)
	}
	return m, nil
}
