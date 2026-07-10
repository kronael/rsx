package book

import (
	"testing"

	"rsx-term/wire"
)

func TestFisheyeOffsetNearTouchOneTickPerCell(t *testing.T) {
	// The first linearTicks ticks are one-to-one, so adjacent ticks near the
	// touch land in adjacent columns.
	for tk := int64(1); tk <= linearTicks; tk++ {
		if off := fisheyeOffset(tk); off != int(tk) {
			t.Fatalf("tick %d: offset %d, want %d (linear zone)", tk, off, tk)
		}
	}
	// Beyond the linear zone the fisheye compresses: successive columns absorb
	// wider tick ranges, so distinct deep ticks share a column.
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

func TestIngestBidsLeftAsksRight(t *testing.T) {
	h := NewHeatmap(20, 4, 1) // half = 10
	bids := []wire.Level{{Px: 9999, Qty: 5, Count: 1}, {Px: 9998, Qty: 7, Count: 2}}
	asks := []wire.Level{{Px: 10001, Qty: 6, Count: 1}, {Px: 10002, Qty: 8, Count: 3}}
	h.Ingest(bids, asks, nil, 100)

	row := h.Rows()[len(h.Rows())-1]
	half := h.Width() / 2
	// Best bid at the touch (col half-1), best ask at the touch (col half).
	if row.Cells[half-1].Side != -1 || row.Cells[half-1].Size != 5 {
		t.Fatalf("best bid should rest at col %d: %+v", half-1, row.Cells[half-1])
	}
	if row.Cells[half].Side != 1 || row.Cells[half].Size != 6 {
		t.Fatalf("best ask should rest at col %d: %+v", half, row.Cells[half])
	}
	// Every populated bid column is left of centre, every ask right.
	for i, c := range row.Cells {
		if c.Side == -1 && i >= half {
			t.Fatalf("bid cell on the ask side at col %d", i)
		}
		if c.Side == 1 && i < half {
			t.Fatalf("ask cell on the bid side at col %d", i)
		}
	}
}

func TestIngestAggregatesDeepLevels(t *testing.T) {
	h := NewHeatmap(24, 4, 1) // half = 12, mid anchored at 10000
	bids := []wire.Level{{Px: 9999, Qty: 1, Count: 1}}
	// A near-touch ask fixes the anchor; two deep asks 10 and 11 ticks above the
	// mid fall in the compressed zone and share one fisheye column, so their
	// sizes sum and their order counts add.
	deep1, deep2 := int64(10010), int64(10011)
	asks := []wire.Level{
		{Px: 10001, Qty: 1, Count: 1},
		{Px: deep1, Qty: 4, Count: 2},
		{Px: deep2, Qty: 6, Count: 5},
	}
	h.Ingest(bids, asks, nil, 1)

	col1, ok1 := h.colFor(deep1)
	col2, ok2 := h.colFor(deep2)
	if !ok1 || !ok2 || col1 != col2 {
		t.Fatalf("deep levels expected in one bucket: %d(%v) vs %d(%v)", col1, ok1, col2, ok2)
	}
	c := h.Rows()[0].Cells[col1]
	if c.Size != 10 || c.Count != 7 {
		t.Fatalf("aggregated bucket = size %d count %d, want 10 / 7", c.Size, c.Count)
	}
}

func TestRingNewestAtBottomAndAges(t *testing.T) {
	h := NewHeatmap(10, 3, 1)
	for i := 0; i < 5; i++ {
		bid := int64(9999 - i)
		ask := int64(10001 - i)
		h.Ingest([]wire.Level{{Px: bid, Qty: 1}}, []wire.Level{{Px: ask, Qty: 1}}, nil, int64(i))
	}
	rows := h.Rows()
	if len(rows) != 3 {
		t.Fatalf("ring len %d, want height 3", len(rows))
	}
	// Oldest retained is bin 2, newest (bottom) is bin 4.
	if rows[0].BinTs != 2 {
		t.Fatalf("oldest retained bin = %d, want 2", rows[0].BinTs)
	}
	if rows[len(rows)-1].BinTs != 4 {
		t.Fatalf("newest bin = %d, want 4", rows[len(rows)-1].BinTs)
	}
}

func TestRecenterHysteresis(t *testing.T) {
	h := NewHeatmap(20, 2, 1)
	h.Ingest([]wire.Level{{Px: 100}}, []wire.Level{{Px: 102}}, nil, 0)
	if h.MidPx() != 101 {
		t.Fatalf("initial mid = %d, want 101", h.MidPx())
	}
	// Small drift (mid 103, +2 ticks) stays inside the band: anchor holds.
	h.Ingest([]wire.Level{{Px: 102}}, []wire.Level{{Px: 104}}, nil, 1)
	if h.MidPx() != 101 {
		t.Fatalf("small drift must not re-anchor: mid %d", h.MidPx())
	}
	// Large drift (mid 141, +40 ticks) clears the band: anchor moves.
	h.Ingest([]wire.Level{{Px: 140}}, []wire.Level{{Px: 142}}, nil, 2)
	if h.MidPx() != 141 {
		t.Fatalf("large drift should re-anchor: mid %d", h.MidPx())
	}
}

func TestIngestTradesRecorded(t *testing.T) {
	h := NewHeatmap(20, 4, 1)
	trades := []TapeEntry{
		{Side: wire.Buy, Px: 10001, Qty: 3},
		{Side: wire.Sell, Px: 9999, Qty: 5},
	}
	h.Ingest([]wire.Level{{Px: 9999, Qty: 1}}, []wire.Level{{Px: 10001, Qty: 1}}, trades, 7)
	row := h.Rows()[0]

	buyCol, _ := h.colFor(10001)
	if row.Cells[buyCol].BuyTrade != 3 {
		t.Fatalf("buy trade not recorded at col %d: %+v", buyCol, row.Cells[buyCol])
	}
	sellCol, _ := h.colFor(9999)
	if row.Cells[sellCol].SellTrade != 5 {
		t.Fatalf("sell trade not recorded at col %d: %+v", sellCol, row.Cells[sellCol])
	}
}
