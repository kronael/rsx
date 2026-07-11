package assistant

import (
	"strings"
	"testing"

	"rsx-term/news"
	"rsx-term/wire"
)

// newsCtx is a fixed news handoff for Render tests (frozen ts so the rendered
// timestamp is deterministic).
func newsCtx() news.AssistantContext {
	h := news.Marker{Text: "Binance lists SOL pair", Source: "Blogs", Tier: 2}
	return news.AssistantContext{
		Origin:   news.OriginNews,
		Venue:    "rsx",
		Symbol:   "SOL-PERP",
		TsNs:     1_700_000_000_000_000_000,
		Headline: &h,
		Bids:     []wire.Level{{Px: 1_499_950, Qty: 40_000_000}, {Px: 1_499_900, Qty: 10_000_000}},
		Asks:     []wire.Level{{Px: 1_500_050, Qty: 35_000_000}},
		MidPx:    1_500_000,
	}
}

func TestRenderNewsContextBlock(t *testing.T) {
	snap := Snapshot{PriceDec: 4, QtyDec: 6, Fills: 3}
	out := Render(newsCtx(), snap)
	for _, want := range []string{
		"[RSX CONTEXT]",
		"origin: news",
		"market: rsx · SOL-PERP  at 2023-11-14 22:13:20 UTC",
		"headline: Binance lists SOL pair  (Blogs · severity 2)",
		"mid: 150.0000",
		"asks: 150.0050×35.000000",
		"bids: 149.9950×40.000000  149.9900×10.000000",
		"[TRADER STATE]",
		"position: flat",
		"open orders: none",
		"fills this session: 3",
	} {
		if !strings.Contains(out, want) {
			t.Fatalf("prompt missing %q:\n%s", want, out)
		}
	}
}

func TestRenderPositionAndOrders(t *testing.T) {
	snap := Snapshot{
		PriceDec: 4, QtyDec: 6,
		Net:   5_000_000, // +5 lots
		Entry: 1_500_000, HasEntry: true,
		// Raw uPnL is Net_raw*mark_raw - Cost_raw at price×qty scale (10^10 here):
		// 5 lots that gained 0.05 in price = $0.25 → raw 2.5e9.
		Upnl:    2_500_000_000,
		HasUpnl: true,
		Fills:   1,
		Orders: []SnapshotOrder{
			{Side: wire.Buy, Px: 1_499_950, Qty: 4_000_000},
			{Side: wire.Sell, Px: 1_500_100, Qty: 2_000_000},
		},
	}
	out := Render(newsCtx(), snap)
	// uPnL divides out the qty scale (10^6) then formats at priceDec: 2.5e9 → $0.25.
	if !strings.Contains(out, "position: +5.000000  entry 150.0000  uPnL +0.2500") {
		t.Fatalf("position line wrong:\n%s", out)
	}
	if !strings.Contains(out, "open orders: BUY 149.9950×4.000000, SELL 150.0100×2.000000") {
		t.Fatalf("orders line wrong:\n%s", out)
	}
}

func TestRenderDashesUnknowns(t *testing.T) {
	// A net position with no entry / uPnL computable must dash them, not fabricate.
	snap := Snapshot{PriceDec: 4, QtyDec: 6, Net: -3_000_000}
	out := Render(newsCtx(), snap)
	if !strings.Contains(out, "position: -3.000000  entry —  uPnL —") {
		t.Fatalf("unknown entry/uPnL should dash:\n%s", out)
	}
	// No mid / empty book → dashes, never a fabricated level.
	bare := news.AssistantContext{Origin: news.OriginBookFreeze, Venue: "rsx", Symbol: "PENGU-PERP", Note: "exact ~100ms bin"}
	out = Render(bare, Snapshot{PriceDec: 6, QtyDec: 4})
	for _, want := range []string{"origin: book freeze", "note: exact ~100ms bin", "mid: —", "asks: —", "bids: —"} {
		if !strings.Contains(out, want) {
			t.Fatalf("book-freeze prompt missing %q:\n%s", want, out)
		}
	}
	if strings.Contains(out, "headline:") {
		t.Fatalf("a book freeze must not fabricate a headline:\n%s", out)
	}
}
