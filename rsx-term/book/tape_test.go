package book

import (
	"testing"

	"rsx-term/wire"
)

func entry(px int64) TapeEntry {
	return TapeEntry{Side: wire.Buy, Px: px, Qty: 1}
}

func TestTapePushNewestFirst(t *testing.T) {
	var tp Tape
	tp.Push(entry(1))
	tp.Push(entry(2))
	tp.Push(entry(3))
	got := tp.Entries()
	want := []int64{3, 2, 1}
	if len(got) != 3 {
		t.Fatalf("len = %d, want 3", len(got))
	}
	for i, px := range want {
		if got[i].Px != px {
			t.Fatalf("Entries()[%d].Px = %d, want %d", i, got[i].Px, px)
		}
	}
}

func TestTapeCapsAt50(t *testing.T) {
	var tp Tape
	for i := 0; i < 60; i++ {
		tp.Push(entry(int64(i)))
	}
	got := tp.Entries()
	if len(got) != MaxTrades {
		t.Fatalf("len = %d, want %d", len(got), MaxTrades)
	}
	// Newest pushed was 59 (first), oldest surviving is entry #10
	// (entries 0-9 dropped).
	if got[0].Px != 59 {
		t.Fatalf("Entries()[0].Px = %d, want 59", got[0].Px)
	}
	if got[49].Px != 10 {
		t.Fatalf("Entries()[49].Px = %d, want 10", got[49].Px)
	}
}

func TestTapeLast(t *testing.T) {
	var tp Tape
	if _, ok := tp.Last(); ok {
		t.Fatalf("Last() on empty tape returned ok")
	}
	tp.Push(entry(1))
	tp.Push(entry(2))
	last, ok := tp.Last()
	if !ok || last.Px != 2 {
		t.Fatalf("Last() = %+v, %v; want Px=2, true", last, ok)
	}
}
