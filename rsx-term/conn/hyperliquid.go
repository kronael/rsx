// This file is the Hyperliquid READ-ONLY market-data source — the second
// backend behind the terminal's generic venue seam (feed.VenueMsg + the
// normalized wire.Snapshot/MdTrade shapes; conn/live.go is the RSX backend,
// conn/mock.go the offline one). It dials wss://api.hyperliquid.xyz/ws,
// subscribes l2Book + trades per coin, and maps HL's JSON into the SAME
// folds the RSX feed drives. Universe + size decimals come from POST /info
// {"type":"meta"}.
//
// TRADING on Hyperliquid is deliberately NOT wired: orders need EIP-712
// signing with an ETH key + nonce management. The venue registers with a nil
// Submitter, which the UI surfaces as an honest "read-only venue" block.
// TODO(hl-trading): implement an HL Submitter (EIP-712 order signing) when
// the founder wants to trade there, not just watch.
package conn

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"sync/atomic"
	"time"

	"github.com/coder/websocket"

	"rsx-term/feed"
	"rsx-term/wire"
)

// HLVenueName is the venue tag on every message this source emits.
const HLVenueName = "hyperliquid"

// hlWsURL / hlInfoURL are Hyperliquid's public endpoints.
const hlWsURL = "wss://api.hyperliquid.xyz/ws"
const hlInfoURL = "https://api.hyperliquid.xyz/info"

// hlMaxDecimals is Hyperliquid's perp price precision rule: prices carry at
// most (6 - szDecimals) decimal places.
const hlMaxDecimals = 6

// hlPingInterval keeps the WS alive (HL drops silent connections at ~60s).
const hlPingInterval = 30 * time.Second

// HLAsset is one entry of the /info meta universe.
type HLAsset struct {
	Name        string `json:"name"`
	SzDecimals  int    `json:"szDecimals"`
	OnlyIsolate bool   `json:"onlyIsolated"`
}

// hlMeta is the /info {"type":"meta"} response envelope.
type hlMeta struct {
	Universe []HLAsset `json:"universe"`
}

// HLInstrument is a normalized HL asset: the venue-local symbol id the
// terminal keys markets by, plus display precision. PriceDec follows HL's
// rule (6 - szDecimals, floored at 0); Tick is one raw price unit.
type HLInstrument struct {
	ID       uint32
	Coin     string
	PriceDec int
	QtyDec   int
}

// DecodeHLMeta parses the /info meta body into ordered instruments
// (id = 1 + universe index, so ids are stable per session).
func DecodeHLMeta(body []byte) ([]HLInstrument, error) {
	var meta hlMeta
	if err := json.Unmarshal(body, &meta); err != nil {
		return nil, fmt.Errorf("hl meta: %w", err)
	}
	out := make([]HLInstrument, 0, len(meta.Universe))
	for i, a := range meta.Universe {
		pxDec := hlMaxDecimals - a.SzDecimals
		if pxDec < 0 {
			pxDec = 0
		}
		out = append(out, HLInstrument{
			ID:       uint32(i + 1),
			Coin:     a.Name,
			PriceDec: pxDec,
			QtyDec:   a.SzDecimals,
		})
	}
	return out, nil
}

// FetchHLMeta POSTs /info {"type":"meta"} and decodes the perp universe.
func FetchHLMeta() ([]HLInstrument, error) {
	client := &http.Client{Timeout: fetchTimeout}
	resp, err := client.Post(hlInfoURL, "application/json", bytes.NewBufferString(`{"type":"meta"}`))
	if err != nil {
		return nil, fmt.Errorf("hl meta fetch: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("hl meta fetch: status %d", resp.StatusCode)
	}
	var buf bytes.Buffer
	if _, err := buf.ReadFrom(resp.Body); err != nil {
		return nil, fmt.Errorf("hl meta read: %w", err)
	}
	return DecodeHLMeta(buf.Bytes())
}

// ParseHLFixed parses Hyperliquid's decimal price/size strings into raw
// fixed-point i64 at dec decimals — integer math, no floats. Excess
// fractional digits truncate (HL never exceeds its own precision rule, but
// a lying frame must not corrupt the fold).
func ParseHLFixed(s string, dec int) (int64, bool) {
	if s == "" {
		return 0, false
	}
	intPart, fracPart := s, ""
	if i := strings.IndexByte(s, '.'); i >= 0 {
		intPart, fracPart = s[:i], s[i+1:]
	}
	if len(fracPart) > dec {
		fracPart = fracPart[:dec]
	}
	for len(fracPart) < dec {
		fracPart += "0"
	}
	digits := intPart + fracPart
	var out int64
	for i := 0; i < len(digits); i++ {
		c := digits[i]
		if c < '0' || c > '9' {
			return 0, false
		}
		next := out*10 + int64(c-'0')
		if next < out {
			return 0, false // overflow
		}
		out = next
	}
	return out, true
}

// hlLevel is one l2Book level: {"px":"42999.0","sz":"0.5","n":3}.
type hlLevel struct {
	Px string `json:"px"`
	Sz string `json:"sz"`
	N  uint32 `json:"n"`
}

// hlBook is the l2Book channel payload.
type hlBook struct {
	Coin   string       `json:"coin"`
	TimeMs int64        `json:"time"`
	Levels [2][]hlLevel `json:"levels"` // [bids, asks]
}

// hlTrade is one trades channel entry. Side: "B" = buy aggressor, "A" = sell.
type hlTrade struct {
	Coin   string `json:"coin"`
	Side   string `json:"side"`
	Px     string `json:"px"`
	Sz     string `json:"sz"`
	TimeMs int64  `json:"time"`
	Tid    uint64 `json:"tid"`
}

// hlFrame is the WS message envelope: {"channel":"l2Book"|"trades"|"pong"...}.
type hlFrame struct {
	Channel string          `json:"channel"`
	Data    json.RawMessage `json:"data"`
}

// HL is the Hyperliquid market-data source: an instrument table plus a
// background reader that emits venue-tagged normalized frames on Events().
// Construction does NOT dial; Start does (opt-in, off the render path).
type HL struct {
	instruments []HLInstrument
	byCoin      map[string]HLInstrument
	events      chan any
	closed      atomic.Bool
	active      atomic.Pointer[websocket.Conn]
}

// NewHL builds the source over a fetched universe, watching only the listed
// coins (empty = all).
func NewHL(instruments []HLInstrument, coins []string) *HL {
	keep := map[string]bool{}
	for _, c := range coins {
		keep[strings.ToUpper(strings.TrimSpace(c))] = true
	}
	h := &HL{byCoin: map[string]HLInstrument{}, events: make(chan any, 256)}
	for _, ins := range instruments {
		if len(keep) > 0 && !keep[strings.ToUpper(ins.Coin)] {
			continue
		}
		h.instruments = append(h.instruments, ins)
		h.byCoin[ins.Coin] = ins
	}
	return h
}

// Instruments returns the watched universe.
func (h *HL) Instruments() []HLInstrument { return h.instruments }

// Events is the venue-tagged message stream (feed.VenueMsg / VenueUp/Down).
func (h *HL) Events() <-chan any { return h.events }

// Start launches the named background reader. It never blocks the caller:
// dial failures emit feed.VenueDown and retry with bounded backoff until ctx
// ends or Close is called.
func (h *HL) Start(ctx context.Context) {
	go h.readLoop(ctx)
}

// Close stops the reader and interrupts a blocked read: it marks the source
// closed (so the reconnect loop won't redial) and CloseNows the active socket
// so an idle Read returns immediately — mirroring LiveGateway.Close, safe to
// wire to a live venue-off toggle.
func (h *HL) Close() {
	h.closed.Store(true)
	if c := h.active.Load(); c != nil {
		_ = c.CloseNow()
	}
}

// readLoop dials, subscribes, and folds frames until the source closes; on
// any error it reports VenueDown and redials with bounded backoff. The
// terminal renders fine with this loop dead — the venue just stays empty.
func (h *HL) readLoop(ctx context.Context) {
	backoff := time.Duration(0)
	for {
		if h.closed.Load() || ctx.Err() != nil {
			return
		}
		conn, err := h.dialAndSubscribe(ctx)
		if err != nil {
			h.emit(feed.VenueDown{Venue: HLVenueName})
			backoff = nextBackoff(backoff)
			if !sleepBackoff(ctx, backoff) {
				return
			}
			continue
		}
		backoff = 0
		h.active.Store(conn)
		h.emit(feed.VenueUp{Venue: HLVenueName})
		pingCtx, stopPing := context.WithCancel(ctx)
		go h.pingLoop(pingCtx, conn)
		h.drain(ctx, conn)
		stopPing()
		_ = conn.CloseNow()
		h.active.Store(nil)
		if h.closed.Load() || ctx.Err() != nil {
			return
		}
		h.emit(feed.VenueDown{Venue: HLVenueName})
	}
}

// dialAndSubscribe opens the WS and subscribes l2Book + trades per coin.
func (h *HL) dialAndSubscribe(ctx context.Context) (*websocket.Conn, error) {
	conn, _, err := websocket.Dial(ctx, hlWsURL, nil)
	if err != nil {
		return nil, err
	}
	for _, ins := range h.instruments {
		for _, kind := range []string{"l2Book", "trades"} {
			frame := fmt.Sprintf(`{"method":"subscribe","subscription":{"type":%q,"coin":%q}}`, kind, ins.Coin)
			wctx, cancel := context.WithTimeout(ctx, writeTimeout)
			err := conn.Write(wctx, websocket.MessageText, []byte(frame))
			cancel()
			if err != nil {
				_ = conn.CloseNow()
				return nil, err
			}
		}
	}
	return conn, nil
}

// pingLoop keeps the socket alive ({"method":"ping"} every hlPingInterval).
func (h *HL) pingLoop(ctx context.Context, conn *websocket.Conn) {
	ticker := time.NewTicker(hlPingInterval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			wctx, cancel := context.WithTimeout(ctx, writeTimeout)
			err := conn.Write(wctx, websocket.MessageText, []byte(`{"method":"ping"}`))
			cancel()
			if err != nil {
				return // the read loop observes the drop and reconnects
			}
		}
	}
}

// drain reads until the socket errors, emitting each decoded frame.
func (h *HL) drain(ctx context.Context, conn *websocket.Conn) {
	for {
		_, data, err := conn.Read(ctx)
		if err != nil {
			return
		}
		for _, msg := range h.DecodeHLMessage(data) {
			h.emit(msg)
		}
	}
}

// emit forwards without ever blocking the reader: if the UI stalls long
// enough to fill the buffer, market data drops (books recover on the next
// snapshot — HL l2Book frames are full snapshots).
func (h *HL) emit(msg any) {
	select {
	case h.events <- msg:
	default:
	}
}

// DecodeHLMessage maps one WS frame to venue-tagged normalized messages
// (possibly several — a trades frame batches prints). Unknown channels and
// unknown coins decode to nothing; a malformed frame is skipped, never fatal.
func (h *HL) DecodeHLMessage(data []byte) []any {
	var frame hlFrame
	if err := json.Unmarshal(data, &frame); err != nil {
		return nil
	}
	switch frame.Channel {
	case "l2Book":
		var bk hlBook
		if err := json.Unmarshal(frame.Data, &bk); err != nil {
			return nil
		}
		ins, ok := h.byCoin[bk.Coin]
		if !ok {
			return nil
		}
		snap := wire.Snapshot{
			SymbolID: ins.ID,
			Bids:     h.levels(bk.Levels[0], ins),
			Asks:     h.levels(bk.Levels[1], ins),
			TsNs:     uint64(bk.TimeMs) * 1_000_000,
			Seq:      uint64(bk.TimeMs),
		}
		return []any{feed.VenueMsg{Venue: HLVenueName, Msg: snap}}
	case "trades":
		var trades []hlTrade
		if err := json.Unmarshal(frame.Data, &trades); err != nil {
			return nil
		}
		var out []any
		for _, tr := range trades {
			ins, ok := h.byCoin[tr.Coin]
			if !ok {
				continue
			}
			px, okPx := ParseHLFixed(tr.Px, ins.PriceDec)
			qty, okQty := ParseHLFixed(tr.Sz, ins.QtyDec)
			if !okPx || !okQty {
				continue
			}
			taker := uint32(0) // "B" = buy aggressor
			if tr.Side == "A" {
				taker = 1
			}
			out = append(out, feed.VenueMsg{Venue: HLVenueName, Msg: wire.MdTrade{
				SymbolID:  ins.ID,
				Px:        px,
				Qty:       qty,
				TakerSide: taker,
				TsNs:      uint64(tr.TimeMs) * 1_000_000,
				Seq:       tr.Tid,
			}})
		}
		return out
	default: // pong, subscriptionResponse, …
		return nil
	}
}

// levels maps one side of an l2Book frame, skipping unparseable entries.
func (h *HL) levels(raw []hlLevel, ins HLInstrument) []wire.Level {
	out := make([]wire.Level, 0, len(raw))
	for _, l := range raw {
		px, okPx := ParseHLFixed(l.Px, ins.PriceDec)
		qty, okQty := ParseHLFixed(l.Sz, ins.QtyDec)
		if !okPx || !okQty || qty <= 0 {
			continue
		}
		out = append(out, wire.Level{Px: px, Qty: qty, Count: l.N})
	}
	return out
}

// DefaultHLCoins is the curated default watch (breadth without subscribing
// all ~150 perps). RSX_TERM_HL_COINS overrides; "all" watches everything.
var DefaultHLCoins = []string{
	"BTC", "ETH", "SOL", "DOGE", "XRP", "AVAX", "LINK", "OP", "ARB", "SUI",
	"APT", "SEI", "TIA", "WIF", "PEPE", "BNB", "ADA", "LTC", "NEAR", "INJ",
	"AAVE", "CRV", "LDO", "JUP",
}

// SectorOf maps a coin to its market-map sector. HL carries no sector
// metadata, so this is a small static table (news view grouping); unknown
// coins land in "other".
func SectorOf(coin string) string {
	switch strings.ToUpper(coin) {
	case "BTC", "ETH", "BNB", "SOL", "XRP", "DOGE", "ADA", "LTC":
		return "majors"
	case "AVAX", "NEAR", "APT", "SUI", "SEI", "TIA", "TON", "TRX", "DOT", "ATOM", "INJ":
		return "L1"
	case "OP", "ARB", "STRK", "ZK", "MATIC", "POL", "MANTA", "BLAST":
		return "L2"
	case "AAVE", "CRV", "LDO", "UNI", "MKR", "COMP", "SNX", "SUSHI", "JUP", "PENDLE", "ENA":
		return "defi"
	case "WIF", "PEPE", "BONK", "SHIB", "FLOKI", "MEME", "POPCAT", "BRETT", "MOG", "PENGU":
		return "meme"
	case "FET", "RNDR", "TAO", "WLD", "AI16Z", "VIRTUAL":
		return "ai"
	default:
		return "other"
	}
}
