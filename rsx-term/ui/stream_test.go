package ui

import (
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/conn"
	"rsx-term/wire"
)

// streamModel builds a stream-mode model over a mock, sized to a known
// terminal, with a live book folded and one bin sealed (so the axis is
// anchored).
func streamModel(t *testing.T, mock *conn.MockGateway) Model {
	t.Helper()
	m := New(Config{
		Symbol:   "PENGU-PERP",
		SymbolID: 10,
		Sub:      mock,
		PriceDec: 6,
		QtyDec:   4,
		Tick:     1,
		Stream:   true,
	})
	m = apply(m, tea.WindowSizeMsg{Width: 100, Height: 30})
	m = apply(m, wire.Snapshot{
		SymbolID: 10,
		Bids:     []wire.Level{{Px: 9999, Qty: 50000, Count: 1}, {Px: 9998, Qty: 80000, Count: 3}},
		Asks:     []wire.Level{{Px: 10001, Qty: 60000, Count: 1}, {Px: 10002, Qty: 40000, Count: 2}},
		Seq:      1,
	})
	m = apply(m, binTickMsg(time.Now()))
	return m
}

func TestDefaultViewUnchanged(t *testing.T) {
	// Stream OFF renders the classic DOM view (its book panel + help legend).
	// Byte-for-byte lock is TestDomViewGolden; this asserts mode selection.
	dom := stripANSI(New(Config{Symbol: "PENGU-PERP"}).View())
	if !strings.Contains(dom, "book") || !strings.Contains(dom, "q quit  b/s side") {
		t.Fatalf("default view should be the DOM view: %q", dom)
	}
	if strings.Contains(dom, "now") && strings.Contains(dom, "cursor") {
		t.Fatalf("default view must not render the stream chrome")
	}
	str := stripANSI(streamModel(t, &conn.MockGateway{}).View())
	for _, want := range []string{"mid", "now", "f place"} {
		if !strings.Contains(str, want) {
			t.Fatalf("stream view missing %q:\n%s", want, str)
		}
	}
}

func TestStreamFixedGrid(t *testing.T) {
	// The view is a FIXED grid repainted in place: exactly `height` lines,
	// whether the ring is empty or full — never an append-scroll log.
	m := streamModel(t, &conn.MockGateway{})
	if got := strings.Count(m.View(), "\n") + 1; got != 30 {
		t.Fatalf("fresh grid = %d lines, want 30", got)
	}
	for i := 0; i < 40; i++ {
		m = apply(m, binTickMsg(time.Now()))
	}
	if got := strings.Count(m.View(), "\n") + 1; got != 30 {
		t.Fatalf("full grid = %d lines, want 30", got)
	}
}

func TestStreamGutterShowsHorizons(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	plain := stripANSI(m.View())
	for _, want := range []string{"−10s", "−1m", "−2m", "now"} {
		if !strings.Contains(plain, want) {
			t.Fatalf("time gutter missing %q:\n%s", want, plain)
		}
	}
}

func TestSizeTierLogScaled(t *testing.T) {
	if got := sizeTier(0, 1000); got != 0 {
		t.Fatalf("zero size => tier 0, got %d", got)
	}
	if got := sizeTier(1, 1000); got < 1 {
		t.Fatalf("any nonzero size => at least tier 1, got %d", got)
	}
	small, big := sizeTier(5, 1000), sizeTier(1000, 1000)
	if big <= small {
		t.Fatalf("bigger size => higher tier: small %d, big %d", small, big)
	}
	if big != sizeTiers {
		t.Fatalf("the reference max should hit the top tier, got %d", big)
	}
}

func TestCellCountChannel(t *testing.T) {
	// Glyph = order count, colour = size: a whale (huge size, one order) shows
	// the faint density glyph; a wall of many orders shows the solid one.
	whale := cell{size: 1000, count: 1}
	wall := cell{size: 10, count: 20}
	if !strings.ContainsRune(cellStr(whale, 1000, 1, modeTrue), glyphs.countRamp[1]) {
		t.Fatalf("whale (count 1) should render the faint density glyph")
	}
	if !strings.ContainsRune(cellStr(wall, 1000, 1, modeTrue), glyphs.countRamp[4]) {
		t.Fatalf("wall (count 20) should render the solid density glyph")
	}
}

func TestCellPersistenceGlyph(t *testing.T) {
	held := cell{size: 100, count: 2, ageNs: int64(persistThreshold)}
	if !strings.ContainsRune(cellStr(held, 1000, 1, modeTrue), glyphs.persistent) {
		t.Fatalf("long-standing liquidity should render %q", glyphs.persistent)
	}
	fresh := cell{size: 100, count: 2, ageNs: int64(persistThreshold) - 1}
	if strings.ContainsRune(cellStr(fresh, 1000, 1, modeTrue), glyphs.persistent) {
		t.Fatalf("fresh liquidity must not carry the persistence mark")
	}
}

func TestCellTradeLayerCoEqual(t *testing.T) {
	// A print renders the aggressor-hued magnitude glyph OVER the resting
	// book — big prints pick a heavier glyph than small ones.
	small := cell{size: 100, count: 2, tradeQty: 2, tradeSide: wire.Sell}
	big := cell{size: 100, count: 2, tradeQty: 1000, tradeSide: wire.Buy}
	sGlyph := stripANSI(cellStr(small, 1000, 1000, modeTrue))
	bGlyph := stripANSI(cellStr(big, 1000, 1000, modeTrue))
	if sGlyph == bGlyph {
		t.Fatalf("trade magnitude must scale the glyph: small %q vs big %q", sGlyph, bGlyph)
	}
	if !strings.ContainsRune(string(glyphs.tradeRamp), rune([]rune(bGlyph)[0])) {
		t.Fatalf("trade glyph %q not from the trade ramp", bGlyph)
	}
}

func TestCellPlainDegrade(t *testing.T) {
	c := cell{size: 500, count: 3}
	s := cellStr(c, 1000, 1, modePlain)
	if strings.Contains(s, "\x1b") {
		t.Fatalf("plain mode must emit no colour escapes: %q", s)
	}
	if s != string(glyphs.countRamp[sizeTier(500, 1000)]) {
		t.Fatalf("plain glyph should encode the size tier: %q", s)
	}
}

func TestStreamFooterTouchLadder(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	plain := stripANSI(m.View())
	// Exact levels, nearest the touch first, at display precision.
	for _, want := range []string{"0.009999×5.0000", "0.009998×8.0000", "0.010001×6.0000"} {
		if !strings.Contains(plain, want) {
			t.Fatalf("touch ladder missing %q:\n%s", want, plain)
		}
	}
}

func TestGameEntryPresetAndPlace(t *testing.T) {
	mock := &conn.MockGateway{}
	m := streamModel(t, mock)
	m = press(m, "3") // arm preset 3 = 5 whole units
	m = press(m, "f") // place at the cursor (unset → joins own-side touch)
	if len(mock.Submitted) != 1 {
		t.Fatalf("f should fire exactly one order, got %d", len(mock.Submitted))
	}
	o := mock.Submitted[0]
	if o.Side != wire.Buy || o.Px != 9999 || o.Qty != 50000 || o.Tif != wire.Gtc {
		t.Fatalf("place = %+v, want BUY 50000 @ 9999 GTC", o)
	}
}

func TestGameEntryCursorMovesAndPlaces(t *testing.T) {
	mock := &conn.MockGateway{}
	m := streamModel(t, mock)
	m = press(m, "j") // snap to best bid 9999
	m = press(m, "h")
	m = press(m, "h") // two ticks deeper: 9997
	m = press(m, "f")
	if len(mock.Submitted) != 1 || mock.Submitted[0].Px != 9997 {
		t.Fatalf("cursor place = %+v, want px 9997", mock.Submitted)
	}
}

func TestGameEntryCrossIsIocAtFarTouch(t *testing.T) {
	mock := &conn.MockGateway{}
	m := streamModel(t, mock)
	m = press(m, "s") // sell side
	m = press(m, "@") // shift+2: cross with preset 2
	if len(mock.Submitted) != 1 {
		t.Fatalf("cross should fire one order, got %d", len(mock.Submitted))
	}
	o := mock.Submitted[0]
	if o.Side != wire.Sell || o.Px != 9999 || o.Qty != 20000 || o.Tif != wire.Ioc {
		t.Fatalf("cross = %+v, want SELL 20000 @ 9999 IOC", o)
	}
}

func TestGameEntryFatFingerStillBlocks(t *testing.T) {
	mock := &conn.MockGateway{}
	m := New(Config{
		Symbol: "PENGU-PERP", Sub: mock, Stream: true, Tick: 1,
		SizePresets: []int64{2_000_000, 1, 1, 1, 1},
	})
	m = apply(m, tea.WindowSizeMsg{Width: 100, Height: 30})
	m = apply(m, wire.Snapshot{
		Bids: []wire.Level{{Px: 9999, Qty: 5, Count: 1}},
		Asks: []wire.Level{{Px: 10001, Qty: 5, Count: 1}},
		Seq:  1,
	})
	m = press(m, "1")
	m = press(m, "f")
	if len(mock.Submitted) != 0 {
		t.Fatalf("over-cap order must be BLOCKED, got %+v", mock.Submitted)
	}
	if !strings.Contains(m.status, "BLOCKED") {
		t.Fatalf("status should say BLOCKED: %q", m.status)
	}
}

func TestGameEntryCancelNearestCursor(t *testing.T) {
	mock := &conn.MockGateway{}
	m := streamModel(t, mock)
	m = apply(m, wire.Accepted{Oid: 1, Cid: "a", Order: wire.OrderReq{Side: wire.Buy, Px: 9995, Qty: 1}})
	m = apply(m, wire.Accepted{Oid: 2, Cid: "b", Order: wire.OrderReq{Side: wire.Buy, Px: 9999, Qty: 1}})
	m = press(m, "j") // cursor to best bid 9999
	m = press(m, "d")
	if len(mock.Cancelled) != 1 || mock.Cancelled[0] != "b" {
		t.Fatalf("d should cancel the order nearest the cursor: %v", mock.Cancelled)
	}
}

func TestStreamOwnOrdersMarkedOnMapAndRuler(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	m = apply(m, wire.Accepted{Oid: 9, Cid: "c", Order: wire.OrderReq{Side: wire.Buy, Px: 9998, Qty: 30000}})
	plain := stripANSI(m.View())
	if !strings.ContainsRune(plain, glyphs.ownOrder) {
		t.Fatalf("own resting order must be marked %q on the map:\n%s", glyphs.ownOrder, plain)
	}
	if !strings.ContainsRune(plain, glyphs.ownBuy) {
		t.Fatalf("own buy must show %q on the ruler:\n%s", glyphs.ownBuy, plain)
	}
}

func TestStreamTapeRailShowsPrints(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	m = apply(m, wire.MdTrade{Px: 10001, Qty: 30000, TakerSide: 0, Seq: 2})
	plain := stripANSI(m.View())
	if !strings.Contains(plain, "┆") || !strings.Contains(plain, "0.010001") {
		t.Fatalf("tape rail should list the print:\n%s", plain)
	}
}

func TestStableBasisNeverFlickersDown(t *testing.T) {
	basis := foldBasis(1, 1000) // rise instantly
	if basis != 1000 {
		t.Fatalf("basis should jump to a new max, got %d", basis)
	}
	decayed := foldBasis(basis, 10) // decay slowly, never snap down
	if decayed >= basis || decayed < basis-basis>>basisDecayShift {
		t.Fatalf("basis should decay by 1/256 per bin: %d → %d", basis, decayed)
	}
	if foldBasis(1, 0) != 1 {
		t.Fatalf("basis floors at 1")
	}
}

func TestStreamHelpOverlay(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	m = press(m, "?")
	plain := stripANSI(m.View())
	if !strings.Contains(plain, "KEYS — BOOK") {
		t.Fatalf("? should open the book key reference:\n%s", plain)
	}
	// Generated from the keymap table: verbs, classes, and the rebind note.
	for _, want := range []string{"place resting limit", "⇧1-5", "RSX_TERM_KEYMAP"} {
		if !strings.Contains(plain, want) {
			t.Fatalf("help missing %q:\n%s", want, plain)
		}
	}
	m = press(m, "z")
	if strings.Contains(stripANSI(m.View()), "KEYS — BOOK") {
		t.Fatalf("any key should close the help overlay")
	}
	// Context-sensitive: on the news screen the overlay shows news verbs.
	m = press(m, "tab")
	m = press(m, "?")
	plain = stripANSI(m.View())
	if !strings.Contains(plain, "KEYS — NEWS") || !strings.Contains(plain, "hand the headline") {
		t.Fatalf("news help should show the news grammar:\n%s", plain)
	}
}
