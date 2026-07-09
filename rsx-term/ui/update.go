package ui

import (
	"fmt"
	"strconv"
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
		m.tape.Push(book.TapeEntry{Side: side, Px: v.Px, Qty: v.Qty})
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
		m.status = fmt.Sprintf("fill %d: %d @ %d", v.Oid, v.Qty, v.Px)

	case feed.Latency:
		s := v.Sample
		m.lastLat = &s
		m.latWindow.Add(v.Sample.TotalNs)

	case tea.WindowSizeMsg:
		m.width = v.Width
		m.height = v.Height
	}
	return m, nil
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
	}

	// Editing keys. Each also clears any pending confirm — its preview would
	// be stale once the form changes.
	edited := true
	switch {
	case len(key) == 1 && key[0] >= '0' && key[0] <= '9':
		m.appendDigit(rune(key[0]))
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
	default:
		edited = false
	}
	if edited {
		m.pendingConfirm = nil
	}
	return m, nil
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
	m.status = sentStatus(o)
	m.pxBuf = ""
	m.qtyBuf = ""
	m.focus = FocusPx
	m.reduceOnly = false
	m.postOnly = false
	m.pendingConfirm = nil
	// side and tif are intentionally kept for the next order.
	return m, nil
}

// buildOrder parses the form into an OrderReq, or false if either buffer is
// empty / unparseable / non-positive.
func (m Model) buildOrder() (wire.OrderReq, bool) {
	if m.pxBuf == "" || m.qtyBuf == "" {
		return wire.OrderReq{}, false
	}
	px, errPx := strconv.ParseInt(m.pxBuf, 10, 64)
	qty, errQty := strconv.ParseInt(m.qtyBuf, 10, 64)
	if errPx != nil || errQty != nil || px <= 0 || qty <= 0 {
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

// sentStatus renders the confirmation line for a submitted order,
// e.g. "sent BUY 5 @ 10001 [GTC] ro po".
func sentStatus(o wire.OrderReq) string {
	s := fmt.Sprintf("sent %s %d @ %d [%s]", o.Side.Label(), o.Qty, o.Px, o.Tif.Label())
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
	qty, err := strconv.ParseInt(m.qtyBuf, 10, 64)
	if m.qtyBuf == "" || err != nil || qty <= 0 {
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

// arm applies the fat-finger guard, then sets the confirm preview. Every order
// path (enter / market / flatten / reverse) goes through it, so nothing
// oversized can even reach the confirm.
func (m Model) arm(o wire.OrderReq) (tea.Model, tea.Cmd) {
	if o.Qty > maxOrderQty {
		m.status = fmt.Sprintf("BLOCKED: qty %d exceeds max %d (fat-finger guard)", o.Qty, maxOrderQty)
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
