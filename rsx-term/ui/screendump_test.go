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
	dump("4-help-book", press(m, "?"))                 // BOOK key overlay
	dump("5-switcher", press(m, "x"))                  // symbol-switcher chord (candidate list)
	dump("8-llm-empty", press(press(m, "tab"), "tab")) // offline LLM, no context

	// Drive several moving books across bin ticks so the NEWS majors tiles show
	// real moves and the co-move overview accumulates enough shared bins to read
	// (offline default is dashes — this is the harness feeding data, not the app
	// fabricating it).
	m = driveMajors(m)
	mn := press(m, "tab") // book → news (populated)
	dump("2-news", mn)
	dump("7-help-news", press(mn, "?")) // NEWS key overlay (context-sensitive)

	// Hand the top headline (with its linked market's frozen book) to the
	// assistant so the LLM pane shows a populated context over the offline
	// placeholder reply.
	dump("3-llm-context", press(mn, "enter"))

	// The venue picker needs a second venue to be meaningful; dump it from a
	// dedicated two-venue model so it doesn't shift the primary model's news
	// breadth venue.
	dump("6-venue", press(twoVenueModel(), "f9"))
}

// driveMajors feeds moving single-level books for BTC/ETH/SOL/XRP (and a
// wiggling PENGU) across bin ticks — BTC trends up, ETH tracks it, SOL runs
// inverse, XRP drifts — so the NEWS view's move tiles and co-move overview have
// real, varied history to render.
func driveMajors(m Model) Model {
	majors := []struct {
		id         uint32
		base, step int64
	}{
		{1, 500000, 400},   // BTC  ▲ up
		{2, 300000, 180},   // ETH  ▲ tracks BTC
		{3, 1500000, -900}, // SOL  ▼ inverse
		{4, 60000, 20},     // XRP  ~ small drift
	}
	for i := 0; i < 8; i++ {
		for _, s := range majors {
			c := s.base + s.step*int64(i)
			m = apply(m, wire.Snapshot{
				SymbolID: s.id,
				Bids:     []wire.Level{{Px: c - 1, Qty: 40, Count: 3}},
				Asks:     []wire.Level{{Px: c + 1, Qty: 40, Count: 3}},
				Seq:      uint64(100 + i),
			})
		}
		m = apply(m, wire.Snapshot{
			SymbolID: 10,
			Bids:     []wire.Level{{Px: 10000 - int64(i), Qty: 30, Count: 3}},
			Asks:     []wire.Level{{Px: 10002 + int64(i), Qty: 20, Count: 2}},
			Seq:      uint64(200 + i),
		})
		m = apply(m, binTickMsg(time.Now()))
	}
	return m
}

// twoVenueModel is a sized stream model with a second (read-only) venue, so the
// F9 venue picker has more than one venue to list.
func twoVenueModel() Model {
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
		},
		Venues: []VenueConfig{{
			Name:        "hyperliquid",
			Code:        "h",
			Instruments: []Instrument{{ID: 1, Name: "BTC-PERP", PriceDec: 1, QtyDec: 6, Tick: 1, Sector: "majors"}},
		}},
	})
	return apply(m, tea.WindowSizeMsg{Width: 120, Height: 40})
}
