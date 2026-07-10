package ui

import (
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/conn"
	"rsx-term/wire"
)

// pairModel builds a two-symbol stream model with live books on both, sized,
// with one bin folded, sitting on the pair screen.
func pairModel(t *testing.T, mock *conn.MockGateway) Model {
	t.Helper()
	m := New(Config{
		Symbol:   "PENGU-PERP",
		SymbolID: 10,
		Sub:      mock,
		PriceDec: 6,
		QtyDec:   4,
		Tick:     1,
		Stream:   true,
		Instruments: []Instrument{
			{ID: 10, Name: "PENGU-PERP", PriceDec: 6, QtyDec: 4, Tick: 1},
			{ID: 11, Name: "WIF-PERP", PriceDec: 6, QtyDec: 4, Tick: 1},
		},
	})
	m = apply(m, tea.WindowSizeMsg{Width: 100, Height: 30})
	m = apply(m, wire.Snapshot{
		SymbolID: 10,
		Bids:     []wire.Level{{Px: 9999, Qty: 50000, Count: 1}},
		Asks:     []wire.Level{{Px: 10001, Qty: 60000, Count: 1}},
		Seq:      1,
	})
	m = apply(m, wire.Snapshot{
		SymbolID: 11,
		Bids:     []wire.Level{{Px: 20995, Qty: 40, Count: 2}},
		Asks:     []wire.Level{{Px: 21010, Qty: 35, Count: 1}},
		Seq:      1,
	})
	m = apply(m, binTickMsg(time.Now()))
	m = press(m, "tab") // book → pair
	return m
}

func TestScreenCycle(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	want := []screen{screenPair, screenNews, screenLLM, screenBook}
	for _, s := range want {
		m = press(m, "tab")
		if m.screen != s {
			t.Fatalf("tab cycle: screen %v, want %v", m.screen, s)
		}
	}
	m = press(m, "shift+tab")
	if m.screen != screenLLM {
		t.Fatalf("shift+tab should cycle back, got %v", m.screen)
	}
}

func TestAssignCodesUniqueAndReserved(t *testing.T) {
	instruments := []Instrument{
		{ID: 1, Name: "PENGU-PERP"},
		{ID: 2, Name: "PEPE-PERP"}, // collides with PENGU on 'p'… both must resolve
		{ID: 3, Name: "BTC-PERP"},
		{ID: 4, Name: "SOL-PERP", Code: "z"}, // explicit code wins
	}
	assignCodes(instruments)
	seen := map[string]bool{}
	for _, ins := range instruments {
		if ins.Code == "" {
			t.Fatalf("%s got no code", ins.Name)
		}
		if seen[ins.Code] {
			t.Fatalf("duplicate code %q", ins.Code)
		}
		seen[ins.Code] = true
		for _, r := range ins.Code {
			if strings.ContainsRune("bsdrpqx.[]0123456789", r) {
				t.Fatalf("code %q uses a reserved action key", ins.Code)
			}
		}
	}
	if instruments[3].Code != "z" {
		t.Fatalf("explicit code overridden: %q", instruments[3].Code)
	}
}

func TestPairArmAndBuyLifted(t *testing.T) {
	mock := &conn.MockGateway{}
	m := pairModel(t, mock)
	code := m.instrumentFor(m.pairVenue(), 11).Code
	m = press(m, code) // arm WIF
	if m.armedSym != 11 {
		t.Fatalf("letter %q should arm symbol 11, got %d", code, m.armedSym)
	}
	m = press(m, "b") // buy 1 lot at market — lifts WIF's offer
	if len(mock.Submitted) != 1 {
		t.Fatalf("b should fire one order, got %d", len(mock.Submitted))
	}
	o := mock.Submitted[0]
	if o.Symbol != 11 || o.Side != wire.Buy || o.Px != 21010 || o.Tif != wire.Ioc {
		t.Fatalf("pair buy = %+v, want symbol 11 BUY IOC @ 21010", o)
	}
	// 1 lot = LotNotional(100) at px 0.021010 → 100/0.021010 ≈ 4759.6 units.
	wantQty := int64(100) * pow10(10) / 21010
	if o.Qty != wantQty {
		t.Fatalf("lot qty = %d, want %d", o.Qty, wantQty)
	}
}

func TestPairVimCountSells(t *testing.T) {
	mock := &conn.MockGateway{}
	m := pairModel(t, mock)
	m = press(m, m.instrumentFor(m.pairVenue(), 10).Code)
	m = press(m, "3")
	m = press(m, "s") // sell 3 lots — hits PENGU's bid
	if len(mock.Submitted) != 1 {
		t.Fatalf("s should fire one order, got %d", len(mock.Submitted))
	}
	o := mock.Submitted[0]
	oneLot := int64(100) * pow10(10) / 9999
	if o.Side != wire.Sell || o.Px != 9999 || o.Qty != 3*oneLot {
		t.Fatalf("3s = %+v, want SELL %d @ 9999", o, 3*oneLot)
	}
	if m.countBuf != "" {
		t.Fatalf("count buffer must clear after firing")
	}
}

func TestPairFlattenReduceOnly(t *testing.T) {
	mock := &conn.MockGateway{}
	m := pairModel(t, mock)
	m = apply(m, wire.Fill{Oid: 1, Px: 21000, Qty: 500, Side: wire.Buy, Symbol: 11})
	m = press(m, m.instrumentFor(m.pairVenue(), 11).Code)
	m = press(m, ".")
	if len(mock.Submitted) != 1 {
		t.Fatalf(". should fire one order, got %d", len(mock.Submitted))
	}
	o := mock.Submitted[0]
	if o.Symbol != 11 || o.Side != wire.Sell || o.Qty != 500 || !o.ReduceOnly || o.Px != 20995 {
		t.Fatalf("flatten = %+v, want reduce-only SELL 500 @ 20995", o)
	}
}

func TestPairUnarmedTradeIsNoop(t *testing.T) {
	mock := &conn.MockGateway{}
	m := pairModel(t, mock)
	m = press(m, "b")
	if len(mock.Submitted) != 0 {
		t.Fatalf("b with nothing armed must not trade: %+v", mock.Submitted)
	}
	if !strings.Contains(m.status, "arm a symbol") {
		t.Fatalf("status should coach the grammar: %q", m.status)
	}
}

func TestPairNotionalCapBlocks(t *testing.T) {
	mock := &conn.MockGateway{}
	m := pairModel(t, mock)
	m = press(m, m.instrumentFor(m.pairVenue(), 11).Code)
	m = press(m, "9")
	m = press(m, "9") // 99 lots ≈ 9,900 notional < default cap 10,000 — passes
	m = press(m, "b")
	if len(mock.Submitted) != 1 {
		t.Fatalf("99 lots should pass the default cap: %+v", mock.Submitted)
	}
	// Tighten the cap and retry: blocked outright.
	m.cfg.MaxNotional = 50
	m = press(m, m.instrumentFor(m.pairVenue(), 11).Code)
	m = press(m, "b")
	if len(mock.Submitted) != 1 {
		t.Fatalf("over-cap order must be BLOCKED, got %+v", mock.Submitted)
	}
	if !strings.Contains(m.status, "BLOCKED") {
		t.Fatalf("status should say BLOCKED: %q", m.status)
	}
}

func TestPairViewRowsAndArmHighlight(t *testing.T) {
	m := pairModel(t, &conn.MockGateway{})
	// ~2 lots: 1 lot = 100 notional / mid 0.021002 → 47_614_512 raw qty.
	m = apply(m, wire.Fill{Oid: 1, Px: 21000, Qty: 95_229_026, Side: wire.Buy, Symbol: 11})
	m = press(m, m.instrumentFor(m.pairVenue(), 11).Code)
	plain := stripANSI(m.View())
	for _, want := range []string{"PENGU-PERP", "WIF-PERP", "0.021002", "ARMED WIF-PERP", "L+2.0"} {
		if !strings.Contains(plain, want) {
			t.Fatalf("pair view missing %q:\n%s", want, plain)
		}
	}
	if got := strings.Count(m.View(), "\n") + 1; got != 30 {
		t.Fatalf("pair view = %d lines, want a fixed 30", got)
	}
}

func TestReduceOnlyToggleAppliesEverywhere(t *testing.T) {
	mock := &conn.MockGateway{}
	m := pairModel(t, mock)
	m = press(m, "r") // persistent RO mode
	m = press(m, m.instrumentFor(m.pairVenue(), 10).Code)
	m = press(m, "b")
	if len(mock.Submitted) != 1 || !mock.Submitted[0].ReduceOnly {
		t.Fatalf("RO toggle must apply to pair orders: %+v", mock.Submitted)
	}
	if !strings.Contains(stripANSI(m.View()), " RO ") {
		t.Fatalf("mode line should show the RO toggle prominently")
	}
}

func TestBookSymbolSwitcher(t *testing.T) {
	m := pairModel(t, &conn.MockGateway{})
	m = press(m, "tab")
	m = press(m, "tab")
	m = press(m, "tab") // pair → news → llm → book
	if m.screen != screenBook {
		t.Fatalf("expected book screen, got %v", m.screen)
	}
	m = press(m, "x")
	if !m.switching {
		t.Fatalf("x should open the symbol switcher")
	}
	m = press(m, m.instrumentFor(m.pairVenue(), 11).Code)
	if m.active != 11 || m.switching {
		t.Fatalf("code should switch the book instantly: active %d", m.active)
	}
	if !strings.Contains(stripANSI(m.View()), "WIF-PERP") {
		t.Fatalf("book header should show the new symbol")
	}
	// The new symbol's book is already folding: the footer shows its touch.
	if !strings.Contains(stripANSI(m.View()), "0.021010") {
		t.Fatalf("switched book should render WIF's ask touch:\n%s", stripANSI(m.View()))
	}
}

func TestPerSymbolPositionsIndependent(t *testing.T) {
	m := pairModel(t, &conn.MockGateway{})
	m = apply(m, wire.Fill{Oid: 1, Px: 9999, Qty: 100, Side: wire.Buy, Symbol: 10})
	m = apply(m, wire.Fill{Oid: 2, Px: 21000, Qty: 50, Side: wire.Sell, Symbol: 11})
	if got := m.marketFor("rsx", 10).position.Net; got != 100 {
		t.Fatalf("symbol 10 net = %d, want +100", got)
	}
	if got := m.marketFor("rsx", 11).position.Net; got != -50 {
		t.Fatalf("symbol 11 net = %d, want -50", got)
	}
}
