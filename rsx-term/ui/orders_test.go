package ui

import (
	"strings"
	"testing"

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
