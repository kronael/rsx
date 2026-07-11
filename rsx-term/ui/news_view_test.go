package ui

import (
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/conn"
	"rsx-term/news"
	"rsx-term/wire"
)

// fakeNews is a fixed in-memory news source for view tests.
type fakeNews struct {
	markers []news.Marker // newest first
}

func (f *fakeNews) Markers(sinceNs, untilNs int64) []news.Marker {
	var out []news.Marker
	for _, m := range f.markers {
		if m.TsNs >= sinceNs && m.TsNs <= untilNs {
			out = append(out, m)
		}
	}
	return out
}

func (f *fakeNews) All() []news.Marker { return f.markers }
func (f *fakeNews) Enabled() bool      { return true }

// newsModel is a two-symbol stream model with a fake feed, on the news
// screen.
func newsModel(t *testing.T, mock *conn.MockGateway) Model {
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
			{ID: 10, Name: "PENGU-PERP", PriceDec: 6, QtyDec: 4, Tick: 1, Sector: "meme"},
			{ID: 3, Name: "SOL-PERP", PriceDec: 4, QtyDec: 6, Tick: 1, Sector: "majors"},
		},
		News: &fakeNews{markers: []news.Marker{
			{TsNs: 2e18, Text: "Exchange halts withdrawals", Source: "Twitter", Tier: 3},
			{TsNs: 1e18, Text: "Binance lists SOL pair", Source: "Blogs", Symbols: []string{"SOLUSDT"}, Tier: 2},
		}},
	})
	m = apply(m, tea.WindowSizeMsg{Width: 110, Height: 32})
	m = apply(m, wire.Snapshot{
		SymbolID: 3,
		Bids:     []wire.Level{{Px: 1_499_950, Qty: 40_000_000, Count: 2}},
		Asks:     []wire.Level{{Px: 1_500_050, Qty: 35_000_000, Count: 1}},
		Seq:      1,
	})
	m = apply(m, binTickMsg(time.Now()))
	m = press(m, "tab")
	m = press(m, "tab") // book → pair → news
	return m
}

func TestNewsViewSectorMapAndFeed(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{})
	plain := stripANSI(m.View())
	for _, want := range []string{"majors", "meme", "SOL", "PENGU", "halts withdrawals", "lists SOL pair", "‼"} {
		if !strings.Contains(plain, want) {
			t.Fatalf("news view missing %q:\n%s", want, plain)
		}
	}
	if got := strings.Count(m.View(), "\n") + 1; got != 32 {
		t.Fatalf("news view = %d lines, want a fixed 32", got)
	}
}

func TestNewsSearchFilters(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{})
	m = press(m, "/")
	for _, r := range "sol" {
		m = press(m, string(r))
	}
	plain := stripANSI(m.View())
	if strings.Contains(plain, "halts withdrawals") {
		t.Fatalf("search should filter the feed:\n%s", plain)
	}
	if !strings.Contains(plain, "lists SOL pair") || !strings.Contains(plain, "search: sol_") {
		t.Fatalf("query row missing:\n%s", plain)
	}
	// While typing, letters must NOT jump views or trade.
	if m.screen != screenNews {
		t.Fatalf("typing in search left the news screen")
	}
	m = press(m, "esc")
	if m.newsQuery != "" {
		t.Fatalf("esc should clear the search")
	}
}

func TestNewsSelectionAndHandoff(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{})
	m = press(m, "j") // select the second (older, SOL-linked) headline
	m = press(m, "enter")
	if m.screen != screenLLM {
		t.Fatalf("enter should open the assistant, got %v", m.screen)
	}
	if m.assistCtx == nil {
		t.Fatalf("handoff must package a context")
	}
	ctx := *m.assistCtx
	if ctx.Symbol != "SOL-PERP" || ctx.Venue != "rsx" {
		t.Fatalf("headline should link to SOL's market: %+v", ctx)
	}
	if ctx.Headline.Text != "Binance lists SOL pair" {
		t.Fatalf("wrong headline packaged: %q", ctx.Headline.Text)
	}
	if len(ctx.Bids) != 1 || ctx.Bids[0].Px != 1_499_950 {
		t.Fatalf("book snapshot not frozen into the context: %+v", ctx.Bids)
	}
	if ctx.MidPx != 1_500_000 {
		t.Fatalf("mid at handoff = %d", ctx.MidPx)
	}
	// The snapshot is a copy: the live book folding on must not mutate it.
	m = apply(m, wire.Delta{SymbolID: 3, Side: 0, Px: 1_499_950, Qty: 0, Seq: 2})
	if len(m.assistCtx.Bids) != 1 || m.assistCtx.Bids[0].Qty != 40_000_000 {
		t.Fatalf("handoff context must stay frozen: %+v", m.assistCtx.Bids)
	}
}

func TestLLMViewRendersHandoffAndPlaceholder(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{})
	m = press(m, "j")
	m = press(m, "enter")
	plain := stripANSI(m.View())
	for _, want := range []string{"ASSISTANT", "rsx · SOL-PERP", "lists SOL pair", "150.0000", "placeholder"} {
		if !strings.Contains(plain, want) {
			t.Fatalf("assistant view missing %q:\n%s", want, plain)
		}
	}
	m = press(m, "esc")
	if m.screen != screenNews {
		t.Fatalf("esc should return to the news view")
	}
}

func TestLLMViewWithoutContext(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{})
	m = press(m, "tab") // news → llm without a handoff
	plain := stripANSI(m.View())
	if !strings.Contains(plain, "no context yet") {
		t.Fatalf("empty assistant should say so:\n%s", plain)
	}
}

func TestNewsLetterJumpsToBook(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{})
	code := m.instrumentFor("rsx", 3).Code
	m = press(m, code)
	if m.screen != screenBook || m.active != 3 {
		t.Fatalf("letter should jump into the symbol's book: screen %v active %d", m.screen, m.active)
	}
}

func TestBookNKeyOpensNews(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	m = press(m, "n")
	if m.screen != screenNews {
		t.Fatalf("n should open the news view from the book")
	}
}
