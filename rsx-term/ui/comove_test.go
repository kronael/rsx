package ui

import (
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/conn"
	"rsx-term/wire"
)

func TestCoMoveTogetherInverseIndependent(t *testing.T) {
	up := []int64{1, 2, 3, 4, 5, 6}
	down := []int64{6, 5, 4, 3, 2, 1}
	if co, ok := coMove(up, up); !ok || co != 1 {
		t.Fatalf("identical series co-move = %v (ok=%v), want +1", co, ok)
	}
	if co, ok := coMove(up, down); !ok || co != -1 {
		t.Fatalf("opposite series co-move = %v (ok=%v), want -1", co, ok)
	}
	// Too few shared bins → not enough to be honest.
	if _, ok := coMove([]int64{1, 2}, up); ok {
		t.Fatalf("short series must report ok=false")
	}
	// Uncorrelated: a zig-zag against a steady rise → mixed sign → near 0.
	zig := []int64{1, 2, 1, 2, 1, 2}
	co, ok := coMove(zig, up)
	if !ok || co > 0.5 || co < -0.5 {
		t.Fatalf("uncorrelated co-move = %v, want near 0", co)
	}
}

func TestCoMoveRefPrefersBtcThenEth(t *testing.T) {
	withBtc := New(Config{
		Symbol: "SOL-PERP", SymbolID: 2, Stream: true,
		Instruments: []Instrument{
			{ID: 2, Name: "SOL-PERP"},
			{ID: 3, Name: "ETH-PERP"},
			{ID: 1, Name: "BTC-PERP"},
		},
	})
	if ref, ok := withBtc.coMoveRef("rsx"); !ok || ref.Name != "BTC-PERP" {
		t.Fatalf("ref should prefer BTC, got %q (ok=%v)", ref.Name, ok)
	}
	noBtc := New(Config{
		Symbol: "SOL-PERP", SymbolID: 2, Stream: true,
		Instruments: []Instrument{
			{ID: 2, Name: "SOL-PERP"},
			{ID: 3, Name: "ETH-PERP"},
		},
	})
	if ref, ok := noBtc.coMoveRef("rsx"); !ok || ref.Name != "ETH-PERP" {
		t.Fatalf("ref should fall back to ETH, got %q (ok=%v)", ref.Name, ok)
	}
}

// coMoveModel is a two-symbol (BTC reference + SOL) stream model, sized.
func coMoveModel(t *testing.T) Model {
	t.Helper()
	m := New(Config{
		Symbol:   "BTC-PERP",
		SymbolID: 1,
		Sub:      &conn.MockGateway{},
		PriceDec: 1,
		QtyDec:   4,
		Tick:     1,
		Stream:   true,
		Instruments: []Instrument{
			{ID: 1, Name: "BTC-PERP", PriceDec: 1, QtyDec: 4, Tick: 1, Sector: "majors"},
			{ID: 2, Name: "SOL-PERP", PriceDec: 1, QtyDec: 4, Tick: 1, Sector: "majors"},
		},
	})
	return apply(m, tea.WindowSizeMsg{Width: 110, Height: 32})
}

func TestNewsCoMovementOverview(t *testing.T) {
	m := coMoveModel(t)
	// Both mids rise together, bin after bin: SOL should read as strongly
	// co-moving with the BTC reference.
	for i := 0; i < 8; i++ {
		px := int64(1000 + i)
		m = apply(m, wire.Bbo{SymbolID: 1, BidPx: px, AskPx: px + 1, BidQty: 1, AskQty: 1})
		m = apply(m, wire.Bbo{SymbolID: 2, BidPx: px, AskPx: px + 1, BidQty: 1, AskQty: 1})
		m = apply(m, binTickMsg(time.Unix(1_700_000_000+int64(i), 0)))
	}
	m = press(m, "tab") // book → news
	plain := stripANSI(m.View())
	if !strings.Contains(plain, "co-move vs BTC ~6s") {
		t.Fatalf("news view should show the co-move overview vs BTC:\n%s", plain)
	}
	if !strings.Contains(plain, "≡ SOL") && !strings.Contains(plain, "= SOL") {
		t.Fatalf("SOL should read as co-moving with BTC:\n%s", plain)
	}
}

func TestNewsCoMovementInverse(t *testing.T) {
	m := coMoveModel(t)
	// BTC rises while SOL falls, bin after bin: SOL reads as inverse (≠).
	for i := 0; i < 8; i++ {
		up, dn := int64(1000+i), int64(1000-i)
		m = apply(m, wire.Bbo{SymbolID: 1, BidPx: up, AskPx: up + 1, BidQty: 1, AskQty: 1})
		m = apply(m, wire.Bbo{SymbolID: 2, BidPx: dn, AskPx: dn + 1, BidQty: 1, AskQty: 1})
		m = apply(m, binTickMsg(time.Unix(1_700_000_000+int64(i), 0)))
	}
	m = press(m, "tab")
	if !strings.Contains(stripANSI(m.View()), "≠ SOL") {
		t.Fatalf("SOL moving against BTC should read as inverse (≠):\n%s", stripANSI(m.View()))
	}
}
