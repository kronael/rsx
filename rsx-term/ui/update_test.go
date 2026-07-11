package ui

import (
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/book"
	"rsx-term/conn"
	"rsx-term/wire"
)

// newModel builds a model over a mock for the tests.
func newModel(mock *conn.MockGateway) Model {
	return New(Config{
		Symbol:     "PENGU-PERP",
		SymbolID:   10,
		Endpoint:   "mock://demo",
		MdEndpoint: "mock://demo",
		Sub:        mock,
	})
}

// press feeds one key through Update and returns the updated model. It builds
// the right tea.KeyMsg for named keys; anything else is treated as runes.
func press(m Model, key string) Model {
	var msg tea.KeyMsg
	switch key {
	case "enter":
		msg = tea.KeyMsg{Type: tea.KeyEnter}
	case "tab":
		msg = tea.KeyMsg{Type: tea.KeyTab}
	case "esc":
		msg = tea.KeyMsg{Type: tea.KeyEsc}
	case "backspace":
		msg = tea.KeyMsg{Type: tea.KeyBackspace}
	case "f3":
		msg = tea.KeyMsg{Type: tea.KeyF3}
	case "f2":
		msg = tea.KeyMsg{Type: tea.KeyF2}
	case "f9":
		msg = tea.KeyMsg{Type: tea.KeyF9}
	case "ctrl+c":
		msg = tea.KeyMsg{Type: tea.KeyCtrlC}
	default:
		msg = tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune(key)}
	}
	updated, _ := m.Update(msg)
	return updated.(Model)
}

// typeDigits presses each rune of s in turn.
func typeDigits(m Model, s string) Model {
	for _, r := range s {
		m = press(m, string(r))
	}
	return m
}

// apply feeds a non-key message through Update.
func apply(m Model, msg tea.Msg) Model {
	updated, _ := m.Update(msg)
	return updated.(Model)
}

func TestTypingLandsInFocusedBuffer(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m = typeDigits(m, "12")
	if m.pxBuf != "12" {
		t.Fatalf("pxBuf = %q, want 12", m.pxBuf)
	}
	m = press(m, "tab")
	m = typeDigits(m, "5")
	if m.qtyBuf != "5" {
		t.Fatalf("qtyBuf = %q, want 5", m.qtyBuf)
	}
	if m.pxBuf != "12" {
		t.Fatalf("pxBuf changed after tab: %q", m.pxBuf)
	}
}

func TestDigitCap(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m = typeDigits(m, "1234567890123456789012") // 22 digits
	if len(m.pxBuf) != digitCap {
		t.Fatalf("pxBuf len = %d, want %d", len(m.pxBuf), digitCap)
	}
}

func TestFormToggles(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m = press(m, "s")
	if m.side != wire.Sell {
		t.Fatalf("side = %v, want Sell", m.side)
	}
	m = press(m, "b")
	if m.side != wire.Buy {
		t.Fatalf("side = %v, want Buy", m.side)
	}
	m = press(m, "t")
	if m.tif != wire.Ioc {
		t.Fatalf("tif = %v, want Ioc", m.tif)
	}
	m = press(m, "t")
	if m.tif != wire.Fok {
		t.Fatalf("tif = %v, want Fok", m.tif)
	}
	m = press(m, "r")
	if !m.reduceOnly {
		t.Fatalf("reduceOnly not set")
	}
	m = press(m, "p")
	if !m.postOnly {
		t.Fatalf("postOnly not set")
	}
}

func TestBackspace(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m = typeDigits(m, "123")
	m = press(m, "backspace")
	if m.pxBuf != "12" {
		t.Fatalf("pxBuf = %q, want 12", m.pxBuf)
	}
}

func TestEnterIncompleteDoesNotSubmit(t *testing.T) {
	mock := &conn.MockGateway{}
	m := newModel(mock)
	m = typeDigits(m, "1") // price only
	m = press(m, "enter")
	if !strings.Contains(m.status, "incomplete") {
		t.Fatalf("status = %q, want contains incomplete", m.status)
	}
	if len(mock.Submitted) != 0 {
		t.Fatalf("submitted %d on incomplete form", len(mock.Submitted))
	}
	if m.pendingConfirm != nil {
		t.Fatalf("pendingConfirm set on incomplete form")
	}
}

func TestSubmitGtcBuy(t *testing.T) {
	mock := &conn.MockGateway{}
	m := newModel(mock)
	m = typeDigits(m, "10001")
	m = press(m, "tab")
	m = typeDigits(m, "5")

	m = press(m, "enter") // preview
	if m.pendingConfirm == nil {
		t.Fatalf("no preview after first enter")
	}
	if len(mock.Submitted) != 0 {
		t.Fatalf("submitted on first enter")
	}

	m = press(m, "enter") // submit
	if len(mock.Submitted) != 1 {
		t.Fatalf("submitted %d, want 1", len(mock.Submitted))
	}
	want := wire.OrderReq{Side: wire.Buy, Px: 10001, Qty: 5, Tif: wire.Gtc}
	if mock.Submitted[0] != want {
		t.Fatalf("submitted %+v, want %+v", mock.Submitted[0], want)
	}
	if !strings.Contains(m.status, "sent BUY 5 @ 10001 [GTC]") {
		t.Fatalf("status = %q", m.status)
	}
	if m.pxBuf != "" || m.qtyBuf != "" {
		t.Fatalf("buffers not cleared: px=%q qty=%q", m.pxBuf, m.qtyBuf)
	}
	if m.focus != FocusPx {
		t.Fatalf("focus not reset to FocusPx")
	}
	if m.pendingConfirm != nil {
		t.Fatalf("pendingConfirm not cleared after submit")
	}
}

func TestSubmitIocSellReducePost(t *testing.T) {
	mock := &conn.MockGateway{}
	m := newModel(mock)
	m = press(m, "s") // sell
	m = press(m, "t") // ioc
	m = press(m, "r") // reduce-only
	m = press(m, "p") // post-only
	m = typeDigits(m, "9998")
	m = press(m, "tab")
	m = typeDigits(m, "3")

	m = press(m, "enter") // preview
	m = press(m, "enter") // submit

	if len(mock.Submitted) != 1 {
		t.Fatalf("submitted %d, want 1", len(mock.Submitted))
	}
	want := wire.OrderReq{Side: wire.Sell, Px: 9998, Qty: 3, Tif: wire.Ioc, ReduceOnly: true, PostOnly: true}
	if mock.Submitted[0] != want {
		t.Fatalf("submitted %+v, want %+v", mock.Submitted[0], want)
	}
	if !strings.Contains(m.status, "sent SELL 3 @ 9998 [IOC] ro po") {
		t.Fatalf("status = %q", m.status)
	}
	// side and tif kept; ro/po reset.
	if m.side != wire.Sell || m.tif != wire.Ioc {
		t.Fatalf("side/tif reset: side=%v tif=%v", m.side, m.tif)
	}
	if m.reduceOnly || m.postOnly {
		t.Fatalf("ro/po not reset")
	}
}

func TestSingleEnterSubmitsNothing(t *testing.T) {
	mock := &conn.MockGateway{}
	m := newModel(mock)
	m = typeDigits(m, "100")
	m = press(m, "tab")
	m = typeDigits(m, "2")
	m = press(m, "enter") // first press only
	if len(mock.Submitted) != 0 {
		t.Fatalf("submitted %d on single enter", len(mock.Submitted))
	}
	if m.pendingConfirm == nil {
		t.Fatalf("no preview after first enter")
	}
}

func TestEscClearsPendingConfirm(t *testing.T) {
	mock := &conn.MockGateway{}
	m := newModel(mock)
	m = typeDigits(m, "100")
	m = press(m, "tab")
	m = typeDigits(m, "2")
	m = press(m, "enter") // preview
	if m.pendingConfirm == nil {
		t.Fatalf("no preview to cancel")
	}
	m = press(m, "esc")
	if m.pendingConfirm != nil {
		t.Fatalf("esc did not clear pendingConfirm")
	}
	if m.status != "order not sent" {
		t.Fatalf("status = %q, want 'order not sent'", m.status)
	}
	if len(mock.Submitted) != 0 {
		t.Fatalf("esc submitted an order")
	}
}

func TestEditKeyClearsPendingConfirm(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m = typeDigits(m, "100")
	m = press(m, "tab")
	m = typeDigits(m, "2")
	m = press(m, "enter") // preview
	if m.pendingConfirm == nil {
		t.Fatalf("no preview")
	}
	m = press(m, "t") // editing key
	if m.pendingConfirm != nil {
		t.Fatalf("editing key did not clear pendingConfirm")
	}
}

func TestSubmitAgainstDownMock(t *testing.T) {
	mock := &conn.MockGateway{Down: true}
	m := newModel(mock)
	m = typeDigits(m, "100")
	m = press(m, "tab")
	m = typeDigits(m, "2")
	m = press(m, "enter") // preview
	m = press(m, "enter") // submit -> fails
	if !strings.Contains(m.status, "submit failed") {
		t.Fatalf("status = %q, want contains 'submit failed'", m.status)
	}
	if len(mock.Submitted) != 0 {
		t.Fatalf("down mock recorded a submit")
	}
	if m.pendingConfirm == nil {
		t.Fatalf("pendingConfirm cleared on failed submit (should retry)")
	}
}

func TestCancelNoOpenOrder(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m = press(m, "c")
	if m.status != "no open order to cancel" {
		t.Fatalf("status = %q", m.status)
	}
}

func TestAcceptedThenCancel(t *testing.T) {
	mock := &conn.MockGateway{}
	m := newModel(mock)
	m = apply(m, wire.Accepted{
		Oid:   7,
		Order: wire.OrderReq{Side: wire.Buy, Px: 9998, Qty: 14, Tif: wire.Gtc},
		Cid:   "00000000000000000001",
		RttNs: 10440,
	})
	m = press(m, "c")
	if len(mock.Cancelled) != 1 || mock.Cancelled[0] != "00000000000000000001" {
		t.Fatalf("cancelled = %v", mock.Cancelled)
	}
	if !strings.Contains(m.status, "cancel sent for order 7") {
		t.Fatalf("status = %q", m.status)
	}
}

func TestDoneRemovesOrderAndUnknownIsNoOp(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m = apply(m, wire.Accepted{Oid: 7, Order: wire.OrderReq{Side: wire.Buy, Px: 9998, Qty: 14}, Cid: "c1", RttNs: -1})
	if len(m.openOrders) != 1 {
		t.Fatalf("openOrders = %d, want 1", len(m.openOrders))
	}
	m = apply(m, wire.Done{Oid: 999, RttNs: -1}) // unknown oid
	if len(m.openOrders) != 1 {
		t.Fatalf("unknown Done changed openOrders: %d", len(m.openOrders))
	}
	m = apply(m, wire.Done{Oid: 7, RttNs: -1})
	if len(m.openOrders) != 0 {
		t.Fatalf("Done did not remove order: %d", len(m.openOrders))
	}
}

func TestFillFoldsPosition(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m = apply(m, wire.Fill{Oid: 7, Px: 9998, Qty: 14, Side: wire.Buy})
	if m.position.Net != 14 {
		t.Fatalf("net = %d, want 14", m.position.Net)
	}
	entry, ok := m.position.Entry()
	if !ok || entry != 9998 {
		t.Fatalf("entry = %d ok = %v, want 9998 true", entry, ok)
	}
	if m.fills != 1 {
		t.Fatalf("fills = %d, want 1", m.fills)
	}
}

func TestMarketDataFold(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m = apply(m, wire.Snapshot{
		SymbolID: 10,
		Bids:     []wire.Level{{Px: 10000, Qty: 7, Count: 1}, {Px: 9999, Qty: 15, Count: 1}},
		Asks:     []wire.Level{{Px: 10001, Qty: 5, Count: 1}},
		Seq:      1,
	})
	m = apply(m, wire.Delta{SymbolID: 10, Side: 0, Px: 9998, Qty: 9, Count: 1, Seq: 2})
	found := false
	for _, l := range m.book.Bids {
		if l.Px == 9998 && l.Qty == 9 {
			found = true
		}
	}
	if !found {
		t.Fatalf("delta level not folded into bids: %+v", m.book.Bids)
	}
	m = apply(m, wire.MdTrade{SymbolID: 10, Px: 10001, Qty: 5, TakerSide: 0, Seq: 3})
	last, ok := m.tape.Last()
	if !ok || last.Px != 10001 || last.Side != wire.Buy {
		t.Fatalf("tape last = %+v ok = %v", last, ok)
	}
}

func TestAcceptedRttFoldsLatency(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m = apply(m, wire.Accepted{Oid: 7, Order: wire.OrderReq{Side: wire.Buy, Px: 9998, Qty: 14}, Cid: "c1", RttNs: 12345})
	if m.lastLat == nil || m.lastLat.TotalNs != 12345 {
		t.Fatalf("lastLat = %+v", m.lastLat)
	}
	if m.lastLat.NetNs != book.NsUnknown {
		t.Fatalf("live leg should be NsUnknown, got %d", m.lastLat.NetNs)
	}
}

// withBook returns m with a two-level book so buildFlattenOrder always has
// a best bid/ask to price against.
func withBook(m Model) Model {
	return apply(m, wire.Snapshot{
		Bids: []wire.Level{{Px: 9_999, Qty: 5}, {Px: 9_998, Qty: 5}},
		Asks: []wire.Level{{Px: 10_001, Qty: 5}, {Px: 10_002, Qty: 5}},
		Seq:  1,
	})
}

func TestBuildFlattenOrderFlatIsNone(t *testing.T) {
	m := withBook(newModel(&conn.MockGateway{}))
	if _, ok := m.buildFlattenOrder(); ok {
		t.Fatalf("flat position built a flatten order")
	}
}

func TestBuildFlattenOrderLongSellsReduceOnly(t *testing.T) {
	m := withBook(newModel(&conn.MockGateway{}))
	m = apply(m, wire.Fill{Oid: 1, Px: 9_998, Qty: 7, Side: wire.Buy})

	o, ok := m.buildFlattenOrder()
	if !ok {
		t.Fatalf("no flatten order for a long position")
	}
	want := wire.OrderReq{Side: wire.Sell, Px: 9_999, Qty: 7, Tif: wire.Ioc, ReduceOnly: true}
	if o != want {
		t.Fatalf("flatten order = %+v, want %+v", o, want)
	}
}

func TestBuildFlattenOrderShortBuysReduceOnly(t *testing.T) {
	m := withBook(newModel(&conn.MockGateway{}))
	m = apply(m, wire.Fill{Oid: 1, Px: 10_002, Qty: 4, Side: wire.Sell})

	o, ok := m.buildFlattenOrder()
	if !ok {
		t.Fatalf("no flatten order for a short position")
	}
	want := wire.OrderReq{Side: wire.Buy, Px: 10_001, Qty: 4, Tif: wire.Ioc, ReduceOnly: true}
	if o != want {
		t.Fatalf("flatten order = %+v, want %+v", o, want)
	}
}

func TestBuildFlattenOrderNoOpposingBook(t *testing.T) {
	m := newModel(&conn.MockGateway{}) // no book at all
	m = apply(m, wire.Fill{Oid: 1, Px: 9_998, Qty: 7, Side: wire.Buy})
	if _, ok := m.buildFlattenOrder(); ok {
		t.Fatalf("built a flatten order with no book to price against")
	}
}

func TestFlattenKeyFlatIsNoOp(t *testing.T) {
	mock := &conn.MockGateway{}
	m := withBook(newModel(mock))
	m = press(m, "x")
	if m.pendingConfirm != nil {
		t.Fatalf("x on a flat position opened a confirm preview")
	}
	if !strings.Contains(m.status, "no position") {
		t.Fatalf("status = %q, want mentions no position", m.status)
	}
	if len(mock.Submitted) != 0 {
		t.Fatalf("flatten on a flat position submitted an order")
	}
}

func TestFlattenKeyLongRequiresConfirm(t *testing.T) {
	mock := &conn.MockGateway{}
	m := withBook(newModel(mock))
	m = apply(m, wire.Fill{Oid: 1, Px: 9_998, Qty: 7, Side: wire.Buy})

	m = press(m, "x")
	want := wire.OrderReq{Side: wire.Sell, Px: 9_999, Qty: 7, Tif: wire.Ioc, ReduceOnly: true}
	if m.pendingConfirm == nil || *m.pendingConfirm != want {
		t.Fatalf("preview after x = %+v, want %+v", m.pendingConfirm, want)
	}
	if len(mock.Submitted) != 0 {
		t.Fatalf("x submitted before the confirm enter")
	}

	m = press(m, "enter")
	if len(mock.Submitted) != 1 || mock.Submitted[0] != want {
		t.Fatalf("submitted = %+v, want [%+v]", mock.Submitted, want)
	}
}

func TestFlattenKeyNoBookIsNoOp(t *testing.T) {
	mock := &conn.MockGateway{}
	m := newModel(mock) // no book
	m = apply(m, wire.Fill{Oid: 1, Px: 9_998, Qty: 7, Side: wire.Buy})

	m = press(m, "x")
	if m.pendingConfirm != nil {
		t.Fatalf("x opened a confirm preview with no book to price against")
	}
	if !strings.Contains(m.status, "no live book") {
		t.Fatalf("status = %q, want mentions no live book", m.status)
	}
}

// stripANSI removes SGR escape sequences so tests can assert on visible text.
func stripANSI(s string) string {
	for strings.Contains(s, "\x1b") {
		i := strings.Index(s, "\x1b")
		j := strings.Index(s[i:], "m")
		if j < 0 {
			break
		}
		s = s[:i] + s[i+j+1:]
	}
	return s
}
