package ui

import (
	"os"
	"path/filepath"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/assistant"
	"rsx-term/conn"
	"rsx-term/news"
	"rsx-term/wire"
)

// TestDumpScreens renders every screen to ./tmp/screens/*.txt for visual
// review (the "see it" aid). Gated behind RSX_TERM_DUMP=1 so it never runs in
// the normal suite or CI — it writes files, asserts nothing.
func TestDumpScreens(t *testing.T) {
	if os.Getenv("RSX_TERM_DUMP") == "" {
		t.Skip("set RSX_TERM_DUMP=1 to dump rendered screens")
	}
	m := New(Config{
		Symbol:   "PENGU-PERP",
		SymbolID: 10,
		Sub:      &conn.MockGateway{},
		PriceDec: 6,
		QtyDec:   4,
		Tick:     1,
		Stream:   true,
		Instruments: []Instrument{
			{ID: 10, Name: "PENGU-PERP", PriceDec: 6, QtyDec: 4, Tick: 1, Sector: "meme"},
			{ID: 1, Name: "BTC-PERP", PriceDec: 1, QtyDec: 6, Tick: 1, Sector: "majors"},
			{ID: 2, Name: "ETH-PERP", PriceDec: 2, QtyDec: 5, Tick: 1, Sector: "majors"},
			{ID: 3, Name: "SOL-PERP", PriceDec: 4, QtyDec: 6, Tick: 1, Sector: "majors"},
			{ID: 4, Name: "XRP-PERP", PriceDec: 5, QtyDec: 4, Tick: 1, Sector: "majors"},
		},
		News: &fakeNews{markers: []news.Marker{
			{TsNs: 3e18, Text: "SOL ETF speculation lifts the majors", Source: "Twitter", Symbols: []string{"SOLUSDT"}, Tier: 3},
			{TsNs: 2e18, Text: "XRP appeal filed; volatility spikes", Source: "Blogs", Symbols: []string{"XRPUSDT"}, Tier: 2},
			{TsNs: 1e18, Text: "ETH staking inflows rise", Source: "Wire", Symbols: []string{"ETHUSDT"}, Tier: 1},
		}},
		Assist: assistant.New(""), // offline: renders the placeholder pane
	})
	m = apply(m, tea.WindowSizeMsg{Width: 120, Height: 40})
	m = apply(m, wire.Snapshot{
		SymbolID: 10,
		Bids:     []wire.Level{{Px: 10000, Qty: 30, Count: 3}, {Px: 9998, Qty: 15, Count: 2}, {Px: 9997, Qty: 30, Count: 5}},
		Asks:     []wire.Level{{Px: 10001, Qty: 5, Count: 1}, {Px: 10002, Qty: 20, Count: 4}, {Px: 10004, Qty: 12, Count: 2}},
		Seq:      1,
	})
	m = apply(m, binTickMsg(time.Now()))

	dir := "../tmp/screens"
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	dump := func(name string, mm Model) {
		if err := os.WriteFile(filepath.Join(dir, name+".txt"), []byte(stripANSI(mm.View())), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	dump("1-book", m)
	mn := press(m, "tab") // book → news
	dump("2-news", mn)
	ml := press(mn, "tab") // news → llm (offline placeholder)
	dump("3-llm", ml)
	mh := press(m, "?") // help overlay
	dump("4-help", mh)
}
