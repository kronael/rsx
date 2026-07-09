package ui

import (
	"strings"
	"testing"

	"rsx-term/book"
	"rsx-term/conn"
	"rsx-term/wire"
)

func TestOwnOrderLevels(t *testing.T) {
	m := Model{openOrders: []OpenOrder{{Side: wire.Buy, Px: 100}, {Side: wire.Sell, Px: 110}}}
	bids, asks := m.ownOrderLevels()
	if !bids[100] || !asks[110] || len(bids) != 1 || len(asks) != 1 {
		t.Fatalf("ownOrderLevels: bids=%v asks=%v", bids, asks)
	}
}

func TestLevelMarker(t *testing.T) {
	own := map[int64]bool{100: true}
	if !strings.Contains(levelMarker(100, own, 105, true), "▸") {
		t.Fatal("own-order marker missing (should win over last-trade)")
	}
	if !strings.Contains(levelMarker(105, map[int64]bool{}, 105, true), "‹") {
		t.Fatal("last-trade marker missing")
	}
	if levelMarker(1, map[int64]bool{}, 2, true) != " " {
		t.Fatal("unmarked level should be a single space")
	}
}

func TestViewOpenOrders(t *testing.T) {
	m := Model{openOrders: []OpenOrder{{Side: wire.Buy, Px: 9999, Qty: 15}}}
	out := m.viewOpenOrders()
	if !strings.Contains(out, "BUY") || !strings.Contains(out, "9999") || !strings.Contains(out, "15") {
		t.Fatalf("open-orders panel missing content:\n%s", out)
	}
}

func TestClampSel(t *testing.T) {
	m := Model{}
	if m.clampSel(3) != 0 {
		t.Fatal("empty list should clamp to 0")
	}
	m.openOrders = []OpenOrder{{}, {}, {}}
	if m.clampSel(-1) != 0 || m.clampSel(5) != 2 || m.clampSel(1) != 1 {
		t.Fatal("clampSel out of range")
	}
}

func TestCancelSelected(t *testing.T) {
	mock := &conn.MockGateway{}
	m := newModel(mock)
	m.openOrders = []OpenOrder{{Cid: "aa", Oid: 1}, {Cid: "bb", Oid: 2}}
	m.orderSel = 0
	m.handleCancel()
	if len(mock.Cancelled) != 1 || mock.Cancelled[0] != "aa" {
		t.Fatalf("cancel should target selected (oldest): %v", mock.Cancelled)
	}
	m.orderSel = 1
	m.handleCancel()
	if mock.Cancelled[len(mock.Cancelled)-1] != "bb" {
		t.Fatalf("cancel should target selected (newest): %v", mock.Cancelled)
	}
}

func TestCancelAll(t *testing.T) {
	mock := &conn.MockGateway{}
	m := newModel(mock)
	m.openOrders = []OpenOrder{{Cid: "aa"}, {Cid: "bb"}}
	m.handleCancelAll()
	if len(mock.Cancelled) != 2 {
		t.Fatalf("cancel-all should cancel every order: %v", mock.Cancelled)
	}
}

func TestHandleMarket(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m.qtyBuf = "5"
	m.side = wire.Buy
	m.book.Asks = []wire.Level{{Px: 10002, Qty: 20}}
	got, _ := m.handleMarket()
	mm := got.(Model)
	if mm.pendingConfirm == nil {
		t.Fatal("market should arm a confirm")
	}
	o := *mm.pendingConfirm
	if o.Side != wire.Buy || o.Px != 10002 || o.Qty != 5 || o.Tif != wire.Ioc {
		t.Fatalf("market order wrong: %+v", o)
	}
	got2, _ := newModel(&conn.MockGateway{}).handleMarket()
	if got2.(Model).pendingConfirm != nil {
		t.Fatal("no qty should be a no-op, not a confirm")
	}
}

func TestHelpOverlayToggle(t *testing.T) {
	m := press(newModel(&conn.MockGateway{}), "?")
	if !m.showHelp {
		t.Fatal("? should open help")
	}
	m = press(m, "b")
	if m.showHelp {
		t.Fatal("any key should close help")
	}
}

func TestHopBar(t *testing.T) {
	if hopBar(book.Sample{NetNs: book.NsUnknown, InternalNs: book.NsUnknown, EngineNs: book.NsUnknown}, 18) != "" {
		t.Fatal("all-pending legs should render no bar")
	}
	bar := hopBar(book.Sample{NetNs: 2500, InternalNs: 7600, EngineNs: 340}, 18)
	if !strings.Contains(bar, "█") {
		t.Fatalf("known legs should render a bar: %q", bar)
	}
}

func TestFatFingerGuard(t *testing.T) {
	got, _ := (Model{}).arm(wire.OrderReq{Side: wire.Buy, Px: 1, Qty: maxOrderQty + 1})
	if got.(Model).pendingConfirm != nil {
		t.Fatal("oversized order must be hard-blocked, not armed")
	}
	got2, _ := (Model{}).arm(wire.OrderReq{Side: wire.Buy, Px: 1, Qty: 10})
	if got2.(Model).pendingConfirm == nil {
		t.Fatal("normal order should arm")
	}
}

func TestReverse(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m.book.Bids = []wire.Level{{Px: 9999, Qty: 50}}
	m.position.Net = 20 // long → sell 40 to flip short
	got, _ := m.handleReverse()
	o := got.(Model).pendingConfirm
	if o == nil || o.Side != wire.Sell || o.Qty != 40 || o.ReduceOnly {
		t.Fatalf("reverse of +20 should be a non-reduce-only Sell 40: %+v", o)
	}
}

func TestFmtDec(t *testing.T) {
	if got := fmtDec(10001, 6); got != "0.010001" {
		t.Fatalf("PENGU price raw 10001 @ 6dp = %q, want 0.010001", got)
	}
	if got := fmtDec(100000, 4); got != "10.0000" {
		t.Fatalf("qty raw 100000 @ 4dp = %q, want 10.0000", got)
	}
	if got := fmtDec(-250, 6); got != "-0.000250" {
		t.Fatalf("negative: %q", got)
	}
	if got := fmtDec(10001, 0); got != "10001" {
		t.Fatalf("0 decimals should be raw: %q", got)
	}
}

func TestStepPxNudgesByTick(t *testing.T) {
	m := New(Config{Sub: &conn.MockGateway{}, Tick: 5})
	m.pxBuf = "100"
	m = press(m, "+")
	if m.pxBuf != "105" {
		t.Fatalf("+ one tick: pxBuf=%q want 105", m.pxBuf)
	}
	m = press(m, "-")
	m = press(m, "-")
	if m.pxBuf != "95" {
		t.Fatalf("- two ticks: pxBuf=%q want 95", m.pxBuf)
	}
}

func TestStepPxFloorsAtOneTick(t *testing.T) {
	m := New(Config{Sub: &conn.MockGateway{}, Tick: 10})
	m.pxBuf = "10"
	m = press(m, "-") // would go to 0 → floored at one tick
	if m.pxBuf != "10" {
		t.Fatalf("- must floor at one tick: pxBuf=%q want 10", m.pxBuf)
	}
}

func TestStepPxSeedsFromMidWhenEmpty(t *testing.T) {
	m := New(Config{Sub: &conn.MockGateway{}, Tick: 1})
	m.book.Bids = []wire.Level{{Px: 100, Qty: 1}}
	m.book.Asks = []wire.Level{{Px: 104, Qty: 1}}
	m = press(m, "+") // mid=102 (rounded to tick) then +1 tick
	if m.pxBuf != "103" {
		t.Fatalf("+ from empty seeds from mid: pxBuf=%q want 103", m.pxBuf)
	}
}

func TestJoinBidAsk(t *testing.T) {
	m := New(Config{Sub: &conn.MockGateway{}, Tick: 1})
	m.book.Bids = []wire.Level{{Px: 999, Qty: 1}}
	m.book.Asks = []wire.Level{{Px: 1002, Qty: 1}}
	if got := press(m, "j"); got.pxBuf != "999" {
		t.Fatalf("j join bid: pxBuf=%q want 999", got.pxBuf)
	}
	if got := press(m, "k"); got.pxBuf != "1002" {
		t.Fatalf("k join ask: pxBuf=%q want 1002", got.pxBuf)
	}
}

func TestArmedTogglesAndBanner(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	if m.viewArmedBanner() != "" {
		t.Fatal("banner must be empty when confirm is on")
	}
	m = press(m, "f2")
	if !m.armed {
		t.Fatal("f2 should arm")
	}
	if !strings.Contains(stripANSI(m.viewArmedBanner()), "ARMED") {
		t.Fatalf("armed banner missing:\n%s", m.viewArmedBanner())
	}
	m = press(m, "f2")
	if m.armed {
		t.Fatal("f2 should re-arm safety (toggle off)")
	}
}

func TestArmedSubmitsOnSingleEnter(t *testing.T) {
	mock := &conn.MockGateway{}
	m := New(Config{Sub: mock})
	m = press(m, "f2") // ARM
	m = typeDigits(m, "100")
	m = press(m, "tab")
	m = typeDigits(m, "5")
	m = press(m, "enter") // single enter fires in ARMED mode
	if len(mock.Submitted) != 1 {
		t.Fatalf("ARMED single-enter should submit once, got %d", len(mock.Submitted))
	}
	if m.pendingConfirm != nil {
		t.Fatal("ARMED mode must not set a pending confirm")
	}
}

func TestArmedStillHonorsFatFingerGuard(t *testing.T) {
	mock := &conn.MockGateway{}
	m := New(Config{Sub: mock})
	m = press(m, "f2") // ARM
	m = typeDigits(m, "1")
	m = press(m, "tab")
	m = typeDigits(m, "9999999") // over maxOrderQty
	m = press(m, "enter")
	if len(mock.Submitted) != 0 {
		t.Fatalf("fat-finger guard must block even in ARMED mode, submitted %d", len(mock.Submitted))
	}
	if !strings.Contains(m.status, "BLOCKED") {
		t.Fatalf("expected BLOCKED status, got %q", m.status)
	}
}
