package main

import (
	"encoding/binary"
	"testing"
)

// encodeBBOFrame builds a length-prefixed MdFrame carrying a Bbo
// message, mirroring the rsx-marketdata producer, to exercise decodeBBO.
func encodeBBOFrame(sym uint32, bidPx, askPx int64) []byte {
	var inner []byte
	inner = appendVarintField(inner, 1, uint64(sym))   // Bbo.symbol
	inner = appendVarintField(inner, 2, uint64(bidPx)) // Bbo.bid_px
	inner = appendVarintField(inner, 3, 1)             // Bbo.bid_qty (ignored)
	inner = appendVarintField(inner, 5, uint64(askPx)) // Bbo.ask_px

	var body []byte
	body = appendLenField(body, 1, inner) // MdFrame.bbo

	frame := make([]byte, 4+len(body))
	binary.BigEndian.PutUint32(frame[:4], uint32(len(body)))
	copy(frame[4:], body)
	return frame
}

func appendVarintField(dst []byte, field int, v uint64) []byte {
	dst = binary.AppendUvarint(dst, uint64(field)<<3|0)
	return binary.AppendUvarint(dst, v)
}

func appendLenField(dst []byte, field int, payload []byte) []byte {
	dst = binary.AppendUvarint(dst, uint64(field)<<3|2)
	dst = binary.AppendUvarint(dst, uint64(len(payload)))
	return append(dst, payload...)
}

func TestDecodeBBORoundTrip(t *testing.T) {
	frame := encodeBBOFrame(10, 49900, 50100)
	sym, bid, ask, ok := decodeBBO(frame)
	if !ok {
		t.Fatal("decodeBBO returned ok=false")
	}
	if sym != 10 {
		t.Errorf("sym = %d, want 10", sym)
	}
	if bid != 49900 {
		t.Errorf("bid = %d, want 49900", bid)
	}
	if ask != 50100 {
		t.Errorf("ask = %d, want 50100", ask)
	}
}

func TestDecodeBBORejectsShort(t *testing.T) {
	if _, _, _, ok := decodeBBO([]byte{0x00, 0x01}); ok {
		t.Error("expected ok=false for short frame")
	}
}

func TestDecodeBBONonBBOFrame(t *testing.T) {
	// A frame whose only field is MdFrame.heartbeat (tag 5) carries
	// no Bbo, so decodeBBO must abstain.
	var body []byte
	body = appendLenField(body, 5, appendVarintField(nil, 1, 123))
	frame := make([]byte, 4+len(body))
	binary.BigEndian.PutUint32(frame[:4], uint32(len(body)))
	copy(frame[4:], body)
	if _, _, _, ok := decodeBBO(frame); ok {
		t.Error("expected ok=false for non-BBO frame")
	}
}

func TestBBOSourceUpdateAndRef(t *testing.T) {
	src := newBBOSource()
	if _, ok := src.Ref(10); ok {
		t.Error("empty source should abstain")
	}
	src.update(10, 100, 200)
	if px, ok := src.Ref(10); !ok || px != 150 {
		t.Errorf("Ref = (%d,%v), want (150,true)", px, ok)
	}
	// A one-sided (zero) book must not overwrite the last good mid.
	src.update(10, 0, 200)
	if px, _ := src.Ref(10); px != 150 {
		t.Errorf("mid changed on empty side: %d, want 150", px)
	}
}
