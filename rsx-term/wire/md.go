package wire

import "fmt"

// This file hand-rolls a decoder for the public marketdata feed's
// wire schema, rsx-marketdata/marketdata.proto. The schema is
// deliberately NOT compiled via protoc/protoc-gen-go on any side —
// the Rust producer (rsx-marketdata/src/wire.rs) and the Python
// subscriber (rsx-playground/md_wire.py) both hand-derive the same
// tags, so this Go decoder does too rather than add a codegen
// toolchain. proto3 semantics: an absent scalar field decodes as 0;
// only wire types 0 (varint) and 2 (length-delimited) appear in this
// schema.

// Level is one aggregated price level (marketdata.proto Level).
type Level struct {
	Px    int64
	Qty   int64
	Count uint32
}

// Bbo is a best-bid/offer update (marketdata.proto Bbo).
type Bbo struct {
	SymbolID uint32
	BidPx    int64
	BidQty   int64
	BidCount uint32
	AskPx    int64
	AskQty   int64
	AskCount uint32
	TsNs     uint64
	Seq      uint64
}

// Snapshot is a full L2 snapshot (marketdata.proto Snapshot).
type Snapshot struct {
	SymbolID uint32
	Bids     []Level
	Asks     []Level
	TsNs     uint64
	Seq      uint64
}

// Delta is a single-level L2 update (marketdata.proto Delta). Side:
// 0 = bid, 1 = ask. Qty == 0 removes the level.
type Delta struct {
	SymbolID uint32
	Side     uint32
	Px       int64
	Qty      int64
	Count    uint32
	TsNs     uint64
	Seq      uint64
}

// MdTrade is a trade print (marketdata.proto Trade). TakerSide:
// 0 = buy, 1 = sell.
type MdTrade struct {
	SymbolID  uint32
	Px        int64
	Qty       int64
	TakerSide uint32
	TsNs      uint64
	Seq       uint64
}

// MdHeartbeat is the server->client liveness ping (marketdata.proto
// Heartbeat).
type MdHeartbeat struct {
	TsMs uint64
}

// field holds one decoded protobuf field: a varint scalar value, or
// the raw bytes of a length-delimited submessage.
type field struct {
	wireType uint64
	scalar   uint64
	bytes    []byte
}

// readVarint reads a base-128 varint starting at buf[i], returning
// the decoded value and the index just past it. Errors on a
// truncated varint (ran off the end of buf before the continuation
// bit cleared).
func readVarint(buf []byte, i int) (uint64, int, error) {
	var result uint64
	var shift uint
	for {
		if i >= len(buf) {
			return 0, 0, fmt.Errorf("wire: truncated varint at offset %d", i)
		}
		b := buf[i]
		i++
		result |= uint64(b&0x7f) << shift
		if b&0x80 == 0 {
			return result, i, nil
		}
		shift += 7
		if shift >= 64 {
			return 0, 0, fmt.Errorf("wire: varint too long (>64 bits)")
		}
	}
}

// asI64 interprets a raw varint as a two's-complement signed int64,
// matching the Rust/Python decoders' sint-free (plain int64) fields.
func asI64(v uint64) int64 {
	return int64(v)
}

// parseFields parses a protobuf message body into field number ->
// []field (a field may repeat, e.g. Snapshot's repeated Level). Only
// wire types 0 (varint) and 2 (length-delimited) are legal in this
// schema; any other wire type is an error, never silently skipped.
func parseFields(buf []byte) (map[uint64][]field, error) {
	out := make(map[uint64][]field)
	i := 0
	n := len(buf)
	for i < n {
		key, next, err := readVarint(buf, i)
		if err != nil {
			return nil, err
		}
		i = next
		fieldNum := key >> 3
		wireType := key & 0x7
		var f field
		f.wireType = wireType
		switch wireType {
		case 0:
			val, next, err := readVarint(buf, i)
			if err != nil {
				return nil, err
			}
			i = next
			f.scalar = val
		case 2:
			length, next, err := readVarint(buf, i)
			if err != nil {
				return nil, err
			}
			i = next
			end := i + int(length)
			if length > uint64(n) || end > n || end < i {
				return nil, fmt.Errorf("wire: length-delimited field %d truncated (want %d bytes at offset %d, have %d)", fieldNum, length, i, n-i)
			}
			f.bytes = buf[i:end]
			i = end
		default:
			return nil, fmt.Errorf("wire: unsupported wire type %d on field %d", wireType, fieldNum)
		}
		out[fieldNum] = append(out[fieldNum], f)
	}
	return out, nil
}

// u returns the last unsigned scalar for tag, or 0 (proto3 default)
// if absent.
func u(f map[uint64][]field, tag uint64) uint64 {
	vs := f[tag]
	if len(vs) == 0 {
		return 0
	}
	return vs[len(vs)-1].scalar
}

// levels decodes every repeated length-delimited Level submessage at
// tag, in wire order.
func levels(f map[uint64][]field, tag uint64) ([]Level, error) {
	fs := f[tag]
	out := make([]Level, 0, len(fs))
	for _, raw := range fs {
		lf, err := parseFields(raw.bytes)
		if err != nil {
			return nil, fmt.Errorf("wire: level: %w", err)
		}
		out = append(out, Level{Px: asI64(u(lf, 1)), Qty: asI64(u(lf, 2)), Count: uint32(u(lf, 3))})
	}
	return out, nil
}

// DecodeMd decodes one WS BINARY frame — [len:u32 BE][MdFrame body] —
// into its oneof variant: Bbo, Snapshot, Delta, MdTrade, or
// MdHeartbeat. Returns an error (never panics) on a truncated frame,
// a truncated varint/body, or an unsupported wire type.
func DecodeMd(frame []byte) (any, error) {
	if len(frame) < 4 {
		return nil, fmt.Errorf("wire: md frame too short (%d bytes, need >= 4)", len(frame))
	}
	bodyLen := int(frame[0])<<24 | int(frame[1])<<16 | int(frame[2])<<8 | int(frame[3])
	if 4+bodyLen > len(frame) {
		return nil, fmt.Errorf("wire: md frame body truncated (want %d bytes, have %d)", bodyLen, len(frame)-4)
	}
	body := frame[4 : 4+bodyLen]

	outer, err := parseFields(body)
	if err != nil {
		return nil, fmt.Errorf("wire: md envelope: %w", err)
	}

	for _, tag := range []uint64{1, 2, 3, 4, 5} {
		fs := outer[tag]
		if len(fs) == 0 {
			continue
		}
		raw := fs[len(fs)-1].bytes
		f, err := parseFields(raw)
		if err != nil {
			return nil, fmt.Errorf("wire: md body tag %d: %w", tag, err)
		}
		switch tag {
		case 1: // Bbo
			return Bbo{
				SymbolID: uint32(u(f, 1)),
				BidPx:    asI64(u(f, 2)),
				BidQty:   asI64(u(f, 3)),
				BidCount: uint32(u(f, 4)),
				AskPx:    asI64(u(f, 5)),
				AskQty:   asI64(u(f, 6)),
				AskCount: uint32(u(f, 7)),
				TsNs:     u(f, 8),
				Seq:      u(f, 9),
			}, nil
		case 2: // Snapshot
			bids, err := levels(f, 2)
			if err != nil {
				return nil, err
			}
			asks, err := levels(f, 3)
			if err != nil {
				return nil, err
			}
			return Snapshot{
				SymbolID: uint32(u(f, 1)),
				Bids:     bids,
				Asks:     asks,
				TsNs:     u(f, 4),
				Seq:      u(f, 5),
			}, nil
		case 3: // Delta
			return Delta{
				SymbolID: uint32(u(f, 1)),
				Side:     uint32(u(f, 2)),
				Px:       asI64(u(f, 3)),
				Qty:      asI64(u(f, 4)),
				Count:    uint32(u(f, 5)),
				TsNs:     u(f, 6),
				Seq:      u(f, 7),
			}, nil
		case 4: // Trade
			return MdTrade{
				SymbolID:  uint32(u(f, 1)),
				Px:        asI64(u(f, 2)),
				Qty:       asI64(u(f, 3)),
				TakerSide: uint32(u(f, 4)),
				TsNs:      u(f, 5),
				Seq:       u(f, 6),
			}, nil
		case 5: // Heartbeat
			return MdHeartbeat{TsMs: u(f, 1)}, nil
		}
	}
	return nil, fmt.Errorf("wire: md frame has no oneof variant set")
}
