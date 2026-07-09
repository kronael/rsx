package ui

import (
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/book"
	"rsx-term/conn"
	"rsx-term/wire"
)

// clickAt sends a left button-press at (x,y) and returns the updated model.
func clickAt(m Model, x, y int) Model {
	got, _ := m.handleMouse(tea.MouseMsg{Action: tea.MouseActionPress, Button: tea.MouseButtonLeft, X: x, Y: y})
	return got.(Model)
}

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

func TestFmtNotionalQuotePrecision(t *testing.T) {
	m := New(Config{PriceDec: 6, QtyDec: 4})
	// 5 tokens (raw 50000) @ $0.010001 (raw 10001): raw product 500050000,
	// money = $0.050005 shown at quote (price) precision, not 10 decimals.
	if got := m.fmtNotional(10001 * 50000); got != "0.050005" {
		t.Fatalf("notional = %q, want 0.050005 (no trailing qty-dec zeros)", got)
	}
	// A loss keeps its sign and truncates toward zero.
	if got := m.fmtNotional(-2_500_000); got != "-0.000250" {
		t.Fatalf("negative notional = %q, want -0.000250", got)
	}
}

func TestRiskRowIsHonestlyDashed(t *testing.T) {
	m := New(Config{PriceDec: 6, QtyDec: 4})
	out := stripANSI(m.viewRiskRow())
	for _, want := range []string{"liq —", "ROE —", "mgn", "needs risk engine"} {
		if !strings.Contains(out, want) {
			t.Fatalf("risk row missing %q:\n%s", want, out)
		}
	}
	// It must never fabricate a number — no digits in the dashed figures.
	if strings.ContainsAny(strings.ReplaceAll(out, "░", ""), "0123456789") {
		t.Fatalf("risk row must not show a fabricated number:\n%s", out)
	}
}

func TestNarrowStacksPanels(t *testing.T) {
	m := New(Config{PriceDec: 6, QtyDec: 4, Tick: 1, Symbol: "PENGU-PERP"})
	m.gwConnected, m.mdConnected = true, true
	m.book.Asks = []wire.Level{{Px: 10002, Qty: 60000}}
	m.book.Bids = []wire.Level{{Px: 10000, Qty: 70000}}
	m.recenterLadder()
	m.width, m.height = 90, 30 // 90 < bookWidth+orderWidth+rightWidth (114)
	if !m.narrow() {
		t.Fatal("90 cols should be narrow")
	}
	sharesLine := func(out, a, b string) bool {
		for _, ln := range strings.Split(out, "\n") {
			if strings.Contains(ln, a) && strings.Contains(ln, b) {
				return true
			}
		}
		return false
	}
	narrow := stripANSI(m.viewMain())
	if sharesLine(narrow, "book", "positions") {
		t.Fatal("narrow layout must stack: book and positions should not share a line")
	}
	if !strings.Contains(narrow, "book") || !strings.Contains(narrow, "positions") {
		t.Fatal("narrow layout dropped a panel")
	}
	// Wide: the three titles ride the same top line (horizontal join).
	m.width = 132
	if m.narrow() {
		t.Fatal("132 cols should not be narrow")
	}
	if !sharesLine(stripANSI(m.viewMain()), "book", "positions") {
		t.Fatal("wide layout should place book and positions side by side")
	}
}

func TestParseRaw(t *testing.T) {
	cases := []struct {
		s    string
		dec  int
		want int64
		ok   bool
	}{
		{"0.010001", 6, 10001, true}, // PENGU price the trader reads
		{"5", 4, 50000, true},        // "5" tokens -> raw at 4dp
		{"5.5", 4, 55000, true},
		{".5", 4, 5000, true},
		{"10001", 0, 10001, true},  // 0 decimals: raw == typed
		{"1.5", 0, 0, false},       // fractional at 0 decimals rejected
		{"0.0000001", 6, 0, false}, // more precision than the instrument
		{"1.2.3", 4, 0, false},     // second dot
		{"", 4, 0, false},
		{"1x", 4, 0, false},
	}
	for _, c := range cases {
		got, ok := parseRaw(c.s, c.dec)
		if ok != c.ok || (ok && got != c.want) {
			t.Fatalf("parseRaw(%q,%d) = (%d,%v), want (%d,%v)", c.s, c.dec, got, ok, c.want, c.ok)
		}
	}
}

func TestDecimalInputSubmitsRaw(t *testing.T) {
	mock := &conn.MockGateway{}
	m := New(Config{Sub: mock, PriceDec: 6, QtyDec: 4, Tick: 1})
	// Trader types the price and qty they READ off the ladder.
	m = typeDigits(m, "0.010001") // '.' routes through appendDot
	m = press(m, "tab")
	m = typeDigits(m, "5")
	m = press(m, "enter") // preview
	m = press(m, "enter") // send
	if len(mock.Submitted) != 1 {
		t.Fatalf("submitted %d, want 1", len(mock.Submitted))
	}
	got := mock.Submitted[0]
	if got.Px != 10001 || got.Qty != 50000 {
		t.Fatalf("decimal input -> raw wrong: Px=%d Qty=%d, want 10001/50000", got.Px, got.Qty)
	}
	if !strings.Contains(m.status, "0.010001") || !strings.Contains(m.status, "5.0000") {
		t.Fatalf("sent status not in decimals: %q", m.status)
	}
}

func TestAppendDotRules(t *testing.T) {
	m := New(Config{PriceDec: 6, QtyDec: 4})
	m = typeDigits(m, "0.0.5") // second dot ignored
	if strings.Count(m.pxBuf, ".") != 1 {
		t.Fatalf("second dot should be ignored: %q", m.pxBuf)
	}
	// leading dot expands to 0.
	m2 := New(Config{PriceDec: 6})
	m2 = press(m2, ".")
	if m2.pxBuf != "0." {
		t.Fatalf("leading dot should expand to 0.: %q", m2.pxBuf)
	}
	// no fractional precision -> dot ignored
	m3 := New(Config{PriceDec: 0})
	m3 = press(m3, ".")
	if m3.pxBuf != "" {
		t.Fatalf("dot at 0 decimals should be ignored: %q", m3.pxBuf)
	}
}

func TestPriceAtY(t *testing.T) {
	m := New(Config{PriceDec: 6, QtyDec: 4, Tick: 1})
	m.mdConnected = true
	m.book.Asks = []wire.Level{{Px: 10002, Qty: 1}}
	m.book.Bids = []wire.Level{{Px: 10000, Qty: 1}}
	m.ladderCenter = 10001
	m.width, m.height = 120, 29 // half = clamp((29-9)/2,1,20) = 10
	half := m.ladderRows()
	if half != 10 {
		t.Fatalf("test setup: half=%d, want 10", half)
	}
	// firstLevelY=3: top row is center+half, each row down is -1.
	cases := []struct {
		y    int
		want int64
		ok   bool
	}{
		{3, 10011, true},  // i=0  -> center+half
		{13, 10001, true}, // i=10 -> center
		{23, 9991, true},  // i=20 -> center-half
		{24, 0, false},    // i=21 -> below the ladder (imbalance bar)
		{2, 0, false},     // above the first level
	}
	for _, c := range cases {
		got, ok := m.priceAtY(c.y)
		if ok != c.ok || (ok && got != c.want) {
			t.Fatalf("priceAtY(%d) = (%d,%v), want (%d,%v)", c.y, got, ok, c.want, c.ok)
		}
	}
	// ARMED shifts every row down by one (banner line).
	m.armed = true
	if got, ok := m.priceAtY(4); !ok || got != 10011 {
		t.Fatalf("armed priceAtY(4) = (%d,%v), want (10011,true)", got, ok)
	}
	// Narrow layout: mapping is disabled (offsets differ when stacked).
	m.armed, m.width = false, 90
	if _, ok := m.priceAtY(3); ok {
		t.Fatal("priceAtY must be disabled in the narrow/stacked layout")
	}
}

func TestClickSetsPrice(t *testing.T) {
	m := New(Config{PriceDec: 6, QtyDec: 4, Tick: 1})
	m.mdConnected = true
	m.book.Asks = []wire.Level{{Px: 10002, Qty: 1}}
	m.book.Bids = []wire.Level{{Px: 10000, Qty: 1}}
	m.ladderCenter = 10001
	m.width, m.height = 120, 29
	got := clickAt(m, 5, 13)
	if got.pxBuf != "0.010001" { // center row, as a human decimal
		t.Fatalf("click set pxBuf=%q, want 0.010001", got.pxBuf)
	}
	if got.focus != FocusPx {
		t.Fatal("click should focus the price field")
	}
	// A click outside the book column is ignored.
	if got2 := clickAt(m, bookWidth+5, 13); got2.pxBuf != "" {
		t.Fatalf("click outside book column set pxBuf=%q", got2.pxBuf)
	}
	// Motion (not a press) is ignored.
	motion := tea.MouseMsg{Action: tea.MouseActionMotion, Button: tea.MouseButtonLeft, X: 5, Y: 13}
	if got3, _ := m.handleMouse(motion); got3.(Model).pxBuf != "" {
		t.Fatal("mouse motion should not set a price")
	}
}
