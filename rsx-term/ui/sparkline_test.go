package ui

import "testing"

func TestSparklineEmpty(t *testing.T) {
	if got := sparkline(nil); got != "" {
		t.Fatalf("sparkline(nil) = %q, want empty", got)
	}
}

func TestSparklineFullRamp(t *testing.T) {
	// span 1..8 maps one-to-one onto the 8-glyph ramp.
	got := sparkline([]int64{1, 2, 3, 4, 5, 6, 7, 8})
	want := "▁▂▃▄▅▆▇█"
	if got != want {
		t.Fatalf("sparkline() = %q, want %q", got, want)
	}
}

func TestSparklineFlatWindowUsesMidGlyph(t *testing.T) {
	got := sparkline([]int64{5, 5, 5})
	want := "▅▅▅"
	if got != want {
		t.Fatalf("sparkline(flat) = %q, want %q", got, want)
	}
}

func TestSparklineScalesToOwnRange(t *testing.T) {
	// min=100, max=200, span=100: 100->idx0, 200->idx7 (top glyph),
	// 150->idx (50*7)/100=3 (integer division, truncates toward the low end).
	got := sparkline([]int64{100, 150, 200})
	want := "▁▄█"
	if got != want {
		t.Fatalf("sparkline() = %q, want %q", got, want)
	}
}
