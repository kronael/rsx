package ui

import (
	"strings"
	"testing"

	"rsx-term/wire"
)

func TestLadderRow(t *testing.T) {
	m := New(Config{}) // 0 decimals: prices/qtys render as raw ints
	bid := stripANSI(m.ladderRow(9999, 15, 0, 30, true, false, 5, 3, " "))
	if !strings.Contains(bid, "15") || !strings.Contains(bid, "9999") {
		t.Fatalf("bid row missing qty/price: %q", bid)
	}
	if !strings.Contains(bid, depthGlyph) {
		t.Fatalf("bid row missing depth bar: %q", bid)
	}
	ask := stripANSI(m.ladderRow(10002, 0, 20, 30, false, true, 5, 3, " "))
	if !strings.Contains(ask, "20") || !strings.Contains(ask, "10002") {
		t.Fatalf("ask row missing qty/price: %q", ask)
	}
	empty := stripANSI(m.ladderRow(10005, 0, 0, 30, false, false, 5, 3, " "))
	if !strings.Contains(empty, "10005") {
		t.Fatalf("empty row should still show its price: %q", empty)
	}
	if strings.Contains(empty, depthGlyph) {
		t.Fatalf("empty row should have no depth bar: %q", empty)
	}
}

func TestDepthBarScales(t *testing.T) {
	// The deepest level fills the full width; half-depth fills ~half.
	full := stripANSI(depthBar(100, 100, 6, StyleLive, false))
	if n := strings.Count(full, depthGlyph); n != 6 {
		t.Fatalf("full depth bar = %d cells, want 6", n)
	}
	half := stripANSI(depthBar(50, 100, 6, StyleLive, false))
	if n := strings.Count(half, depthGlyph); n != 3 {
		t.Fatalf("half depth bar = %d cells, want 3", n)
	}
	// Any nonzero qty shows at least one cell (never vanishes).
	tiny := stripANSI(depthBar(1, 1000, 6, StyleLive, false))
	if n := strings.Count(tiny, depthGlyph); n != 1 {
		t.Fatalf("tiny depth bar = %d cells, want 1", n)
	}
	// Zero qty shows nothing.
	if n := strings.Count(stripANSI(depthBar(0, 100, 6, StyleLive, false)), depthGlyph); n != 0 {
		t.Fatalf("zero depth bar = %d cells, want 0", n)
	}
}

func TestRecenterLadder(t *testing.T) {
	m := &Model{}
	m.book.Bids = []wire.Level{{Px: 100}}
	m.book.Asks = []wire.Level{{Px: 102}}
	m.recenterLadder()
	if m.ladderCenter != 101 {
		t.Fatalf("initial centre should be the mid: %d", m.ladderCenter)
	}
	m.book.Bids[0].Px, m.book.Asks[0].Px = 102, 104 // mid 103, drift 2 < band
	m.recenterLadder()
	if m.ladderCenter != 101 {
		t.Fatalf("small drift must NOT reshuffle the axis: %d", m.ladderCenter)
	}
	m.book.Bids[0].Px, m.book.Asks[0].Px = 120, 122 // mid 121, drift 20 > band
	m.recenterLadder()
	if m.ladderCenter != 121 {
		t.Fatalf("large drift should recentre: %d", m.ladderCenter)
	}
}
