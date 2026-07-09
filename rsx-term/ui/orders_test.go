package ui

import (
	"strings"
	"testing"

	"rsx-term/wire"
)

func TestOwnOrderLevels(t *testing.T) {
	m := Model{openOrders: []OpenOrder{{Side: wire.Buy, Px: 100}, {Side: wire.Sell, Px: 110}}}
	bids, asks := m.ownOrderLevels()
	if !bids[100] || !asks[110] || len(bids) != 1 || len(asks) != 1 {
		t.Fatalf("ownOrderLevels: bids=%v asks=%v", bids, asks)
	}
}

func TestLevelMarker(t *testing.T) {
	own := map[int64]bool{100: true}
	if !strings.Contains(levelMarker(100, own, 105, true), "▸") {
		t.Fatal("own-order marker missing (should win over last-trade)")
	}
	if !strings.Contains(levelMarker(105, map[int64]bool{}, 105, true), "‹") {
		t.Fatal("last-trade marker missing")
	}
	if levelMarker(1, map[int64]bool{}, 2, true) != " " {
		t.Fatal("unmarked level should be a single space")
	}
}

func TestViewOpenOrders(t *testing.T) {
	m := Model{openOrders: []OpenOrder{{Side: wire.Buy, Px: 9999, Qty: 15}}}
	out := m.viewOpenOrders()
	if !strings.Contains(out, "BUY") || !strings.Contains(out, "9999") || !strings.Contains(out, "15") {
		t.Fatalf("open-orders panel missing content:\n%s", out)
	}
}
