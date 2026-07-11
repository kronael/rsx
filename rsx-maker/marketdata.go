package main

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"errors"
	"log"
	"time"

	"github.com/coder/websocket"
)

// channelBBO is the marketdata subscription bitmask for BBO frames.
// The server matches rsx-marketdata's CHANNEL_BBO = 1.
const channelBBO = 1

var errTruncated = errors.New("protobuf: truncated frame")
var errUnsupportedWire = errors.New("protobuf: unsupported wire type")

// runMarketdata subscribes to the BBO channel for every configured
// symbol and feeds mids into src until ctx is cancelled. It reconnects
// with capped exponential backoff so a transient marketdata restart
// does not kill the maker — quoting falls back to the mark seam (and
// then defaults) while the feed is down.
func runMarketdata(ctx context.Context, cfg Config, src *bboSource) {
	delay := time.Second
	const maxDelay = 16 * time.Second
	for ctx.Err() == nil {
		if err := readMarketdata(ctx, cfg, src); err != nil && ctx.Err() == nil {
			log.Printf("marketdata: %v (retry in %s)", err, delay)
		}
		select {
		case <-ctx.Done():
			return
		case <-time.After(delay):
		}
		delay *= 2
		if delay > maxDelay {
			delay = maxDelay
		}
	}
}

// readMarketdata holds one marketdata connection: subscribe, then loop
// decoding BBO frames until the connection or context ends.
func readMarketdata(ctx context.Context, cfg Config, src *bboSource) error {
	conn, _, err := websocket.Dial(ctx, cfg.MarketdataWS, nil)
	if err != nil {
		return err
	}
	defer conn.CloseNow()

	for _, sid := range cfg.Symbols {
		sub, _ := json.Marshal(map[string][]int{"S": {int(sid), channelBBO}})
		if err := conn.Write(ctx, websocket.MessageText, sub); err != nil {
			return err
		}
	}

	for ctx.Err() == nil {
		typ, data, err := conn.Read(ctx)
		if err != nil {
			return err
		}
		if typ != websocket.MessageBinary {
			continue
		}
		if sym, bidPx, askPx, ok := decodeBBO(data); ok {
			src.update(sym, bidPx, askPx)
		}
	}
	return ctx.Err()
}

// decodeBBO parses one length-prefixed MdFrame and returns the BBO
// symbol, bid price and ask price if the frame carries a Bbo message.
// It is a minimal port of rsx-playground/md_wire.py: only the fields
// the maker needs (Bbo tag 1: symbol=1, bid_px=2, ask_px=5) are read;
// all other frame types are skipped.
//
// Wire: [len:u32 big-endian][MdFrame body]. MdFrame is a proto3 oneof;
// the set field's number selects the message type. proto3 omits
// zero-valued scalars, so an absent field decodes as 0.
func decodeBBO(data []byte) (symbol uint32, bidPx, askPx int64, ok bool) {
	if len(data) < 4 {
		return 0, 0, 0, false
	}
	bodyLen := int(binary.BigEndian.Uint32(data[:4]))
	if 4+bodyLen > len(data) {
		return 0, 0, 0, false
	}
	body := data[4 : 4+bodyLen]

	fields, err := protoFields(body)
	if err != nil {
		return 0, 0, 0, false
	}
	bbo, present := fields[1] // MdFrame.bbo
	if !present {
		return 0, 0, 0, false
	}
	inner, err := protoFields(bbo)
	if err != nil {
		return 0, 0, 0, false
	}
	sym, _ := protoVarint(inner, 1)
	bid, _ := protoVarint(inner, 2)
	ask, _ := protoVarint(inner, 5)
	return uint32(sym), int64(bid), int64(ask), true
}

// protoFields parses a protobuf message body into a map of field
// number to raw bytes, keeping only the last occurrence of each field.
// Handles varint (wire type 0) and length-delimited (wire type 2) —
// the only two wire types the marketdata schema uses.
func protoFields(buf []byte) (map[int][]byte, error) {
	out := make(map[int][]byte)
	i := 0
	for i < len(buf) {
		key, n := binary.Uvarint(buf[i:])
		if n <= 0 {
			return nil, errTruncated
		}
		i += n
		field := int(key >> 3)
		switch key & 0x7 {
		case 0: // varint
			start := i
			_, n := binary.Uvarint(buf[i:])
			if n <= 0 {
				return nil, errTruncated
			}
			i += n
			out[field] = buf[start:i]
		case 2: // length-delimited
			length, n := binary.Uvarint(buf[i:])
			if n <= 0 {
				return nil, errTruncated
			}
			i += n
			if i+int(length) > len(buf) {
				return nil, errTruncated
			}
			out[field] = buf[i : i+int(length)]
			i += int(length)
		default:
			return nil, errUnsupportedWire
		}
	}
	return out, nil
}

// protoVarint decodes the varint stored for a field, or 0 if absent.
func protoVarint(fields map[int][]byte, field int) (uint64, bool) {
	raw, ok := fields[field]
	if !ok {
		return 0, false
	}
	v, n := binary.Uvarint(raw)
	if n <= 0 {
		return 0, false
	}
	return v, true
}
