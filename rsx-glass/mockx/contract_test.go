package mockx_test

import (
	"bytes"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"rsx-core/wire"
	"rsx-glass/mockx"
)

// The contract slice: proof the mock cannot drift from the real exchange.
//
// testdata/*.bin are the REAL rsx-marketdata producer's canonical wire vectors,
// lifted verbatim from its own pinned golden test (rsx-marketdata/src/
// wire_test.rs, which asserts the Rust encode_* output equals these exact
// bytes). For each we do three things:
//
//  1. mockx.Encode(...) == the real bytes — the mock's Go encoder is
//     byte-identical to the real Rust producer (the strongest structural match:
//     same tags, same proto3 default-elision, same framing).
//  2. wire.DecodeMd(real bytes) yields the expected struct — the real bytes
//     decode through rsx-term's REAL decoder (the one the terminal/live path
//     uses), i.e. the fixtures are valid feed frames.
//  3. wire.DecodeMd(mock bytes) == wire.DecodeMd(real bytes) — mock and real
//     frames are indistinguishable to the shared decoder.
//
// (2) and (3) hold even if the byte-level check in (1) were ever relaxed for a
// benign encoding difference, so the semantic contract is decoder-anchored, not
// just byte-anchored.

// contractCase pairs a real-producer fixture with the mock input that must
// reproduce it and the decoded value that input must yield.
type contractCase struct {
	fixture string
	msg     any // the wire.* value fed to mockx.Encode
	want    any // the value wire.DecodeMd must return for both
}

func contractCases() []contractCase {
	return []contractCase{
		{
			fixture: "bbo.bin",
			msg:     wire.Bbo{SymbolID: 1, BidPx: 100, BidQty: 5, BidCount: 2, AskPx: 101, AskQty: 7, AskCount: 3, TsNs: 1000, Seq: 42},
			want:    wire.Bbo{SymbolID: 1, BidPx: 100, BidQty: 5, BidCount: 2, AskPx: 101, AskQty: 7, AskCount: 3, TsNs: 1000, Seq: 42},
		},
		{
			fixture: "snapshot.bin",
			msg: wire.Snapshot{SymbolID: 1,
				Bids: []wire.Level{{Px: 100, Qty: 5, Count: 2}, {Px: 99, Qty: 3, Count: 1}},
				Asks: []wire.Level{{Px: 101, Qty: 7, Count: 3}}, TsNs: 2000, Seq: 99},
			want: wire.Snapshot{SymbolID: 1,
				Bids: []wire.Level{{Px: 100, Qty: 5, Count: 2}, {Px: 99, Qty: 3, Count: 1}},
				Asks: []wire.Level{{Px: 101, Qty: 7, Count: 3}}, TsNs: 2000, Seq: 99},
		},
		{
			fixture: "delta.bin",
			msg:     wire.Delta{SymbolID: 1, Side: 1, Px: 100, Qty: 5, Count: 1, TsNs: 3000, Seq: 77},
			want:    wire.Delta{SymbolID: 1, Side: 1, Px: 100, Qty: 5, Count: 1, TsNs: 3000, Seq: 77},
		},
		{
			fixture: "trade.bin",
			msg:     wire.MdTrade{SymbolID: 4, Px: 300, Qty: 25, TakerSide: 0, TsNs: 8000, Seq: 66},
			want:    wire.MdTrade{SymbolID: 4, Px: 300, Qty: 25, TakerSide: 0, TsNs: 8000, Seq: 66},
		},
		{
			fixture: "heartbeat.bin",
			msg:     wire.MdHeartbeat{TsMs: 12345},
			want:    wire.MdHeartbeat{TsMs: 12345},
		},
		{
			fixture: "heartbeat_zero.bin",
			msg:     wire.MdHeartbeat{TsMs: 0},
			want:    wire.MdHeartbeat{TsMs: 0},
		},
	}
}

func TestMockEncoderMatchesRealBytes(t *testing.T) {
	for _, tc := range contractCases() {
		t.Run(tc.fixture, func(t *testing.T) {
			real := readFixture(t, tc.fixture)
			got := mockx.Encode(tc.msg)
			if !bytes.Equal(got, real) {
				t.Fatalf("mock bytes drifted from the real producer\n mock: %v\n real: %v", got, real)
			}
		})
	}
}

func TestRealAndMockDecodeIdentically(t *testing.T) {
	for _, tc := range contractCases() {
		t.Run(tc.fixture, func(t *testing.T) {
			real := readFixture(t, tc.fixture)

			fromReal, err := wire.DecodeMd(real)
			if err != nil {
				t.Fatalf("real fixture failed the real decoder: %v", err)
			}
			if !reflect.DeepEqual(fromReal, tc.want) {
				t.Fatalf("real decode = %#v, want %#v", fromReal, tc.want)
			}

			fromMock, err := wire.DecodeMd(mockx.Encode(tc.msg))
			if err != nil {
				t.Fatalf("mock frame failed the real decoder: %v", err)
			}
			if !reflect.DeepEqual(fromMock, fromReal) {
				t.Fatalf("mock decodes differently than real\n mock: %#v\n real: %#v", fromMock, fromReal)
			}
		})
	}
}

// TestEncodeRoundTrip confirms every scenario frame the mock server emits
// survives the real decoder unchanged — the wire types are the SAME on both
// ends (mockx.Encode consumes rsx-term's wire.* types, wire.DecodeMd returns
// them), so this is a true structural round-trip, not a re-parse into a copy.
func TestEncodeRoundTrip(t *testing.T) {
	for i, msg := range mockx.Scenario(mockx.DefaultSymbolID, 1) {
		frame := mockx.Encode(msg)
		if frame == nil {
			t.Fatalf("frame %d: mock could not encode %T", i, msg)
		}
		decoded, err := wire.DecodeMd(frame)
		if err != nil {
			t.Fatalf("frame %d (%T): decode failed: %v", i, msg, err)
		}
		if !reflect.DeepEqual(decoded, msg) {
			t.Fatalf("frame %d round-trip mismatch\n in:  %#v\n out: %#v", i, msg, decoded)
		}
	}
}

func readFixture(t *testing.T, name string) []byte {
	t.Helper()
	b, err := os.ReadFile(filepath.Join("testdata", name))
	if err != nil {
		t.Fatalf("read fixture %s: %v", name, err)
	}
	return b
}
