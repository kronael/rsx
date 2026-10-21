package glass

import (
	"image/color"
	"testing"

	"rsx-core/book"
	"rsx-core/wire"
)

// TestModelReuseSeam is the end-to-end proof that rsx-glass consumes the
// shared model in-language: it drives rsx-core/book (Heatmap.Ingest, LiveFold,
// Persistence — the pure fold layer, no UI code) with wire frames as the live
// feed would deliver them, then Composes a Grid. A valid Grid off real
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

// TestGlyphContrast is the reserved-contrast-pair gate: every background a
// glyph can sit on must give its ink >= 3:1 (WCAG 1.4.11 non-text minimum),
// in BOTH themes. It would have caught the old fg = "ramp one tier up"
// encoding (fg==bg at the top tier — the invisible wall glyph) AND the
// uncapped trade overlay (a buy print on a tier-4 wall was Live-on-Live,
// 1.00:1). The goldens can't see this: they lock stability, not visibility.
func TestGlyphContrast(t *testing.T) {
	const minRatio = 3.0
	t.Cleanup(func() { UseTheme("") })
	for _, theme := range []string{"", "colorblind"} {
		UseTheme(theme)
		name := theme
		if name == "" {
			name = "default"
		}
		// Reserved density / persistence / micro glyph inks, every ramp tier.
		for tier := 0; tier <= sizeTiers; tier++ {
			bg := rampColor(tier)
			if r := contrastRatio(glyphFg(bg), bg); r < minRatio {
				t.Errorf("%s: tier %d glyph fg %.2f:1 < %.1f:1 (invisible/near-invisible glyph)", name, tier, r, minRatio)
			}
		}
		// Trade overlay inks (both aggressor hues) on every background the
		// overlay cap allows.
		for tier := 0; tier <= overlayBgTier; tier++ {
			bg := rampColor(tier)
			for side, ink := range map[string]color.RGBA{"buy": aggressor(wire.Buy), "sell": aggressor(wire.Sell)} {
				if r := contrastRatio(ink, bg); r < minRatio {
					t.Errorf("%s: %s ink on overlay tier %d = %.2f:1 < %.1f:1", name, side, tier, r, minRatio)
				}
			}
		}
	}
}

// TestTradeSideByShape asserts the redundant channel: a buy and a sell of the
// same magnitude render DIFFERENT glyphs (side survives without hue).
func TestTradeSideByShape(t *testing.T) {
	buy := bookCell(cellAgg{size: 100, tradeQty: 500, tradeSide: wire.Buy}, 1000, 1000, false)
	sell := bookCell(cellAgg{size: 100, tradeQty: 500, tradeSide: wire.Sell}, 1000, 1000, false)
	if buy.Glyph == sell.Glyph {
		t.Fatalf("buy and sell prints share glyph %q — side is hue-only", buy.Glyph)
	}
	if !containsRune(tradeBuyRamp, buy.Glyph) {
		t.Fatalf("buy glyph %q not from the buy ramp", buy.Glyph)
	}
	if !containsRune(tradeSellRamp, sell.Glyph) {
		t.Fatalf("sell glyph %q not from the sell ramp", sell.Glyph)
	}
}

func containsRune(rs []rune, r rune) bool {
	for _, c := range rs {
		if c == r {
			return true
		}
	}
	return false
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
