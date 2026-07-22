package glass

import (
	"testing"

	"rsx-core/feed"
	"rsx-core/wire"
)

// The feed adapter's unit contract: folding the decoded wire.* events a
// connector delivers into rsx-term's reused model produces a valid Scene — the
// same seam the live path drives, exercised here without a socket.

const feedW, feedH = 72, 24

func TestFeedFoldsEventsToScene(t *testing.T) {
	f := NewFeed("BTC-PERP", 10, 1, feedH)
	f.Apply(feed.MdUp{}, 0)

	// A snapshot, then a few bins with deltas and trades — the shape the live
	// path folds frame by frame.
	f.Apply(wire.Snapshot{
		SymbolID: 10,
		Bids:     []wire.Level{{Px: 9999, Qty: 50, Count: 1}, {Px: 9997, Qty: 900, Count: 3}},
		Asks:     []wire.Level{{Px: 10001, Qty: 60, Count: 1}, {Px: 10003, Qty: 850, Count: 3}},
	}, 1_000_000_000)
	for i := int64(1); i <= 12; i++ {
		ns := 1_000_000_000 + i*100_000_000
		f.Apply(wire.MdTrade{SymbolID: 10, Px: 10001, Qty: 40, TakerSide: 0}, ns)
		f.Apply(wire.MdTrade{SymbolID: 10, Px: 9999, Qty: 30, TakerSide: 1}, ns)
		f.Bin(ns)
	}

	sc := f.Scene()
	if !sc.Connected {
		t.Fatal("feed should be connected after MdUp")
	}
	if sc.Heat.MidPx() <= 0 {
		t.Fatal("heatmap never anchored — the snapshot did not fold")
	}

	g := NewGrid(feedW, feedH)
	Compose(g, sc)
	if hasBg(g, Live) == 0 {
		t.Fatal("no size-ramp cells rendered — the folded book is empty")
	}
	if !hasGlyph(g, '9') {
		t.Fatal("touch ladder numbers missing — the book's exact levels did not reach the footer")
	}
	if !hasGlyph(g, 'R') || !hasGlyph(g, 'X') {
		t.Fatal("header badge missing")
	}
}

// TestFeedDeltaRemoval proves the fold reuses book.ApplyDelta correctly: a
// qty==0 delta removes the level from the ladder the Scene exposes.
func TestFeedDeltaRemoval(t *testing.T) {
	f := NewFeed("BTC", 10, 1, feedH)
	f.Apply(wire.Snapshot{
		SymbolID: 10,
		Bids:     []wire.Level{{Px: 100, Qty: 5, Count: 1}, {Px: 99, Qty: 9, Count: 1}},
		Asks:     []wire.Level{{Px: 101, Qty: 7, Count: 1}},
	}, 1)
	if got := len(f.Scene().Bids); got != 2 {
		t.Fatalf("bids before removal = %d, want 2", got)
	}
	f.Apply(wire.Delta{SymbolID: 10, Side: 0, Px: 99, Qty: 0}, 2) // remove the 99 bid
	bids := f.Scene().Bids
	if len(bids) != 1 || bids[0].Px != 100 {
		t.Fatalf("after removing px 99, bids = %v, want just px 100", bids)
	}
}

// TestFeedLinkState checks the link transitions the header's link dot reads:
// MdDown drops connected, MdUp restores it.
func TestFeedLinkState(t *testing.T) {
	f := NewFeed("BTC", 10, 1, feedH)
	f.Apply(feed.MdUp{}, 0)
	if !f.Scene().Connected {
		t.Fatal("MdUp should connect")
	}
	f.Apply(feed.MdDown{}, 1)
	if f.Scene().Connected {
		t.Fatal("MdDown should disconnect")
	}
}

// TestFeedIgnoresForeignSymbol confirms a frame for another symbol never folds
// into this feed's book (the live feed may carry peers).
func TestFeedIgnoresForeignSymbol(t *testing.T) {
	f := NewFeed("BTC", 10, 1, feedH)
	f.Apply(wire.Snapshot{SymbolID: 3, Bids: []wire.Level{{Px: 1, Qty: 1, Count: 1}}}, 1)
	sc := f.Scene()
	if len(sc.Bids) != 0 || len(sc.Asks) != 0 {
		t.Fatal("a foreign-symbol snapshot must not fold into this feed")
	}
}
