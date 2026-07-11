package news

import (
	"testing"

	"rsx-term/wire"
)

func TestPackageNewsCarriesHeadlineAndDeepCopies(t *testing.T) {
	bids := []wire.Level{{Px: 100, Qty: 5, Count: 1}}
	asks := []wire.Level{{Px: 101, Qty: 7, Count: 1}}
	h := Marker{Text: "halt", Source: "Twitter", Tier: 3}
	ctx := PackageNews("rsx", "BTC", 42, h, bids, asks, 100)
	if ctx.Origin != OriginNews {
		t.Fatalf("origin = %v, want news", ctx.Origin)
	}
	if ctx.Headline == nil || ctx.Headline.Text != "halt" {
		t.Fatalf("headline not packaged: %+v", ctx.Headline)
	}
	// Deep copy: mutating the source must not touch the frozen context.
	bids[0].Qty = 999
	if ctx.Bids[0].Qty != 5 {
		t.Fatalf("bids must be deep-copied, got %d", ctx.Bids[0].Qty)
	}
}

func TestPackageBookFreezeHasNoHeadline(t *testing.T) {
	asks := []wire.Level{{Px: 101, Qty: 7, Count: 1}}
	ctx := PackageBookFreeze("rsx", "BTC", 42, "~10s window", nil, asks, 100)
	if ctx.Origin != OriginBookFreeze {
		t.Fatalf("origin = %v, want book freeze", ctx.Origin)
	}
	if ctx.Headline != nil {
		t.Fatalf("a book freeze must not carry a headline: %+v", ctx.Headline)
	}
	if ctx.Note != "~10s window" {
		t.Fatalf("note = %q", ctx.Note)
	}
	if ctx.Bids != nil {
		t.Fatalf("nil bids should clone to nil")
	}
	asks[0].Qty = 999
	if ctx.Asks[0].Qty != 7 {
		t.Fatalf("asks must be deep-copied, got %d", ctx.Asks[0].Qty)
	}
}
