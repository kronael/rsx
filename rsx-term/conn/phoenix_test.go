package conn

import (
	"testing"

	"rsx-term/feed"
	"rsx-term/wire"
)

func TestDecodePhxMarkets(t *testing.T) {
	// The live endpoint returns a bare JSON array (the display name rides under
	// a nested metadata object this decoder skips).
	body := []byte(`[
		{"symbol":"SOL","tickSize":100,"baseLotsDecimals":2,"assetId":0,"metadata":{"name":"Solana"}},
		{"symbol":"BTC","tickSize":1,"baseLotsDecimals":4,"assetId":1,"metadata":{"name":"Bitcoin"}},
		{"symbol":"WEIRD","tickSize":1,"baseLotsDecimals":-1,"assetId":2},
		{"symbol":"FINE","tickSize":1,"baseLotsDecimals":12,"assetId":3}
	]`)
	got, err := DecodePhxMarkets(body)
	if err != nil {
		t.Fatalf("decode: %v", err)
	}
	if len(got) != 4 {
		t.Fatalf("markets len = %d, want 4", len(got))
	}
	// ID = assetId + 1: SOL's assetId 0 must NOT stay 0 (that id is the
	// terminal's "unspecified -> primary" sentinel).
	sol := got[0]
	if sol.ID != 1 || sol.Symbol != "SOL" || sol.PriceDec != 4 || sol.QtyDec != 2 {
		t.Fatalf("SOL mapping = %+v (want id 1, pxDec 4, qtyDec 2)", sol)
	}
	if got[1].ID != 2 || got[1].QtyDec != 4 {
		t.Fatalf("BTC mapping = %+v (want id 2, qtyDec 4)", got[1])
	}
	// baseLotsDecimals clamps to [0, 8]: -1 -> 0, 12 -> 8.
	if got[2].QtyDec != 0 {
		t.Fatalf("negative baseLotsDecimals should clamp to 0: %+v", got[2])
	}
	if got[3].QtyDec != 8 {
		t.Fatalf("baseLotsDecimals 12 should clamp to 8: %+v", got[3])
	}
}

func phxFixture() *Phoenix {
	// Ids mirror the assetId+1 convention (SOL assetId 0 -> id 1).
	instruments := []PhxInstrument{
		{ID: 1, Symbol: "SOL", PriceDec: 4, QtyDec: 2},
		{ID: 2, Symbol: "BTC", PriceDec: 4, QtyDec: 4},
	}
	return NewPhoenix(instruments, nil)
}

func TestDecodePhxOrderbookFrame(t *testing.T) {
	p := phxFixture()
	frame := []byte(`{"channel":"orderbook","symbol":"SOL","orderbook":{
		"bids":[[150.25,100.0],[150.2,200.0]],
		"asks":[[150.3,150.0]],
		"mid":150.275}}`)
	msgs := p.DecodePhxMessage(frame)
	if len(msgs) != 1 {
		t.Fatalf("orderbook frame → %d msgs, want 1", len(msgs))
	}
	vm := msgs[0].(feed.VenueMsg)
	if vm.Venue != PhoenixVenueName {
		t.Fatalf("venue tag = %q", vm.Venue)
	}
	snap := vm.Msg.(wire.Snapshot)
	if snap.SymbolID != 1 || len(snap.Bids) != 2 || len(snap.Asks) != 1 {
		t.Fatalf("snapshot shape wrong: %+v", snap)
	}
	// 150.25 at PriceDec 4 → 1502500; 100.0 at QtyDec 2 → 10000; no count.
	if snap.Bids[0].Px != 1502500 || snap.Bids[0].Qty != 10000 || snap.Bids[0].Count != 0 {
		t.Fatalf("best bid mapping: %+v", snap.Bids[0])
	}
	if snap.Asks[0].Px != 1503000 || snap.Asks[0].Qty != 15000 {
		t.Fatalf("best ask mapping: %+v", snap.Asks[0])
	}
}

func TestDecodePhxOrderbookSkipsMalformedLevels(t *testing.T) {
	p := phxFixture()
	// A zero size and a short (one-element) array must both be dropped (never
	// fabricated), leaving only the one clean level. Prices ride as JSON
	// numbers, so a non-numeric price fails the whole frame decode instead
	// (a different, also-safe path — the frame is skipped, next snapshot heals).
	frame := []byte(`{"channel":"orderbook","symbol":"BTC","orderbook":{
		"bids":[[65000.5,0.0],[65000.0],[64999.0,0.5]],
		"asks":[]}}`)
	msgs := p.DecodePhxMessage(frame)
	if len(msgs) != 1 {
		t.Fatalf("orderbook frame → %d msgs, want 1", len(msgs))
	}
	snap := msgs[0].(feed.VenueMsg).Msg.(wire.Snapshot)
	if len(snap.Bids) != 1 || len(snap.Asks) != 0 {
		t.Fatalf("malformed levels not skipped: %+v", snap)
	}
	// 64999.0 at PriceDec 4 → 649990000; 0.5 at QtyDec 4 → 5000.
	if snap.Bids[0].Px != 649990000 || snap.Bids[0].Qty != 5000 {
		t.Fatalf("surviving level mapping: %+v", snap.Bids[0])
	}
}

// TestDecodePhxTradesFrameIgnored pins the orderbook-only decision: a trades
// frame currently decodes to nothing (see TODO(phoenix-trades)). When the
// quote/base price derivation lands, this test becomes the trades-fold test.
func TestDecodePhxTradesFrameIgnored(t *testing.T) {
	p := phxFixture()
	frame := []byte(`{"channel":"trades","symbol":"SOL","trades":[
		{"side":"bid","baseAmount":10.0,"quoteAmount":1500.0,"timestamp":"1775578550","tradeSequenceNumber":"100"}
	]}`)
	if msgs := p.DecodePhxMessage(frame); msgs != nil {
		t.Fatalf("trades frame should decode to nothing for now: %v", msgs)
	}
}

func TestDecodePhxIgnoresOtherFrames(t *testing.T) {
	p := phxFixture()
	for _, frame := range []string{
		`{"type":"pong"}`,
		`{"channel":"orderbook","symbol":"UNKNOWN","orderbook":{"bids":[],"asks":[]}}`,
		`{"channel":"subscribed","symbol":"SOL"}`,
		`not json`,
	} {
		if msgs := p.DecodePhxMessage([]byte(frame)); msgs != nil {
			t.Fatalf("frame %q should decode to nothing: %v", frame, msgs)
		}
	}
}

func TestNewPhoenixFiltersSymbols(t *testing.T) {
	instruments := []PhxInstrument{
		{ID: 0, Symbol: "SOL"}, {ID: 1, Symbol: "BTC"}, {ID: 2, Symbol: "HYPE"},
	}
	p := NewPhoenix(instruments, []string{"sol", " HYPE "})
	if len(p.Instruments()) != 2 {
		t.Fatalf("symbol filter kept %d, want 2", len(p.Instruments()))
	}
}
