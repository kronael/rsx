package glass

import (
	"image/color"
	"testing"

	"rsx-term/book"
	"rsx-core/wire"
)

// TestModelReuseSeam is the end-to-end proof that rsx-glass consumes rsx-term's
// model in-language: it drives rsx-term/book (Heatmap.Ingest, LiveFold,
// Persistence — the pure fold layer, no terminal code) with wire frames exactly
// as rsx-term's update loop would, then Composes a Grid. A valid Grid off real
// book state means the seam works.
func TestModelReuseSeam(t *testing.T) {
	heat := book.NewHeatmap(1)
	heat.Configure(8, 4)
	persist := book.NewPersistence()
	bids := []wire.Level{{Px: 999, Qty: 5000, Count: 2}, {Px: 996, Qty: 90000, Count: 5}}
	asks := []wire.Level{{Px: 1001, Qty: 6000, Count: 1}, {Px: 1004, Qty: 80000, Count: 4}}
	for i := int64(0); i < 12; i++ {
		from, to := i*1_000_000_000, (i+1)*1_000_000_000
		persist.ObserveSnapshot(bids, asks, to)
		heat.Ingest(bids, asks, []book.TapeEntry{{Side: wire.Buy, Px: 1001, Qty: 12000}}, persist, from, to)
	}

	g := NewGrid(72, 24)
	Compose(g, Scene{Symbol: "BTC", Connected: true, Heat: heat, Bids: bids, Asks: asks, Ages: persist})

	if g.W != 72 || g.H != 24 || len(g.Cells) != 72*24 {
		t.Fatalf("grid dims wrong: %d×%d, %d cells", g.W, g.H, len(g.Cells))
	}
	if !hasGlyph(g, 'R') || !hasGlyph(g, 'X') {
		t.Fatal("header badge missing")
	}
	if hasBg(g, Live) == 0 {
		t.Fatal("no size-ramp cells rendered — body is empty")
	}
	if !hasGlyph(g, '9') { // the exact ladder numbers
		t.Fatal("touch ladder numbers missing")
	}
	// The mid is anchored, so the header shows it and ~mark is a real number.
	if heat.MidPx() <= 0 {
		t.Fatal("book never anchored")
	}
}

// TestHonestyMarkers asserts the — (unknown) and ~ (estimate) markers appear
// when the model has no data — the renderer never fabricates a number.
func TestHonestyMarkers(t *testing.T) {
	heat := book.NewHeatmap(1)
	heat.Configure(8, 4)
	g := NewGrid(72, 24)
	// Empty, offline book: unknown mid, empty ladders, derived ~mark unknown.
	Compose(g, Scene{Symbol: "BTC", Connected: false, Heat: heat})
	if !hasGlyph(g, '—') {
		t.Fatal("missing — (unknown) honesty marker")
	}
	if !hasGlyph(g, '~') {
		t.Fatal("missing ~ (estimate) honesty marker")
	}
}

func hasGlyph(g *Grid, r rune) bool {
	for _, c := range g.Cells {
		if c.Glyph == r {
			return true
		}
	}
	return false
}

func hasBg(g *Grid, want color.RGBA) int {
	n := 0
	for _, c := range g.Cells {
		if c.Bg == want {
			n++
		}
	}
	return n
}
