package conn

import (
	"testing"

	"rsx-term/feed"
	"rsx-term/wire"
)

func TestDecodeHLMeta(t *testing.T) {
	body := []byte(`{"universe":[
		{"name":"BTC","szDecimals":5},
		{"name":"PEPE","szDecimals":0},
		{"name":"ETH","szDecimals":4}
	]}`)
	got, err := DecodeHLMeta(body)
	if err != nil {
		t.Fatalf("decode: %v", err)
	}
	if len(got) != 3 {
		t.Fatalf("universe len = %d, want 3", len(got))
	}
	btc := got[0]
	if btc.ID != 1 || btc.Coin != "BTC" || btc.PriceDec != 1 || btc.QtyDec != 5 {
		t.Fatalf("BTC mapping = %+v (want id 1, pxDec 6-5=1, qtyDec 5)", btc)
	}
	if got[1].PriceDec != 6 || got[2].ID != 3 {
		t.Fatalf("PEPE/ETH mapping wrong: %+v", got[1:])
	}
}

func TestParseHLFixed(t *testing.T) {
	cases := []struct {
		s    string
		dec  int
		want int64
		ok   bool
	}{
		{"42999.5", 1, 429995, true},
		{"42999", 1, 429990, true},
		{"0.0000123", 7, 123, true},
		{"1.23456789", 4, 12345, true}, // excess precision truncates
		{"", 2, 0, false},
		{"12a.3", 1, 0, false},
	}
	for _, c := range cases {
		got, ok := ParseHLFixed(c.s, c.dec)
		if got != c.want || ok != c.ok {
			t.Fatalf("ParseHLFixed(%q,%d) = %d,%v want %d,%v", c.s, c.dec, got, ok, c.want, c.ok)
		}
	}
}

func hlFixture() *HL {
	instruments := []HLInstrument{
		{ID: 1, Coin: "BTC", PriceDec: 1, QtyDec: 5},
		{ID: 2, Coin: "ETH", PriceDec: 2, QtyDec: 4},
	}
	return NewHL(instruments, nil)
}

func TestDecodeHLBookFrame(t *testing.T) {
	h := hlFixture()
	frame := []byte(`{"channel":"l2Book","data":{
		"coin":"BTC","time":1700000000123,
		"levels":[
			[{"px":"42999.5","sz":"0.25","n":3},{"px":"42999.0","sz":"1.5","n":1}],
			[{"px":"43000.0","sz":"0.75","n":2}]
		]}}`)
	msgs := h.DecodeHLMessage(frame)
	if len(msgs) != 1 {
		t.Fatalf("book frame → %d msgs, want 1", len(msgs))
	}
	vm := msgs[0].(feed.VenueMsg)
	if vm.Venue != HLVenueName {
		t.Fatalf("venue tag = %q", vm.Venue)
	}
	snap := vm.Msg.(wire.Snapshot)
	if snap.SymbolID != 1 || len(snap.Bids) != 2 || len(snap.Asks) != 1 {
		t.Fatalf("snapshot shape wrong: %+v", snap)
	}
	if snap.Bids[0].Px != 429995 || snap.Bids[0].Qty != 25000 || snap.Bids[0].Count != 3 {
		t.Fatalf("best bid mapping: %+v", snap.Bids[0])
	}
	if snap.TsNs != 1700000000123*1_000_000 {
		t.Fatalf("ts mapping: %d", snap.TsNs)
	}
}

func TestDecodeHLTradesFrame(t *testing.T) {
	h := hlFixture()
	frame := []byte(`{"channel":"trades","data":[
		{"coin":"ETH","side":"A","px":"2250.55","sz":"2.5","time":1700000000500,"tid":42},
		{"coin":"UNKNOWN","side":"B","px":"1.0","sz":"1","time":1,"tid":43},
		{"coin":"BTC","side":"B","px":"43000.0","sz":"0.1","time":1700000000501,"tid":44}
	]}`)
	msgs := h.DecodeHLMessage(frame)
	if len(msgs) != 2 {
		t.Fatalf("trades frame → %d msgs, want 2 (unknown coin skipped)", len(msgs))
	}
	sell := msgs[0].(feed.VenueMsg).Msg.(wire.MdTrade)
	if sell.SymbolID != 2 || sell.TakerSide != 1 || sell.Px != 225055 || sell.Qty != 25000 {
		t.Fatalf("sell print mapping: %+v", sell)
	}
	buy := msgs[1].(feed.VenueMsg).Msg.(wire.MdTrade)
	if buy.SymbolID != 1 || buy.TakerSide != 0 || buy.Seq != 44 {
		t.Fatalf("buy print mapping: %+v", buy)
	}
}

func TestDecodeHLTradesFrameSkipsMalformedSide(t *testing.T) {
	h := hlFixture()
	// A malformed/unknown side must be skipped, not silently reported as a
	// buy aggressor (HL-MALFORMED-TRADE-SIDE-BECOMES-BUY).
	frame := []byte(`{"channel":"trades","data":[
		{"coin":"ETH","side":"S","px":"2250.55","sz":"2.5","time":1700000000500,"tid":42},
		{"coin":"ETH","side":"","px":"2250.55","sz":"2.5","time":1700000000500,"tid":43},
		{"coin":"BTC","side":"B","px":"43000.0","sz":"0.1","time":1700000000501,"tid":44}
	]}`)
	msgs := h.DecodeHLMessage(frame)
	if len(msgs) != 1 {
		t.Fatalf("trades frame → %d msgs, want 1 (malformed sides skipped)", len(msgs))
	}
	buy := msgs[0].(feed.VenueMsg).Msg.(wire.MdTrade)
	if buy.SymbolID != 1 || buy.TakerSide != 0 || buy.Seq != 44 {
		t.Fatalf("surviving buy print mapping: %+v", buy)
	}
}

func TestDecodeHLIgnoresOtherChannels(t *testing.T) {
	h := hlFixture()
	for _, frame := range []string{
		`{"channel":"pong"}`,
		`{"channel":"subscriptionResponse","data":{}}`,
		`not json`,
	} {
		if msgs := h.DecodeHLMessage([]byte(frame)); msgs != nil {
			t.Fatalf("frame %q should decode to nothing: %v", frame, msgs)
		}
	}
}

func TestNewHLFiltersCoins(t *testing.T) {
	instruments := []HLInstrument{
		{ID: 1, Coin: "BTC"}, {ID: 2, Coin: "ETH"}, {ID: 3, Coin: "WIF"},
	}
	h := NewHL(instruments, []string{"btc", " WIF "})
	if len(h.Instruments()) != 2 {
		t.Fatalf("coin filter kept %d, want 2", len(h.Instruments()))
	}
}

func TestSectorOf(t *testing.T) {
	if SectorOf("btc") != "majors" || SectorOf("WIF") != "meme" || SectorOf("XYZNEW") != "other" {
		t.Fatalf("sector table drifted: %s %s %s", SectorOf("btc"), SectorOf("WIF"), SectorOf("XYZNEW"))
	}
}
