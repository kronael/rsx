package ui

import (
	"strings"
	"testing"

	"rsx-term/wire"
)

func TestLadderRow(t *testing.T) {
	bid := ladderRow(9999, 15, true, 0, false, 5, 3, " ")
	if !strings.Contains(bid, "15") || !strings.Contains(bid, "9999") {
		t.Fatalf("bid row missing qty/price: %q", bid)
	}
	ask := ladderRow(10002, 0, false, 20, true, 5, 3, " ")
	if !strings.Contains(ask, "20") || !strings.Contains(ask, "10002") {
		t.Fatalf("ask row missing qty/price: %q", ask)
	}
	empty := ladderRow(10005, 0, false, 0, false, 5, 3, " ")
	if !strings.Contains(empty, "10005") {
		t.Fatalf("empty row should still show its price: %q", empty)
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
