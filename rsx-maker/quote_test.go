package main

import "testing"

func TestQuoteTopLevel(t *testing.T) {
	// ref=50000, 20 bps -> half = 50000*20/10000 = 100.
	// level 0: bid=ref-100, ask=ref+100, tick=1.
	bid, ask := quote(50000, 20, 1, 0)
	if bid != 49900 {
		t.Errorf("bid = %d, want 49900", bid)
	}
	if ask != 50100 {
		t.Errorf("ask = %d, want 50100", ask)
	}
}

func TestQuoteDeeperLevelsWiden(t *testing.T) {
	// step = half/2 = 50. level 1 offset = 100 + 50 = 150.
	bid, ask := quote(50000, 20, 1, 1)
	if bid != 49850 {
		t.Errorf("bid = %d, want 49850", bid)
	}
	if ask != 50150 {
		t.Errorf("ask = %d, want 50150", ask)
	}
}

func TestQuoteTickAlignment(t *testing.T) {
	// tick=10: bid floors, ask ceils to the tick boundary.
	// ref=50005, 10 bps -> half = 50005*10/10000 = 50.
	// raw bid = 49955 -> floor to 49950; raw ask = 50055 -> ceil to 50060.
	bid, ask := quote(50005, 10, 10, 0)
	if bid%10 != 0 || ask%10 != 0 {
		t.Fatalf("prices not tick-aligned: bid=%d ask=%d", bid, ask)
	}
	if bid != 49950 {
		t.Errorf("bid = %d, want 49950", bid)
	}
	if ask != 50060 {
		t.Errorf("ask = %d, want 50060", ask)
	}
}

func TestHalfSpreadFloorsAtOne(t *testing.T) {
	// Tiny ref * bps rounds to 0; half-spread must still be >= 1 so
	// the two sides never collapse onto ref.
	if got := halfSpread(100, 1); got != 1 {
		t.Errorf("halfSpread(100,1) = %d, want 1", got)
	}
	if got := halfSpread(1000000, 20); got != 2000 {
		t.Errorf("halfSpread(1e6,20) = %d, want 2000", got)
	}
}

func TestQuoteSpreadWidensWithBps(t *testing.T) {
	narrowBid, narrowAsk := quote(50000, 10, 1, 0)
	wideBid, wideAsk := quote(50000, 40, 1, 0)
	if (narrowAsk - narrowBid) >= (wideAsk - wideBid) {
		t.Errorf("wider bps must widen spread: narrow=%d wide=%d",
			narrowAsk-narrowBid, wideAsk-wideBid)
	}
}

func TestOrderQtyAlignsToLot(t *testing.T) {
	// 10 lots of size 5 = 50 raw units, already lot-aligned.
	if got := orderQty(10, 5); got != 50 {
		t.Errorf("orderQty(10,5) = %d, want 50", got)
	}
	// At least one lot even when qtyPerLevel is 0.
	if got := orderQty(0, 7); got != 7 {
		t.Errorf("orderQty(0,7) = %d, want 7", got)
	}
}
