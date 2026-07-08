package book

import "testing"

func TestWindowCapsAt128(t *testing.T) {
	var w Window
	for i := 0; i < 200; i++ {
		w.Add(int64(i))
	}
	// Surviving window is values 72..199 (128 values), still ascending.
	min, ok := w.Min()
	if !ok || min != 72 {
		t.Fatalf("Min() = %d, %v; want 72, true", min, ok)
	}
	p50, ok := w.P50()
	if !ok || p50 != 136 {
		t.Fatalf("P50() = %d, %v; want 136, true", p50, ok)
	}
}

func TestWindowP50AndMinKnownSet(t *testing.T) {
	var w Window
	for i := int64(1); i <= 9; i++ {
		w.Add(i)
	}
	// sorted = [1..9], len=9, sorted[len/2] = sorted[4] = 5.
	p50, ok := w.P50()
	if !ok || p50 != 5 {
		t.Fatalf("P50() = %d, %v; want 5, true", p50, ok)
	}
	min, ok := w.Min()
	if !ok || min != 1 {
		t.Fatalf("Min() = %d, %v; want 1, true", min, ok)
	}
}

func TestWindowIgnoresNegative(t *testing.T) {
	var w Window
	w.Add(-1)
	w.Add(-100)
	if _, ok := w.P50(); ok {
		t.Fatalf("P50() ok after only negative Adds")
	}
	w.Add(5)
	p50, ok := w.P50()
	if !ok || p50 != 5 {
		t.Fatalf("P50() = %d, %v; want 5, true", p50, ok)
	}
}

func TestWindowEmpty(t *testing.T) {
	var w Window
	if _, ok := w.P50(); ok {
		t.Fatalf("P50() ok on empty window")
	}
	if _, ok := w.Min(); ok {
		t.Fatalf("Min() ok on empty window")
	}
	if _, ok := w.P99(); ok {
		t.Fatalf("P99() ok on empty window")
	}
	if got := w.Len(); got != 0 {
		t.Fatalf("Len() = %d, want 0", got)
	}
	if got := w.Recent(10); got != nil {
		t.Fatalf("Recent() = %v, want nil", got)
	}
}

func TestWindowP99KnownSet(t *testing.T) {
	var w Window
	for i := int64(1); i <= 100; i++ {
		w.Add(i)
	}
	// sorted = [1..100], len=100, idx = 100*99/100 = 99 -> sorted[99] = 100.
	p99, ok := w.P99()
	if !ok || p99 != 100 {
		t.Fatalf("P99() = %d, %v; want 100, true", p99, ok)
	}
}

func TestWindowP99SmallSetClampsToLastIndex(t *testing.T) {
	var w Window
	w.Add(1)
	w.Add(2)
	w.Add(3)
	// idx = 3*99/100 = 2, clamp not needed but exercises the boundary.
	p99, ok := w.P99()
	if !ok || p99 != 3 {
		t.Fatalf("P99() = %d, %v; want 3, true", p99, ok)
	}
}

func TestWindowLen(t *testing.T) {
	var w Window
	w.Add(1)
	w.Add(2)
	w.Add(-1) // ignored
	if got := w.Len(); got != 2 {
		t.Fatalf("Len() = %d, want 2", got)
	}
}

func TestWindowRecent(t *testing.T) {
	var w Window
	for i := int64(1); i <= 5; i++ {
		w.Add(i)
	}
	got := w.Recent(3)
	want := []int64{3, 4, 5}
	if len(got) != len(want) {
		t.Fatalf("Recent(3) = %v, want %v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("Recent(3) = %v, want %v", got, want)
		}
	}
	// n larger than the window returns everything, oldest first.
	all := w.Recent(100)
	if len(all) != 5 || all[0] != 1 || all[4] != 5 {
		t.Fatalf("Recent(100) = %v, want [1 2 3 4 5]", all)
	}
	// Mutating the returned slice must not affect the window.
	all[0] = 999
	if v, _ := w.Min(); v != 1 {
		t.Fatalf("Min() = %d after mutating Recent() result, want 1 (Recent must copy)", v)
	}
}
