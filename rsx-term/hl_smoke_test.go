package main

import (
	"context"
	"os"
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/conn"
	"rsx-term/ui"
)

// TestHLStandaloneSmoke drives the standalone Hyperliquid terminal
// end-to-end against the REAL venue: meta fetch, WS subscribe, live folds,
// and a render of all four screens. Network-dependent — opt in with
// RSX_TERM_SMOKE_HL=1 (never runs in default/CI test passes).
func TestHLStandaloneSmoke(t *testing.T) {
	if os.Getenv("RSX_TERM_SMOKE_HL") != "1" {
		t.Skip("live-network smoke; set RSX_TERM_SMOKE_HL=1 to run")
	}
	meta, err := conn.FetchHLMeta()
	if err != nil {
		t.Fatalf("meta: %v", err)
	}
	hl := conn.NewHL(meta, []string{"BTC", "ETH", "SOL"})
	instruments := hlInstruments(hl)
	if len(instruments) != 3 {
		t.Fatalf("want BTC/ETH/SOL, got %d instruments", len(instruments))
	}
	first := instruments[0]
	model := ui.New(ui.Config{
		Symbol:      first.Name,
		SymbolID:    first.ID,
		Venue:       conn.HLVenueName,
		PriceDec:    first.PriceDec,
		QtyDec:      first.QtyDec,
		Tick:        first.Tick,
		Stream:      true,
		Instruments: instruments,
	})
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	hl.Start(ctx)
	defer hl.Close()

	var m tea.Model = model
	fold := func(msg tea.Msg) { m, _ = m.Update(msg) }
	fold(tea.WindowSizeMsg{Width: 120, Height: 34})

	deadline := time.After(15 * time.Second)
	binTick := time.NewTicker(100 * time.Millisecond)
	defer binTick.Stop()
	sawBook := false
	for !sawBook {
		select {
		case ev := <-hl.Events():
			fold(ev)
		case ts := <-binTick.C:
			fold(ui.BinTickAt(ts))
			view := stripANSI(m.View())
			if strings.Contains(view, "ask ") && !strings.Contains(view, "ask —") {
				sawBook = true
			}
		case <-deadline:
			t.Fatalf("no live HL book within 15s; last frame:\n%s", stripANSI(m.View()))
		}
	}

	// Every screen renders over live HL data at the fixed grid height.
	for i, want := range []string{"BOOK", "PAIR", "NEWS", "LLM"} {
		view := m.View()
		plain := stripANSI(view)
		if !strings.Contains(plain, want) {
			t.Fatalf("screen %d should be %s:\n%s", i, want, plain)
		}
		if got := strings.Count(view, "\n") + 1; got != 34 {
			t.Fatalf("%s grid = %d lines, want 34", want, got)
		}
		fold(tea.KeyMsg{Type: tea.KeyTab})
	}
	t.Logf("live HL book rendered (BTC/ETH/SOL), all four screens fixed-grid")
}

// stripANSI removes SGR escapes for assertions.
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
