package wire

import "testing"

// Golden byte vectors copied verbatim from
// rsx-marketdata/src/wire_test.rs — the Rust encoder's pinned output.
// Decoding them here proves this Go decoder agrees with the Rust
// producer on the exact tag/field layout.

func TestDecodeMdBboGoldenBytes(t *testing.T) {
	frame := []byte{
		0, 0, 0, 21, 10, 19, 8, 1, 16, 100, 24, 5, 32, 2, 40, 101, 48, 7, 56, 3, 64, 232, 7,
		72, 42,
	}
	got, err := DecodeMd(frame)
	if err != nil {
		t.Fatalf("DecodeMd: %v", err)
	}
	bbo, ok := got.(Bbo)
	if !ok {
		t.Fatalf("expected Bbo, got %T", got)
	}
	want := Bbo{SymbolID: 1, BidPx: 100, BidQty: 5, BidCount: 2, AskPx: 101, AskQty: 7, AskCount: 3, TsNs: 1000, Seq: 42}
	if bbo != want {
		t.Errorf("got %+v want %+v", bbo, want)
	}
}

func TestDecodeMdSnapshotGoldenBytes(t *testing.T) {
	frame := []byte{
		0, 0, 0, 33, 18, 31, 8, 1, 18, 6, 8, 100, 16, 5, 24, 2, 18, 6, 8, 99, 16, 3, 24, 1, 26,
		6, 8, 101, 16, 7, 24, 3, 32, 208, 15, 40, 99,
	}
	got, err := DecodeMd(frame)
	if err != nil {
		t.Fatalf("DecodeMd: %v", err)
	}
	snap, ok := got.(Snapshot)
	if !ok {
		t.Fatalf("expected Snapshot, got %T", got)
	}
	if snap.SymbolID != 1 || snap.TsNs != 2000 || snap.Seq != 99 {
		t.Errorf("got %+v", snap)
	}
	wantBids := []Level{{Px: 100, Qty: 5, Count: 2}, {Px: 99, Qty: 3, Count: 1}}
	wantAsks := []Level{{Px: 101, Qty: 7, Count: 3}}
	if !levelsEqual(snap.Bids, wantBids) {
		t.Errorf("bids = %+v want %+v", snap.Bids, wantBids)
	}
	if !levelsEqual(snap.Asks, wantAsks) {
		t.Errorf("asks = %+v want %+v", snap.Asks, wantAsks)
	}
}

func levelsEqual(a, b []Level) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

func TestDecodeMdDeltaGoldenBytes(t *testing.T) {
	frame := []byte{0, 0, 0, 17, 26, 15, 8, 1, 16, 1, 24, 100, 32, 5, 40, 1, 48, 184, 23, 56, 77}
	got, err := DecodeMd(frame)
	if err != nil {
		t.Fatalf("DecodeMd: %v", err)
	}
	delta, ok := got.(Delta)
	if !ok {
		t.Fatalf("expected Delta, got %T", got)
	}
	want := Delta{SymbolID: 1, Side: 1, Px: 100, Qty: 5, Count: 1, TsNs: 3000, Seq: 77}
	if delta != want {
		t.Errorf("got %+v want %+v", delta, want)
	}
}

func TestDecodeMdTradeGoldenBytesAbsentTakerSide(t *testing.T) {
	frame := []byte{0, 0, 0, 14, 34, 12, 8, 4, 16, 172, 2, 24, 25, 40, 192, 62, 48, 66}
	got, err := DecodeMd(frame)
	if err != nil {
		t.Fatalf("DecodeMd: %v", err)
	}
	trade, ok := got.(MdTrade)
	if !ok {
		t.Fatalf("expected MdTrade, got %T", got)
	}
	// taker_side omitted on the wire (proto3 zero-omission): must decode as 0.
	want := MdTrade{SymbolID: 4, Px: 300, Qty: 25, TakerSide: 0, TsNs: 8000, Seq: 66}
	if trade != want {
		t.Errorf("got %+v want %+v", trade, want)
	}
}

func TestDecodeMdHeartbeatGoldenBytes(t *testing.T) {
	frame := []byte{0, 0, 0, 5, 42, 3, 8, 185, 96}
	got, err := DecodeMd(frame)
	if err != nil {
		t.Fatalf("DecodeMd: %v", err)
	}
	hb, ok := got.(MdHeartbeat)
	if !ok {
		t.Fatalf("expected MdHeartbeat, got %T", got)
	}
	if hb.TsMs != 12345 {
		t.Errorf("TsMs = %d want 12345", hb.TsMs)
	}
}

func TestDecodeMdHeartbeatZeroOmitsScalar(t *testing.T) {
	// timestamp_ms = 0 is a proto3 default: the Heartbeat body is
	// empty, so the frame is just the oneof wrapper (tag 5, length 0).
	frame := []byte{0, 0, 0, 2, 42, 0}
	got, err := DecodeMd(frame)
	if err != nil {
		t.Fatalf("DecodeMd: %v", err)
	}
	hb, ok := got.(MdHeartbeat)
	if !ok {
		t.Fatalf("expected MdHeartbeat, got %T", got)
	}
	if hb.TsMs != 0 {
		t.Errorf("TsMs = %d want 0", hb.TsMs)
	}
}

// TestDecodeMdNegativePriceDelta hand-encodes a Delta with price =
// -1: a two's-complement varint, 10 bytes (0xff * 9, then 0x01),
// since a negative int64 always sets the top bit and thus needs the
// full 64-bit varint encoding.
func TestDecodeMdNegativePriceDelta(t *testing.T) {
	body := []byte{
		8, 1, // symbol_id = 1 (tag 1, varint)
		16, 0, // side = 0 (tag 2, varint) -- omit is also valid but keep explicit
		24,                                                         // tag 3 (price), wire type 0
		0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01, // -1 as u64 varint
	}
	outer := []byte{26, byte(len(body))} // tag 3 (Delta), wire type 2
	outer = append(outer, body...)
	frame := make([]byte, 4)
	frame[0] = byte(len(outer) >> 24)
	frame[1] = byte(len(outer) >> 16)
	frame[2] = byte(len(outer) >> 8)
	frame[3] = byte(len(outer))
	frame = append(frame, outer...)

	got, err := DecodeMd(frame)
	if err != nil {
		t.Fatalf("DecodeMd: %v", err)
	}
	delta, ok := got.(Delta)
	if !ok {
		t.Fatalf("expected Delta, got %T", got)
	}
	if delta.Px != -1 {
		t.Errorf("Px = %d want -1", delta.Px)
	}
}

func TestDecodeMdMalformedFrames(t *testing.T) {
	if _, err := DecodeMd([]byte{0, 0, 1}); err == nil {
		t.Error("3-byte frame: expected error")
	}
	// wire type 5 (unsupported) on a top-level field.
	if _, err := DecodeMd([]byte{0, 0, 0, 1, 0x05}); err == nil {
		t.Error("wire type 5: expected error")
	}
	// truncated varint: continuation bit set, then frame ends.
	if _, err := DecodeMd([]byte{0, 0, 0, 1, 0x80}); err == nil {
		t.Error("truncated varint: expected error")
	}
}
