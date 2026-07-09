package ui

// Flow tests drive the model through a WHOLE user journey — not one fold in
// isolation but the sequence a trader actually performs: connect, watch the
// book, place an order, see it accepted, get filled, manage the position. Each
// test mirrors one section of FLOWS.md (same number/name), so the walkthrough
// doc and the executable checks stay in lockstep. They compose the same helpers
// the unit tests use (press / typeDigits / apply / clickAt / stripANSI).

import (
	"strings"
	"testing"

	"rsx-term/conn"
	"rsx-term/feed"
	"rsx-term/wire"
)

// pengu builds a terminal at real PENGU precision (price 6dp, qty 4dp, tick 1)
// over a mock gateway — the config the flows are written against.
func pengu(mock *conn.MockGateway) Model {
	return New(Config{Symbol: "PENGU-PERP", SymbolID: 10, Sub: mock, PriceDec: 6, QtyDec: 4, Tick: 1})
}

// live connects both links and seeds a two-sided book, the precondition most
// flows start from (a connected terminal showing a live ladder).
func live(mock *conn.MockGateway) Model {
	m := pengu(mock)
	m = apply(m, feed.GwUp{})
	m = apply(m, feed.MdUp{})
	m = apply(m, wire.Snapshot{
		Bids: []wire.Level{{Px: 10000, Qty: 70000}, {Px: 9999, Qty: 150000}},
		Asks: []wire.Level{{Px: 10002, Qty: 60000}, {Px: 10003, Qty: 250000}},
	})
	m.width, m.height = 120, 26
	return m
}

// Flow 1 — Startup & connect: links come up, a snapshot lands, the ladder
// renders and the status bar reads "connected".
func TestFlowStartupConnect(t *testing.T) {
	m := pengu(&conn.MockGateway{})
	if m.gwConnected {
		t.Fatal("starts disconnected")
	}
	if !strings.Contains(stripANSI(m.viewBook()), "no live book") {
		t.Fatal("before marketdata the book is degraded, not blank")
	}
	m = apply(m, feed.GwUp{})
	if !m.gwConnected || m.status != "connected" {
		t.Fatalf("GwUp should connect: gw=%v status=%q", m.gwConnected, m.status)
	}
	m = apply(m, feed.MdUp{})
	m = apply(m, wire.Snapshot{Bids: []wire.Level{{Px: 10000, Qty: 70000}}, Asks: []wire.Level{{Px: 10002, Qty: 60000}}})
	book := stripANSI(m.viewBook())
	if strings.Contains(book, "no live book") {
		t.Fatalf("after snapshot the ladder should render:\n%s", book)
	}
	if !strings.Contains(book, "0.010002") { // ask shown as a human decimal
		t.Fatalf("ladder should show decimal prices:\n%s", book)
	}
}

// Flow 2 — Place a limit order: type the decimals you read, first enter previews
// (no send), second enter submits the raw-i64 order, the ack lands it in the
// working-orders panel.
func TestFlowPlaceLimitOrder(t *testing.T) {
	mock := &conn.MockGateway{}
	m := live(mock)

	m = typeDigits(m, "0.010001")
	m = press(m, "tab")
	m = typeDigits(m, "5")

	m = press(m, "enter") // preview only
	if m.pendingConfirm == nil {
		t.Fatal("first enter must preview")
	}
	if len(mock.Submitted) != 0 {
		t.Fatal("first enter must NOT submit")
	}
	if !strings.Contains(stripANSI(m.viewOrder()), "confirm") {
		t.Fatal("preview should render in the order panel")
	}

	m = press(m, "enter") // send
	if len(mock.Submitted) != 1 {
		t.Fatalf("second enter should submit, got %d", len(mock.Submitted))
	}
	sent := mock.Submitted[0]
	if sent.Px != 10001 || sent.Qty != 50000 || sent.Tif != wire.Gtc {
		t.Fatalf("decimal->raw wrong: %+v", sent)
	}
	if !strings.Contains(m.status, "sent BUY 5.0000 @ 0.010001") {
		t.Fatalf("status not in decimals: %q", m.status)
	}

	// The gateway acks with an oid; the order joins the working-orders panel.
	m = apply(m, wire.Accepted{Oid: 1, Cid: "c1", Order: sent})
	if len(m.openOrders) != 1 {
		t.Fatalf("accept should add a working order, have %d", len(m.openOrders))
	}
	if !strings.Contains(stripANSI(m.viewOpenOrders()), "0.010001") {
		t.Fatal("working-orders panel should show the resting price")
	}
}

// Flow 3 — Cancel a specific working order: ↑/↓ move the cursor, c cancels the
// selected order (not a blind cancel-newest).
func TestFlowCancelSelected(t *testing.T) {
	mock := &conn.MockGateway{}
	m := live(mock)
	m = apply(m, wire.Accepted{Oid: 1, Cid: "c1", Order: wire.OrderReq{Side: wire.Buy, Px: 9999, Qty: 50000}})
	m = apply(m, wire.Accepted{Oid: 2, Cid: "c2", Order: wire.OrderReq{Side: wire.Buy, Px: 9998, Qty: 30000}})

	m = press(m, "down") // select the second order
	m = press(m, "c")
	if len(mock.Cancelled) != 1 || mock.Cancelled[0] != "c2" {
		t.Fatalf("c should cancel the SELECTED order (c2): %v", mock.Cancelled)
	}
	// The exchange confirms with Done; the order leaves the panel.
	m = apply(m, wire.Done{Oid: 2})
	if len(m.openOrders) != 1 || m.openOrders[0].Oid != 1 {
		t.Fatalf("Done should remove only oid 2: %+v", m.openOrders)
	}
}

// Flow 4 — Cancel all: X cancels every working order in one keystroke.
func TestFlowCancelAll(t *testing.T) {
	mock := &conn.MockGateway{}
	m := live(mock)
	m = apply(m, wire.Accepted{Oid: 1, Cid: "c1", Order: wire.OrderReq{Side: wire.Buy, Px: 9999, Qty: 1}})
	m = apply(m, wire.Accepted{Oid: 2, Cid: "c2", Order: wire.OrderReq{Side: wire.Sell, Px: 10003, Qty: 1}})
	m = press(m, "X")
	if len(mock.Cancelled) != 2 {
		t.Fatalf("X should cancel all, got %v", mock.Cancelled)
	}
}

// Flow 5 — Market order: enter a qty, m fires an IOC at the far touch through
// the same confirm gate.
func TestFlowMarketOrder(t *testing.T) {
	mock := &conn.MockGateway{}
	m := live(mock)
	// Market needs only a qty (it prices at the far touch). Focus starts on
	// price, so tab to the qty field.
	m = press(m, "tab")
	m = typeDigits(m, "3")
	m = press(m, "m") // market
	if m.pendingConfirm == nil {
		t.Fatal("m should arm a confirm")
	}
	o := *m.pendingConfirm
	if o.Tif != wire.Ioc || o.Px != 10002 { // buy crosses the best ask (10002)
		t.Fatalf("market should be IOC at the far touch: %+v", o)
	}
	m = press(m, "enter")
	if len(mock.Submitted) != 1 || mock.Submitted[0].Tif != wire.Ioc {
		t.Fatalf("market submit wrong: %v", mock.Submitted)
	}
}

// Flow 6 — A fill builds a position: an F event folds into net / entry / uPnL.
func TestFlowFillBuildsPosition(t *testing.T) {
	m := live(&conn.MockGateway{})
	if !m.position.Flat() {
		t.Fatal("starts flat")
	}
	m = apply(m, wire.Fill{Oid: 1, Px: 9999, Qty: 150000, Side: wire.Buy})
	if m.position.Net != 150000 {
		t.Fatalf("fill should build net: %d", m.position.Net)
	}
	pos := stripANSI(m.viewPositions())
	if !strings.Contains(pos, "LONG") || !strings.Contains(pos, "+15.0000") {
		t.Fatalf("positions should show LONG +15.0000:\n%s", pos)
	}
	if m.fills != 1 {
		t.Fatalf("fill counter = %d", m.fills)
	}
}

// Flow 7 — Flatten: x builds the reduce-only close of the whole position at the
// opposing touch, through the confirm gate; Done clears it.
func TestFlowFlatten(t *testing.T) {
	mock := &conn.MockGateway{}
	m := live(mock)
	m = apply(m, wire.Fill{Oid: 1, Px: 9999, Qty: 150000, Side: wire.Buy}) // long 15

	m = press(m, "x")
	if m.pendingConfirm == nil {
		t.Fatal("x should arm a flatten confirm")
	}
	o := *m.pendingConfirm
	if o.Side != wire.Sell || !o.ReduceOnly || o.Qty != 150000 || o.Px != 10000 {
		t.Fatalf("flatten of a long should be reduce-only Sell 15 at best bid: %+v", o)
	}
	m = press(m, "enter")
	if len(mock.Submitted) != 1 {
		t.Fatalf("flatten should submit: %v", mock.Submitted)
	}
}

// Flow 8 — Reverse: R flips the position (2× net, opposite side, crosses zero,
// NOT reduce-only).
func TestFlowReverse(t *testing.T) {
	mock := &conn.MockGateway{}
	m := live(mock)
	m = apply(m, wire.Fill{Oid: 1, Px: 9999, Qty: 150000, Side: wire.Buy}) // long 15
	m = press(m, "R")
	o := m.pendingConfirm
	if o == nil || o.Side != wire.Sell || o.Qty != 300000 || o.ReduceOnly {
		t.Fatalf("reverse of +15 should be a non-reduce-only Sell 30: %+v", o)
	}
}

// Flow 9 — ARMED (confirm-off): F2 removes the two-enter step; a single enter
// fires. The fat-finger guard still blocks oversized orders.
func TestFlowArmedConfirmOff(t *testing.T) {
	mock := &conn.MockGateway{}
	m := live(mock)
	m = press(m, "f2") // ARM
	if !strings.Contains(stripANSI(m.viewArmedBanner()), "ARMED") {
		t.Fatal("armed banner should warn")
	}
	m = typeDigits(m, "0.010001")
	m = press(m, "tab")
	m = typeDigits(m, "5")
	m = press(m, "enter") // single enter fires
	if len(mock.Submitted) != 1 || m.pendingConfirm != nil {
		t.Fatalf("ARMED single-enter should submit without a preview: submitted=%d", len(mock.Submitted))
	}
	// Even ARMED, an oversized order is hard-blocked — ARMED removes the
	// confirm, never the fat-finger guard.
	blocked := live(mock)
	blocked = press(blocked, "f2")
	blocked = typeDigits(blocked, "0.010001")
	blocked = press(blocked, "tab")
	blocked = typeDigits(blocked, "999999") // way over the cap
	before := len(mock.Submitted)
	blocked = press(blocked, "enter")
	if len(mock.Submitted) != before || !strings.Contains(blocked.status, "BLOCKED") {
		t.Fatalf("fat-finger guard must hold in ARMED: status=%q", blocked.status)
	}
}

// Flow 10 — Mouse click-to-price: left-clicking a ladder row loads that row's
// price into the form (it never submits).
func TestFlowClickToPrice(t *testing.T) {
	mock := &conn.MockGateway{}
	m := live(mock)
	m.ladderCenter = 10001
	m = clickAt(m, 5, 13) // centre row of the ladder (see priceAtY)
	if m.pxBuf == "" {
		t.Fatal("a ladder click should load a price")
	}
	if m.focus != FocusPx {
		t.Fatal("a click should focus the price field")
	}
	if len(mock.Submitted) != 0 {
		t.Fatal("a click must never submit")
	}
}

// Flow 11 — Price helpers: +/- nudge a tick, j/k join the best bid/ask.
func TestFlowPriceHelpers(t *testing.T) {
	m := live(&conn.MockGateway{})
	m = press(m, "j") // join best bid (10000)
	if m.pxBuf != "0.010000" {
		t.Fatalf("j should join best bid as a decimal: %q", m.pxBuf)
	}
	m = press(m, "+") // up one tick -> 10001
	if m.pxBuf != "0.010001" {
		t.Fatalf("+ should nudge one tick: %q", m.pxBuf)
	}
	m = press(m, "k") // join best ask (10002)
	if m.pxBuf != "0.010002" {
		t.Fatalf("k should join best ask: %q", m.pxBuf)
	}
}

// Flow 12 — Fat-finger block: an oversized order is refused outright, never
// previewed, never sent.
func TestFlowFatFingerBlock(t *testing.T) {
	mock := &conn.MockGateway{}
	m := live(mock)
	m = typeDigits(m, "0.010001")
	m = press(m, "tab")
	m = typeDigits(m, "999999") // 999999 lots, way over the cap
	m = press(m, "enter")
	if m.pendingConfirm != nil {
		t.Fatal("oversized order must not even preview")
	}
	if len(mock.Submitted) != 0 || !strings.Contains(m.status, "BLOCKED") {
		t.Fatalf("oversized order must be blocked: status=%q", m.status)
	}
}

// Flow 13 — Reject: an exchange rejection surfaces in the status line.
func TestFlowReject(t *testing.T) {
	m := live(&conn.MockGateway{})
	m = apply(m, wire.Rejected{Reason: "insufficient margin"})
	if !strings.Contains(m.status, "rejected") || !strings.Contains(m.status, "insufficient margin") {
		t.Fatalf("reject should surface the reason: %q", m.status)
	}
}

// Flow 14 — Marketdata down: the ladder degrades to an honest amber row, not a
// blank or stale book; recovering restores it.
func TestFlowMarketdataDegraded(t *testing.T) {
	m := live(&conn.MockGateway{})
	if strings.Contains(stripANSI(m.viewBook()), "no live book") {
		t.Fatal("precondition: book is live")
	}
	m = apply(m, feed.MdDown{})
	if !strings.Contains(stripANSI(m.viewBook()), "no live book") {
		t.Fatal("MdDown should degrade the book to an honest amber row")
	}
	m = apply(m, feed.MdUp{})
	m = apply(m, wire.Bbo{BidPx: 10000, BidQty: 1, AskPx: 10002, AskQty: 1})
	if strings.Contains(stripANSI(m.viewBook()), "no live book") {
		t.Fatal("recovery should restore the ladder")
	}
}

// Flow 15 — Gateway down: the link dot flips to offline; marketdata is
// independent and stays up.
func TestFlowGatewayDegraded(t *testing.T) {
	m := live(&conn.MockGateway{})
	m = apply(m, feed.GwDown{})
	if !strings.Contains(stripANSI(m.viewStatusBar()), "offline") {
		t.Fatal("GwDown should show the offline dot")
	}
	if !m.mdConnected {
		t.Fatal("the marketdata link is independent and should stay up")
	}
}

// Flow 16 — Overlays: F3 opens the latency trace, ? the help; any key closes
// help.
func TestFlowOverlays(t *testing.T) {
	m := live(&conn.MockGateway{})
	m = press(m, "f3")
	if !m.showTrace {
		t.Fatal("F3 should open the trace")
	}
	if !strings.Contains(stripANSI(m.viewTrace()), "LATENCY") {
		t.Fatal("trace should show the latency sections")
	}
	m = press(m, "?")
	if !m.showHelp {
		t.Fatal("? should open help")
	}
	if !strings.Contains(stripANSI(m.viewHelpOverlay()), "KEYS") {
		t.Fatal("help overlay should list the keys")
	}
	m = press(m, "b") // any key closes help
	if m.showHelp {
		t.Fatal("any key should close help")
	}
}
