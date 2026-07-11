package book

import (
	"testing"
	"time"

	"rsx-term/wire"
)

const binNs = int64(100 * time.Millisecond)

// ingestBins seals n consecutive live bins of the same book/trades.
func ingestBins(h *Heatmap, n int, bids, asks []wire.Level, trades []TapeEntry) {
	for i := 0; i < n; i++ {
		from := int64(i) * binNs
		h.Ingest(bids, asks, trades, nil, from, from+binNs)
		trades = nil // trades print once, not every bin
	}
}

func TestFarSpanSchedule(t *testing.T) {
	want := []time.Duration{
		10 * time.Second, 60 * time.Second, 120 * time.Second,
		300 * time.Second, 600 * time.Second, time.Hour, time.Hour,
	}
	for i, span := range want {
		if got := FarSpan(i); got != span {
			t.Fatalf("FarSpan(%d) = %v, want %v", i, got, span)
		}
	}
}

func TestFisheyeOffsetNearTouchOneTickPerCell(t *testing.T) {
	for tk := int64(1); tk <= linearTicks; tk++ {
		if off := fisheyeOffset(tk); off != int(tk) {
			t.Fatalf("tick %d: offset %d, want %d (linear zone)", tk, off, tk)
		}
	}
	// Beyond the linear zone the fisheye compresses: distinct deep ticks share
	// a column.
	a := fisheyeOffset(linearTicks + 2)
	b := fisheyeOffset(linearTicks + 3)
	if a != b {
		t.Fatalf("deep ticks should aggregate: off(%d)=%d, off(%d)=%d",
			linearTicks+2, a, linearTicks+3, b)
	}
	if fisheyeOffset(linearTicks+1) <= linearTicks {
		t.Fatalf("first compressed column must sit past the linear zone")
	}
}

func TestCompressTicksTriangular(t *testing.T) {
	cases := map[int64]int{1: 1, 2: 2, 3: 2, 4: 3, 6: 3, 7: 4}
	for extra, want := range cases {
		if got := compressTicks(extra); got != want {
			t.Fatalf("compressTicks(%d)=%d, want %d", extra, got, want)
		}
	}
}

func TestFisheyeColBidsLeftAsksRight(t *testing.T) {
	const width = 20
	anchor := int64(20000) // mid 10000
	half := width / 2
	if col, ok := FisheyeCol(9999, anchor, 1, width); !ok || col != half-1 {
		t.Fatalf("best bid at touch col %d, got %d (%v)", half-1, col, ok)
	}
	if col, ok := FisheyeCol(10001, anchor, 1, width); !ok || col != half {
		t.Fatalf("best ask at touch col %d, got %d (%v)", half, col, ok)
	}
	if _, ok := FisheyeCol(10000, anchor, 1, width); ok {
		t.Fatalf("the exact mid is the spread gap, no column")
	}
	// Any bid maps left of centre, any ask right — the whole book clamps onto
	// the edge columns rather than falling off.
	for px := int64(1); px < 10000; px += 997 {
		col, ok := FisheyeCol(px, anchor, 1, width)
		if !ok || col >= half {
			t.Fatalf("bid %d landed at col %d (%v)", px, col, ok)
		}
	}
	for px := int64(10001); px < 40000; px += 997 {
		col, ok := FisheyeCol(px, anchor, 1, width)
		if !ok || col < half || col > width-1 {
			t.Fatalf("ask %d landed at col %d (%v)", px, col, ok)
		}
	}
}

func TestFisheyePxRoundTrip(t *testing.T) {
	const width = 40
	// Both anchor parities: an odd anchor (bid+ask odd — the common 1-tick-spread
	// case) previously dropped a half-tick on the bid side, breaking the inverse.
	for _, anchor := range []int64{20000, 20001} {
		for col := 0; col < width; col++ {
			px := FisheyePx(col, anchor, 1, width)
			back, ok := FisheyeCol(px, anchor, 1, width)
			if !ok || back != col {
				t.Fatalf("anchor %d: col %d → px %d → col %d (%v)", anchor, col, px, back, ok)
			}
		}
	}
	// Regression: the best bid at an odd anchor must recover exactly, not one
	// tick low (anchor 20001 ⇒ bid 10000 / ask 10001).
	col, _ := FisheyeCol(10000, 20001, 1, width)
	if got := FisheyePx(col, 20001, 1, width); got != 10000 {
		t.Fatalf("odd-anchor best bid: col %d → px %d, want 10000", col, got)
	}
}

func TestLiveFoldWholeBookWithAges(t *testing.T) {
	ages := NewPersistence()
	ages.ObserveDelta(wire.Delta{Px: 9999, Qty: 5}, 0)
	bids := []wire.Level{{Px: 9999, Qty: 5, Count: 1}, {Px: 9990, Qty: 30, Count: 4}}
	asks := []wire.Level{{Px: 10001, Qty: 6, Count: 2}}
	trades := []TapeEntry{{Side: wire.Sell, Px: 9999, Qty: 3}}
	row := LiveFold(bids, asks, trades, ages, 0, binNs)

	if len(row.Levels) != 3 {
		t.Fatalf("whole book folds: %d levels, want 3", len(row.Levels))
	}
	if row.Levels[0].Px != 9999 || row.Levels[0].AgeNs != binNs {
		t.Fatalf("tracked level should carry its age: %+v", row.Levels[0])
	}
	if row.Levels[1].AgeNs != 0 {
		t.Fatalf("untracked level age must be 0: %+v", row.Levels[1])
	}
	if len(row.Trades) != 1 || row.Trades[0].Qty != 3 || row.Trades[0].Side != wire.Sell {
		t.Fatalf("trade-flow not folded: %+v", row.Trades)
	}
}

func TestNowNeverScrollsOnlyHistoryDriftsUp(t *testing.T) {
	// The "now" row is LiveFold over the current book — built fresh every
	// render, independent of the ring. An idle feed still folds the same book
	// each bin, so the newest live row always shows the current state too.
	h := NewHeatmap(1)
	h.Configure(3, 0)
	bids := []wire.Level{{Px: 9999, Qty: 5, Count: 1}}
	asks := []wire.Level{{Px: 10001, Qty: 6, Count: 1}}
	ingestBins(h, 5, bids, asks, nil)
	rows := h.Rows()
	if len(rows) != 3 {
		t.Fatalf("live ring holds %d, want 3", len(rows))
	}
	newest := rows[len(rows)-1]
	if newest.FromNs != 4*binNs {
		t.Fatalf("newest bin from %d, want %d", newest.FromNs, 4*binNs)
	}
	if len(newest.Levels) != 2 {
		t.Fatalf("newest row must carry the live book")
	}
}

func TestCascadeSealsTenSecondWindow(t *testing.T) {
	h := NewHeatmap(1)
	h.Configure(2, 2) // tiers: 10s then 60s
	bids := []wire.Level{{Px: 9999, Qty: 8, Count: 2}}
	asks := []wire.Level{{Px: 10001, Qty: 4, Count: 1}}
	// 102 bins of 100ms: 100 expire into tier 0 (covering exactly 10s), which
	// seals into tier 1 and restarts.
	ingestBins(h, 102, bids, asks, nil)

	rows := h.Rows()
	if len(rows) != 4 {
		t.Fatalf("rows = %d, want 2 far + 2 live", len(rows))
	}
	sixty := rows[0] // farthest first
	if sixty.Span != 60*time.Second {
		t.Fatalf("top row span %v, want 60s", sixty.Span)
	}
	if len(sixty.Levels) != 2 {
		t.Fatalf("sealed 10s window should have promoted into the 60s tier: %+v", sixty.Levels)
	}
	// The steady book time-weight-averages to itself.
	for _, l := range sixty.Levels {
		switch l.Px {
		case 9999:
			if l.Size != 8 || l.Side != -1 {
				t.Fatalf("bid profile drifted: %+v", l)
			}
		case 10001:
			if l.Size != 4 || l.Side != 1 {
				t.Fatalf("ask profile drifted: %+v", l)
			}
		default:
			t.Fatalf("unexpected px %d", l.Px)
		}
	}
	ten := rows[1]
	if ten.Span != 10*time.Second {
		t.Fatalf("second row span %v, want 10s", ten.Span)
	}
}

func TestFarRowTimeWeightedAverage(t *testing.T) {
	h := NewHeatmap(1)
	h.Configure(1, 1)
	big := []wire.Level{{Px: 9999, Qty: 100, Count: 10}}
	small := []wire.Level{{Px: 9999, Qty: 20, Count: 2}}
	asks := []wire.Level{{Px: 10001, Qty: 1, Count: 1}}
	// The newest bin stays live (cap 1); the 16 EXPIRED bins are 4 of size 100
	// and 12 of size 20 → time-weighted mean 40.
	for i := 0; i < 4; i++ {
		from := int64(i) * binNs
		h.Ingest(big, asks, nil, nil, from, from+binNs)
	}
	for i := 4; i < 17; i++ {
		from := int64(i) * binNs
		h.Ingest(small, asks, nil, nil, from, from+binNs)
	}
	rows := h.Rows()
	far := rows[0]
	var got *LevelSample
	for i := range far.Levels {
		if far.Levels[i].Px == 9999 {
			got = &far.Levels[i]
		}
	}
	if got == nil {
		t.Fatalf("far row lost the level: %+v", far.Levels)
	}
	if got.Size != 40 {
		t.Fatalf("time-weighted mean size = %d, want 40", got.Size)
	}
	if got.Count != 4 {
		t.Fatalf("time-weighted mean count = %d, want 4", got.Count)
	}
}

func TestFarRowSumsTradeFlow(t *testing.T) {
	h := NewHeatmap(1)
	h.Configure(1, 1)
	bids := []wire.Level{{Px: 9999, Qty: 5, Count: 1}}
	asks := []wire.Level{{Px: 10001, Qty: 5, Count: 1}}
	h.Ingest(bids, asks, []TapeEntry{{Side: wire.Buy, Px: 10001, Qty: 3}}, nil, 0, binNs)
	h.Ingest(bids, asks, []TapeEntry{{Side: wire.Buy, Px: 10001, Qty: 4}}, nil, binNs, 2*binNs)
	h.Ingest(bids, asks, nil, nil, 2*binNs, 3*binNs)

	far := h.Rows()[0]
	if len(far.Trades) != 1 {
		t.Fatalf("far trades = %+v, want one coalesced buy", far.Trades)
	}
	tr := far.Trades[0]
	if tr.Px != 10001 || tr.Qty != 7 || tr.Side != wire.Buy {
		t.Fatalf("coalesced trade = %+v, want 7 @ 10001 buy", tr)
	}
}

func TestConfigurePreservesLiveTrimsToCap(t *testing.T) {
	h := NewHeatmap(1)
	h.Configure(5, 0)
	bids := []wire.Level{{Px: 9999, Qty: 5}}
	asks := []wire.Level{{Px: 10001, Qty: 5}}
	ingestBins(h, 5, bids, asks, nil)
	h.Configure(2, 3) // shrink live, add tiers — a resize, history kept
	if h.LiveLen() != 2 {
		t.Fatalf("live trimmed to %d, want 2", h.LiveLen())
	}
	if h.FarRows() != 3 {
		t.Fatalf("far rows %d, want 3", h.FarRows())
	}
	if h.MidPx() != 10000 {
		t.Fatalf("anchor lost on resize: mid %d", h.MidPx())
	}
}

func TestRecenterHysteresis(t *testing.T) {
	h := NewHeatmap(1)
	h.Configure(2, 0)
	h.Ingest([]wire.Level{{Px: 100, Qty: 1}}, []wire.Level{{Px: 102, Qty: 1}}, nil, nil, 0, binNs)
	if h.MidPx() != 101 {
		t.Fatalf("initial mid = %d, want 101", h.MidPx())
	}
	// Small drift (+2 ticks) stays inside the band: anchor holds.
	h.Ingest([]wire.Level{{Px: 102, Qty: 1}}, []wire.Level{{Px: 104, Qty: 1}}, nil, nil, binNs, 2*binNs)
	if h.MidPx() != 101 {
		t.Fatalf("small drift must not re-anchor: mid %d", h.MidPx())
	}
	// Large drift (+40 ticks) clears the band: anchor moves.
	h.Ingest([]wire.Level{{Px: 140, Qty: 1}}, []wire.Level{{Px: 142, Qty: 1}}, nil, nil, 2*binNs, 3*binNs)
	if h.MidPx() != 141 {
		t.Fatalf("large drift should re-anchor: mid %d", h.MidPx())
	}
}

func TestPersistenceProxyLifecycle(t *testing.T) {
	p := NewPersistence()
	sec := int64(time.Second)
	p.ObserveSnapshot([]wire.Level{{Px: 9999, Qty: 5}}, []wire.Level{{Px: 10001, Qty: 3}}, 0)
	if got := p.AgeNs(9999, 30*sec); got != 30*sec {
		t.Fatalf("standing level age = %d, want 30s", got)
	}
	// A size change keeps the clock (the level is still held).
	p.ObserveDelta(wire.Delta{Px: 9999, Qty: 9}, 40*sec)
	if got := p.AgeNs(9999, 60*sec); got != 60*sec {
		t.Fatalf("size change must not reset the clock: %d", got)
	}
	// Emptying resets; reappearing restarts.
	p.ObserveDelta(wire.Delta{Px: 9999, Qty: 0}, 70*sec)
	if got := p.AgeNs(9999, 80*sec); got != 0 {
		t.Fatalf("removed level must read fresh: %d", got)
	}
	p.ObserveDelta(wire.Delta{Px: 9999, Qty: 2}, 90*sec)
	if got := p.AgeNs(9999, 95*sec); got != 5*sec {
		t.Fatalf("restarted clock = %d, want 5s", got)
	}
	// A snapshot missing a tracked level resets it too.
	p.ObserveSnapshot(nil, []wire.Level{{Px: 10001, Qty: 3}}, 100*sec)
	if got := p.AgeNs(9999, 110*sec); got != 0 {
		t.Fatalf("snapshot drop must reset: %d", got)
	}
	if got := p.AgeNs(10001, 110*sec); got != 110*sec {
		t.Fatalf("surviving level keeps its clock: %d", got)
	}
}
