// Package mockx is a deterministic mock RSX market-data exchange for tests: a
// small WebSocket server that speaks the SAME public-feed wire protocol the
// real rsx-marketdata producer does, so rsx-glass's live path can be exercised
// end to end without a cluster.
//
// This file is the encoder half of that protocol. rsx-term ships only the
// DECODER (wire.DecodeMd); the real producer's encoder lives in Rust
// (rsx-marketdata/src/wire.rs). This encoder is the Go mirror of that Rust one:
// the same hand-derived protobuf schema (marketdata.proto), the same STABLE
// field tags, the same proto3 rule that a zero-valued scalar is omitted, and
// the same framing — one MdFrame per WebSocket BINARY message, prefixed by a
// 4-byte big-endian length. It takes rsx-term's real wire types as input, so
// the mock and the terminal agree on the shape by construction; contract_test.go
// pins the bytes against the real Rust producer's golden vectors so this can
// never drift.
package mockx

import (
	"encoding/binary"

	"rsx-core/wire"
)

// Oneof body tags in the MdFrame envelope (marketdata.proto). STABLE.
const (
	tagBbo       = 1
	tagSnapshot  = 2
	tagDelta     = 3
	tagTrade     = 4
	tagHeartbeat = 5
)

// Encode encodes one market-data message (a wire.Snapshot / wire.Bbo /
// wire.Delta / wire.MdTrade / wire.MdHeartbeat) to a length-prefixed MdFrame
// frame. An unknown type returns nil — the caller only ever feeds it the five
// feed variants.
func Encode(msg any) []byte {
	switch v := msg.(type) {
	case wire.Snapshot:
		return EncodeSnapshot(v)
	case wire.Bbo:
		return EncodeBbo(v)
	case wire.Delta:
		return EncodeDelta(v)
	case wire.MdTrade:
		return EncodeTrade(v)
	case wire.MdHeartbeat:
		return EncodeHeartbeat(v.TsMs)
	default:
		return nil
	}
}

// EncodeSnapshot encodes a full L2 snapshot (oneof tag 2).
func EncodeSnapshot(s wire.Snapshot) []byte {
	var body []byte
	body = addVarint(body, 1, uint64(s.SymbolID))
	for _, bookLevel := range s.Bids {
		body = addSub(body, 2, level(bookLevel))
	}
	for _, bookLevel := range s.Asks {
		body = addSub(body, 3, level(bookLevel))
	}
	body = addVarint(body, 4, s.TsNs)
	body = addVarint(body, 5, s.Seq)
	return frame(addSub(nil, tagSnapshot, body))
}

// EncodeBbo encodes a best-bid/offer update (oneof tag 1).
func EncodeBbo(b wire.Bbo) []byte {
	var body []byte
	body = addVarint(body, 1, uint64(b.SymbolID))
	body = addI64(body, 2, b.BidPx)
	body = addI64(body, 3, b.BidQty)
	body = addVarint(body, 4, uint64(b.BidCount))
	body = addI64(body, 5, b.AskPx)
	body = addI64(body, 6, b.AskQty)
	body = addVarint(body, 7, uint64(b.AskCount))
	body = addVarint(body, 8, b.TsNs)
	body = addVarint(body, 9, b.Seq)
	return frame(addSub(nil, tagBbo, body))
}

// EncodeDelta encodes a single-level L2 delta (oneof tag 3).
func EncodeDelta(d wire.Delta) []byte {
	var body []byte
	body = addVarint(body, 1, uint64(d.SymbolID))
	body = addVarint(body, 2, uint64(d.Side))
	body = addI64(body, 3, d.Px)
	body = addI64(body, 4, d.Qty)
	body = addVarint(body, 5, uint64(d.Count))
	body = addVarint(body, 6, d.TsNs)
	body = addVarint(body, 7, d.Seq)
	return frame(addSub(nil, tagDelta, body))
}

// EncodeTrade encodes a trade print (oneof tag 4).
func EncodeTrade(t wire.MdTrade) []byte {
	var body []byte
	body = addVarint(body, 1, uint64(t.SymbolID))
	body = addI64(body, 2, t.Px)
	body = addI64(body, 3, t.Qty)
	body = addVarint(body, 4, uint64(t.TakerSide))
	body = addVarint(body, 5, t.TsNs)
	body = addVarint(body, 6, t.Seq)
	return frame(addSub(nil, tagTrade, body))
}

// EncodeHeartbeat encodes a server->client liveness ping (oneof tag 5).
func EncodeHeartbeat(tsMs uint64) []byte {
	var body []byte
	body = addVarint(body, 1, tsMs)
	return frame(addSub(nil, tagHeartbeat, body))
}

// level encodes one WireLevel submessage body (px=1, qty=2, count=3).
func level(bookLevel wire.Level) []byte {
	var b []byte
	b = addI64(b, 1, bookLevel.Px)
	b = addI64(b, 2, bookLevel.Qty)
	b = addVarint(b, 3, uint64(bookLevel.Count))
	return b
}

// tagKey is a protobuf field key: (field_number << 3) | wire_type.
func tagKey(field, wireType int) uint64 { return uint64(field)<<3 | uint64(wireType) }

// addVarint appends a proto3 varint (wire type 0) field, OMITTING it when the
// value is zero — the proto3 default-elision the Rust producer and the Python
// subscriber both rely on (an absent scalar decodes as 0).
func addVarint(b []byte, field int, v uint64) []byte {
	if v == 0 {
		return b
	}
	b = binary.AppendUvarint(b, tagKey(field, 0))
	return binary.AppendUvarint(b, v)
}

// addI64 appends an int64 field: prost encodes int64 as the plain varint of its
// two's-complement bit pattern (a negative value is a 10-byte varint), so the
// bit-cast to uint64 reproduces the Rust encoding exactly.
func addI64(b []byte, field int, v int64) []byte {
	return addVarint(b, field, uint64(v))
}

// addSub appends a length-delimited submessage (wire type 2). Unlike scalars a
// submessage is ALWAYS emitted — a selected oneof variant carries its tag and
// length even when its own body is empty (a zero heartbeat is tag 5, length 0),
// and each repeated Level is one submessage.
func addSub(b []byte, field int, sub []byte) []byte {
	b = binary.AppendUvarint(b, tagKey(field, 2))
	b = binary.AppendUvarint(b, uint64(len(sub)))
	return append(b, sub...)
}

// frame prepends the 4-byte big-endian length prefix to an MdFrame body:
// [len:u32 BE][MdFrame protobuf bytes]. The WebSocket transport already
// delimits messages, so the prefix is redundant here, but it is kept for parity
// with the real producer and to keep the frame format transport-agnostic.
func frame(body []byte) []byte {
	out := make([]byte, 4, 4+len(body))
	binary.BigEndian.PutUint32(out, uint32(len(body)))
	return append(out, body...)
}
