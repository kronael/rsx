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
}
