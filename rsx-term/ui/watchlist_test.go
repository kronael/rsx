package ui

import (
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/conn"
	"rsx-term/wire"
)

// watchModel builds a two-symbol stream model with live books on both, sized,
// with one bin folded, on the book screen. The second symbol gives the news
// breadth map and the book switcher something to switch to.
func watchModel(t *testing.T, mock *conn.MockGateway) Model {
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
	return m
}

func TestScreenCycle(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	want := []screen{screenNews, screenLLM, screenBook}
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

func TestBookSymbolSwitcher(t *testing.T) {
	m := watchModel(t, &conn.MockGateway{})
	if m.screen != screenBook {
		t.Fatalf("expected book screen, got %v", m.screen)
	}
	m = press(m, "x")
	if !m.switching {
		t.Fatalf("x should open the symbol switcher")
	}
	m = press(m, m.instrumentFor("rsx", 11).Code)
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
	m := watchModel(t, &conn.MockGateway{})
	m = apply(m, wire.Fill{Oid: 1, Px: 9999, Qty: 100, Side: wire.Buy, Symbol: 10})
	m = apply(m, wire.Fill{Oid: 2, Px: 21000, Qty: 50, Side: wire.Sell, Symbol: 11})
	if got := m.marketFor("rsx", 10).position.Net; got != 100 {
		t.Fatalf("symbol 10 net = %d, want +100", got)
	}
	if got := m.marketFor("rsx", 11).position.Net; got != -50 {
		t.Fatalf("symbol 11 net = %d, want -50", got)
	}
}

func TestBookReduceOnlyToggleApplies(t *testing.T) {
	mock := &conn.MockGateway{}
	m := watchModel(t, mock)
	m = press(m, "r") // persistent reduce-only mode (global toggle)
	m = press(m, "f") // place at the own-side touch
	if len(mock.Submitted) != 1 || !mock.Submitted[0].ReduceOnly {
		t.Fatalf("RO toggle must apply to book orders: %+v", mock.Submitted)
	}
	if !strings.Contains(stripANSI(m.View()), " RO ") {
		t.Fatalf("mode line should show the RO toggle prominently")
	}
}

func TestBookNotionalCapBlocks(t *testing.T) {
	mock := &conn.MockGateway{}
	m := New(Config{
		Symbol: "PENGU-PERP", SymbolID: 10, Sub: mock, Stream: true,
		PriceDec: 6, QtyDec: 4, Tick: 1,
		MaxNotional: 1,                              // 1 quote-unit ceiling
		SizePresets: []int64{2_000_000, 1, 1, 1, 1}, // preset 1 clears the qty cap…
	})
	m = apply(m, tea.WindowSizeMsg{Width: 100, Height: 30})
	m = apply(m, wire.Snapshot{
		SymbolID: 10,
		Bids:     []wire.Level{{Px: 9999, Qty: 5, Count: 1}},
		Asks:     []wire.Level{{Px: 10001, Qty: 5, Count: 1}},
		Seq:      1,
	})
	m = press(m, "1") // arm the big preset
	m = press(m, "f") // place at the touch — …but the notional guard bites
	if len(mock.Submitted) != 0 {
		t.Fatalf("over-notional order must be BLOCKED, got %+v", mock.Submitted)
	}
	if !strings.Contains(m.status, "BLOCKED") {
		t.Fatalf("status should say BLOCKED: %q", m.status)
	}
}
