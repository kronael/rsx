package main

import (
	"math"
	"testing"
)

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

func TestDepthQtyDefaultIsUniform(t *testing.T) {
	// Empty profile must be byte-for-byte the orderQty ladder.
	for level := 0; level < 8; level++ {
		if got, want := depthQty("", 10, 5, level), orderQty(10, 5); got != want {
			t.Errorf("depthQty(\"\",10,5,%d) = %d, want %d", level, got, want)
		}
	}
}

func TestDepthQtyWallsRampMonotonic(t *testing.T) {
	// walls: thin touch, deep walls — strictly non-decreasing with depth,
	// clamped to the last multiplier past the ramp end.
	prev := int64(0)
	for level := 0; level < len(wallRamp); level++ {
		q := depthQty("walls", 10, 5, level)
		if q < prev {
			t.Errorf("walls qty shrank at level %d: %d < %d", level, q, prev)
		}
		prev = q
	}
	if got, want := depthQty("walls", 10, 5, 99), depthQty("walls", 10, 5, len(wallRamp)-1); got != want {
		t.Errorf("past-ramp level = %d, want clamp to deepest %d", got, want)
	}
	// Touch is thinner than base, the wall is several times deeper.
	base := orderQty(10, 5)
	if touch := depthQty("walls", 10, 5, 0); touch >= base {
		t.Errorf("walls touch %d not thinner than base %d", touch, base)
	}
	if wall := depthQty("walls", 10, 5, len(wallRamp)-1); wall < 4*base {
		t.Errorf("walls deep level %d not a wall vs base %d", wall, base)
	}
}

func TestDepthQtyWallsLotAlignedAndNonZero(t *testing.T) {
	for level := 0; level < 6; level++ {
		q := depthQty("walls", 1, 7, level)
		if q < 7 {
			t.Errorf("level %d qty %d below one lot", level, q)
		}
		if q%7 != 0 {
			t.Errorf("level %d qty %d not lot-aligned", level, q)
		}
	}
}

func TestMinQtyRaisesToTheMinimum(t *testing.T) {
	// min 1e10 raw at price 6_000_000: ceil(1e10 / 6e6) = 1667 raw units,
	// rounded up to the next multiple of lot 100 = 1700.
	spec := symbolSpec{tick: 1, lot: 100, minNotional: 10_000_000_000}
	got := minQty(100, 6_000_000, spec)
	if got != 1700 {
		t.Fatalf("qty = %d, want 1700", got)
	}
	if got*6_000_000 < spec.minNotional {
		t.Fatalf("notional %d below minimum %d", got*6_000_000, spec.minNotional)
	}
	if prior := got - spec.lot; prior*6_000_000 >= spec.minNotional {
		t.Fatalf("qty %d is not the smallest lot multiple", got)
	}
}

func TestMinQtyKeepsAQtyAlreadyAtTheMinimum(t *testing.T) {
	spec := symbolSpec{tick: 1, lot: 10, minNotional: 1000}
	// price 5, qty 200: notional exactly 1000.
	if got := minQty(200, 5, spec); got != 200 {
		t.Fatalf("qty = %d, want 200", got)
	}
	// one lot less is raised back to exactly the minimum.
	if got := minQty(190, 5, spec); got != 200 {
		t.Fatalf("qty = %d, want 200", got)
	}
}

func TestMinQtyPerLimitPrice(t *testing.T) {
	// A lower bid price needs more qty than the ask for the same floor.
	spec := symbolSpec{tick: 1, lot: 100, minNotional: 1_000_000}
	bid := minQty(100, 900, spec)
	ask := minQty(100, 1100, spec)
	if bid <= ask {
		t.Fatalf("bid qty %d should exceed ask qty %d", bid, ask)
	}
	for _, c := range []struct{ px, qty int64 }{{900, bid}, {1100, ask}} {
		if c.px*c.qty < spec.minNotional || c.qty%spec.lot != 0 {
			t.Fatalf("px %d qty %d misses the floor or the lot", c.px, c.qty)
		}
	}
}

func TestMinQtyWithoutAFloorIsIdentity(t *testing.T) {
	if got := minQty(7, 100, symbolSpec{lot: 1}); got != 7 {
		t.Fatalf("qty = %d, want 7", got)
	}
}

func TestMinQtySaturatesInsteadOfOverflowing(t *testing.T) {
	spec := symbolSpec{lot: 3, minNotional: math.MaxInt64}
	got := minQty(3, 1, spec)
	if got <= 0 || got%3 != 0 {
		t.Fatalf("qty = %d, want a positive lot multiple", got)
	}
}

func TestCatalogMinNotionalPrefersTheCatalogValue(t *testing.T) {
	value := int64(2_000_000_000)
	if got := catalogMinNotional(&value, 2, 8); got != value {
		t.Fatalf("got %d, want %d", got, value)
	}
}

func TestCatalogMinNotionalFallsBackToOneUSD(t *testing.T) {
	// price 2 + qty 8 decimals: 1 USD = 10^10 raw.
	if got := catalogMinNotional(nil, 2, 8); got != 10_000_000_000 {
		t.Fatalf("got %d, want 1e10", got)
	}
	if got := catalogMinNotional(nil, 6, 4); got != 10_000_000_000 {
		t.Fatalf("got %d, want 1e10", got)
	}
	if got := catalogMinNotional(nil, 0, 0); got != 0 {
		t.Fatalf("got %d, want 0 when decimals are unknown", got)
	}
}

func TestJitterShiftKeepsTheTopOffThePreviousOppositeSide(t *testing.T) {
	base := topQuote{bid: 995, ask: 1005}
	prev := topQuote{bid: 995, ask: 1005}
	for _, tc := range []struct {
		name  string
		shift int64
		want  int64
	}{
		{"zero", 0, 0},
		{"up stops one tick under the old ask", 500, 9},
		{"down stops one tick over the old bid", -500, -9},
		{"small shift passes", 3, 3},
	} {
		if got := jitterShift(tc.shift, 1, base, prev); got != tc.want {
			t.Errorf("%s: shift %d, want %d", tc.name, got, tc.want)
		}
	}
}

func TestJitterShiftAlignsToTheTick(t *testing.T) {
	base := topQuote{bid: 1000, ask: 1100}
	prev := topQuote{bid: 1000, ask: 1100}
	got := jitterShift(10_000, 25, base, prev)
	if got != 75 || got%25 != 0 {
		t.Errorf("shift %d, want 75", got)
	}
}

func TestJitterShiftPullsALadderBackUnderThePreviousAsk(t *testing.T) {
	prev := topQuote{bid: 995, ask: 1005}
	above := topQuote{bid: 1100, ask: 1110}
	if got := jitterShift(0, 1, above, prev); got != -96 {
		t.Errorf("shift %d, want -96", got)
	}
}

func TestJitterShiftWithoutAPreviousLadderIsFree(t *testing.T) {
	base := topQuote{bid: 995, ask: 1005}
	if got := jitterShift(500, 1, base, topQuote{}); got != 500 {
		t.Errorf("shift %d, want 500", got)
	}
}
