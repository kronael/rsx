package book

import (
	"testing"

	"rsx-term/wire"
)

func TestApplySnapshotReplacesPriorState(t *testing.T) {
	var b Book
	b.ApplySnapshot(wire.Snapshot{
		Bids: []wire.Level{{Px: 100, Qty: 1}},
		Asks: []wire.Level{{Px: 101, Qty: 1}},
	})
	b.ApplySnapshot(wire.Snapshot{
		Bids: []wire.Level{{Px: 200, Qty: 2}},
		Asks: []wire.Level{{Px: 201, Qty: 2}},
	})
	if len(b.Bids) != 1 || b.Bids[0].Px != 200 {
		t.Fatalf("bids not replaced: %+v", b.Bids)
	}
	if len(b.Asks) != 1 || b.Asks[0].Px != 201 {
		t.Fatalf("asks not replaced: %+v", b.Asks)
	}
}

func TestApplyDeltaInsertsBidDescending(t *testing.T) {
	var b Book
	b.ApplySnapshot(wire.Snapshot{Bids: []wire.Level{{Px: 100, Qty: 1}, {Px: 90, Qty: 1}}})
	b.ApplyDelta(wire.Delta{Side: 0, Px: 95, Qty: 5, Count: 1})
	want := []int64{100, 95, 90}
	if len(b.Bids) != 3 {
		t.Fatalf("len = %d, want 3: %+v", len(b.Bids), b.Bids)
	}
	for i, px := range want {
		if b.Bids[i].Px != px {
			t.Fatalf("bids[%d].Px = %d, want %d (%+v)", i, b.Bids[i].Px, px, b.Bids)
		}
	}
}

func TestApplyDeltaInsertsAskAscending(t *testing.T) {
	var b Book
	b.ApplySnapshot(wire.Snapshot{Asks: []wire.Level{{Px: 100, Qty: 1}, {Px: 110, Qty: 1}}})
	b.ApplyDelta(wire.Delta{Side: 1, Px: 105, Qty: 5, Count: 1})
	want := []int64{100, 105, 110}
	if len(b.Asks) != 3 {
		t.Fatalf("len = %d, want 3: %+v", len(b.Asks), b.Asks)
	}
	for i, px := range want {
		if b.Asks[i].Px != px {
			t.Fatalf("asks[%d].Px = %d, want %d (%+v)", i, b.Asks[i].Px, px, b.Asks)
		}
	}
}

func TestApplyDeltaUpdatesInPlace(t *testing.T) {
	var b Book
	b.ApplySnapshot(wire.Snapshot{Bids: []wire.Level{{Px: 100, Qty: 1}, {Px: 90, Qty: 1}}})
	b.ApplyDelta(wire.Delta{Side: 0, Px: 100, Qty: 7, Count: 3})
	if len(b.Bids) != 2 {
		t.Fatalf("len changed: %+v", b.Bids)
	}
	if b.Bids[0].Px != 100 || b.Bids[0].Qty != 7 || b.Bids[0].Count != 3 {
		t.Fatalf("level not updated in place: %+v", b.Bids[0])
	}
	if b.Bids[1].Px != 90 {
		t.Fatalf("sort order disturbed: %+v", b.Bids)
	}
}

func TestApplyDeltaRemovesLevel(t *testing.T) {
	var b Book
	b.ApplySnapshot(wire.Snapshot{Bids: []wire.Level{{Px: 100, Qty: 1}, {Px: 90, Qty: 1}}})
	b.ApplyDelta(wire.Delta{Side: 0, Px: 100, Qty: 0})
	if len(b.Bids) != 1 || b.Bids[0].Px != 90 {
		t.Fatalf("level not removed: %+v", b.Bids)
	}
}

func TestApplyDeltaRemoveAbsentPxNoop(t *testing.T) {
	var b Book
	b.ApplySnapshot(wire.Snapshot{Bids: []wire.Level{{Px: 100, Qty: 1}}})
	b.ApplyDelta(wire.Delta{Side: 0, Px: 50, Qty: 0})
	if len(b.Bids) != 1 || b.Bids[0].Px != 100 {
		t.Fatalf("book mutated on no-op remove: %+v", b.Bids)
	}
}

func TestApplyDeltaUnknownSideIgnored(t *testing.T) {
	var b Book
	b.ApplySnapshot(wire.Snapshot{
		Bids: []wire.Level{{Px: 100, Qty: 1}},
		Asks: []wire.Level{{Px: 101, Qty: 1}},
	})
	b.ApplyDelta(wire.Delta{Side: 2, Px: 999, Qty: 5})
	if len(b.Bids) != 1 || len(b.Asks) != 1 {
		t.Fatalf("book mutated on unknown side: %+v %+v", b.Bids, b.Asks)
	}
}

func TestSpread(t *testing.T) {
	var b Book
	b.ApplySnapshot(wire.Snapshot{
		Bids: []wire.Level{{Px: 100, Qty: 1}},
		Asks: []wire.Level{{Px: 105, Qty: 1}},
	})
	if got := b.Spread(); got != 5 {
		t.Fatalf("Spread() = %d, want 5", got)
	}

	var noBids Book
	noBids.ApplySnapshot(wire.Snapshot{Asks: []wire.Level{{Px: 105, Qty: 1}}})
	if got := noBids.Spread(); got != 0 {
		t.Fatalf("Spread() with no bids = %d, want 0", got)
	}

	var noAsks Book
	noAsks.ApplySnapshot(wire.Snapshot{Bids: []wire.Level{{Px: 100, Qty: 1}}})
	if got := noAsks.Spread(); got != 0 {
		t.Fatalf("Spread() with no asks = %d, want 0", got)
	}
}

func TestMidFromLadder(t *testing.T) {
	var b Book
	b.ApplySnapshot(wire.Snapshot{
		Bids: []wire.Level{{Px: 100, Qty: 1}},
		Asks: []wire.Level{{Px: 110, Qty: 1}},
	})
	got, ok := b.Mid()
	if !ok || got != 105 {
		t.Fatalf("Mid() = %d, %v; want 105, true", got, ok)
	}
}

func TestMidFallsBackToBbo(t *testing.T) {
	var b Book
	b.ApplySnapshot(wire.Snapshot{Bids: []wire.Level{{Px: 100, Qty: 1}}})
	b.ApplyBbo(wire.Bbo{BidPx: 100, AskPx: 110})
	got, ok := b.Mid()
	if !ok || got != 105 {
		t.Fatalf("Mid() = %d, %v; want 105, true", got, ok)
	}
}

func TestMidFalseWhenIncomplete(t *testing.T) {
	var noBbo Book
	noBbo.ApplySnapshot(wire.Snapshot{Bids: []wire.Level{{Px: 100, Qty: 1}}})
	if _, ok := noBbo.Mid(); ok {
		t.Fatalf("Mid() ok with no BBO and one-sided ladder")
	}

	var zeroSideBbo Book
	zeroSideBbo.ApplySnapshot(wire.Snapshot{Bids: []wire.Level{{Px: 100, Qty: 1}}})
	zeroSideBbo.ApplyBbo(wire.Bbo{BidPx: 100, AskPx: 0})
	if _, ok := zeroSideBbo.Mid(); ok {
		t.Fatalf("Mid() ok with zero-side BBO")
	}

	var empty Book
	if _, ok := empty.Mid(); ok {
		t.Fatalf("Mid() ok on fully empty book")
	}
}

func TestEmpty(t *testing.T) {
	var b Book
	if !b.Empty() {
		t.Fatalf("Empty() = false initially")
	}
	b.ApplySnapshot(wire.Snapshot{Bids: []wire.Level{{Px: 100, Qty: 1}}})
	if b.Empty() {
		t.Fatalf("Empty() = true after adding a level")
	}
	b.ApplyDelta(wire.Delta{Side: 0, Px: 100, Qty: 0})
	if !b.Empty() {
		t.Fatalf("Empty() = false after removing all levels")
	}
}
