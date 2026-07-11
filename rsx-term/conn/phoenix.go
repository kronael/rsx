// This file is the Phoenix READ-ONLY market-data source — a third backend
// behind the terminal's generic venue seam (feed.VenueMsg + the normalized
// wire.Snapshot/MdTrade shapes; conn/live.go is the RSX backend, conn/mock.go
// the offline one, conn/hyperliquid.go the HL one). It dials
// wss://perp-api.phoenix.trade/v1/ws, subscribes the orderbook channel per
// symbol, and maps Phoenix's JSON into the SAME folds the RSX feed drives.
// Universe + size decimals come from GET /v1/view/exchange/markets.
//
// Phoenix is an on-chain Solana perp DEX. TRADING is deliberately NOT wired:
// orders settle on-chain and need a wallet signer. The venue registers with a
// nil Submitter, which the UI surfaces as an honest "read-only venue" block.
// TODO(phoenix-trading): implement a Phoenix Submitter (Solana wallet order
// signing) when the founder wants to trade there, not just watch.
//
// TODO(phoenix-trades): the trades channel carries no explicit price — each
// print is {side, baseAmount, quoteAmount, ...} and the price is
// quoteAmount/baseAmount. Deriving that as fixed-point i64 without floats needs
// a scaled integer division (px_raw = quoteFixed * 10^PriceDec / baseFixed)
// that overflows int64 for large notionals and rounds lossily, so this source
// is orderbook-only for now — the required core. Add the tape once the
// division is done cleanly (math/big off this off-path reader is acceptable).
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

// PhoenixVenueName is the venue tag on every message this source emits.
const PhoenixVenueName = "phoenix"

// phxWsURL / phxMarketsURL are Phoenix's public endpoints.
const phxWsURL = "wss://perp-api.phoenix.trade/v1/ws"
const phxMarketsURL = "https://perp-api.phoenix.trade/v1/view/exchange/markets"

// phxPriceDec is the read-only display precision for every Phoenix symbol. The
// WS already sends decimal prices, so this is a fixed sensible display default
// (ParseHLFixed truncates any excess fractional digits), NOT a per-symbol rule.
const phxPriceDec = 4

// phxMaxQtyDec caps the per-symbol size precision derived from baseLotsDecimals.
const phxMaxQtyDec = 8

// phxPingInterval keeps the WS alive. Phoenix documents no heartbeat; a ping
// the server ignores is harmless (mirrors HL's keep-alive).
const phxPingInterval = 30 * time.Second

// PhxMarket is one entry of the /v1/view/exchange/markets response — a bare
// JSON array of these (the display name rides under a nested "metadata" object
// this decoder skips; the symbol is the venue key). Only the four fields the
// terminal needs are decoded.
type PhxMarket struct {
	Symbol           string `json:"symbol"`
	TickSize         int    `json:"tickSize"`
	BaseLotsDecimals int    `json:"baseLotsDecimals"`
	AssetID          int    `json:"assetId"`
}

// PhxInstrument is a normalized Phoenix market: the venue-local symbol id the
// terminal keys markets by, plus display precision. PriceDec is the fixed
// read-only default (phxPriceDec); QtyDec is baseLotsDecimals clamped to
// [0, phxMaxQtyDec]; Tick is one raw price unit.
//
// ID is assetId + 1, NOT the raw assetId: the terminal reserves symbol id 0 as
// "unspecified -> primary" (see the ui frameSymbol helper), and Phoenix's
// assetId 0 (SOL) would collide with that sentinel and misroute. HL reserves 0
// the same way (its ids are 1-based); this mirrors that discipline. The wire
// subscribe keys on the symbol string, so the offset is purely internal.
type PhxInstrument struct {
	ID       uint32
	Symbol   string
	PriceDec int
	QtyDec   int
}

// DecodePhxMarkets parses the markets body (a bare JSON array) into instruments
// (id = assetId + 1, PriceDec = phxPriceDec, QtyDec = baseLotsDecimals clamped
// to [0, 8]).
func DecodePhxMarkets(body []byte) ([]PhxInstrument, error) {
	var markets []PhxMarket
	if err := json.Unmarshal(body, &markets); err != nil {
		return nil, fmt.Errorf("phoenix markets: %w", err)
	}
	out := make([]PhxInstrument, 0, len(markets))
	for _, m := range markets {
		qtyDec := m.BaseLotsDecimals
		if qtyDec < 0 {
			qtyDec = 0
		}
		if qtyDec > phxMaxQtyDec {
			qtyDec = phxMaxQtyDec
		}
		out = append(out, PhxInstrument{
			ID:       uint32(m.AssetID) + 1,
			Symbol:   m.Symbol,
			PriceDec: phxPriceDec,
			QtyDec:   qtyDec,
		})
	}
	return out, nil
}

// FetchPhxMarkets GETs the markets endpoint and decodes the perp universe.
func FetchPhxMarkets() ([]PhxInstrument, error) {
	client := &http.Client{Timeout: fetchTimeout}
	resp, err := client.Get(phxMarketsURL)
	if err != nil {
		return nil, fmt.Errorf("phoenix markets fetch: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("phoenix markets fetch: status %d", resp.StatusCode)
	}
	var buf bytes.Buffer
	if _, err := buf.ReadFrom(resp.Body); err != nil {
		return nil, fmt.Errorf("phoenix markets read: %w", err)
	}
	return DecodePhxMarkets(buf.Bytes())
}

// phxLevel is one orderbook level: [price, size] as JSON numbers. json.Number
// captures each literal as its decimal string so ParseHLFixed folds it to
// fixed-point i64 without a float ever touching the wire value. A short array
// (malformed frame) zero-fills to "", which ParseHLFixed rejects.
type phxLevel [2]json.Number

// phxOrderbook is the orderbook channel payload: a FULL L2 snapshot each time.
type phxOrderbook struct {
	Bids []phxLevel  `json:"bids"`
	Asks []phxLevel  `json:"asks"`
	Mid  json.Number `json:"mid"`
}

// phxFrame is the WS message envelope. The payload rides inline on the same
// object as the channel/symbol tags (no HL-style nested "data").
type phxFrame struct {
	Channel   string        `json:"channel"`
	Symbol    string        `json:"symbol"`
	Orderbook *phxOrderbook `json:"orderbook"`
}

// Phoenix is the Phoenix market-data source: an instrument table plus a
// background reader that emits venue-tagged normalized frames on Events().
// Construction does NOT dial; Start does (opt-in, off the render path).
type Phoenix struct {
	instruments []PhxInstrument
	bySymbol    map[string]PhxInstrument
	events      chan any
	closed      atomic.Bool
	active      atomic.Pointer[websocket.Conn]
}

// NewPhoenix builds the source over a fetched universe, watching only the
// listed symbols (empty = all).
func NewPhoenix(instruments []PhxInstrument, symbols []string) *Phoenix {
	keep := map[string]bool{}
	for _, s := range symbols {
		keep[strings.ToUpper(strings.TrimSpace(s))] = true
	}
	p := &Phoenix{bySymbol: map[string]PhxInstrument{}, events: make(chan any, 256)}
	for _, ins := range instruments {
		if len(keep) > 0 && !keep[strings.ToUpper(ins.Symbol)] {
			continue
		}
		p.instruments = append(p.instruments, ins)
		p.bySymbol[ins.Symbol] = ins
	}
	return p
}

// Instruments returns the watched universe.
func (p *Phoenix) Instruments() []PhxInstrument { return p.instruments }

// Events is the venue-tagged message stream (feed.VenueMsg / VenueUp/Down).
func (p *Phoenix) Events() <-chan any { return p.events }

// Start launches the named background reader. It never blocks the caller: dial
// failures emit feed.VenueDown and retry with bounded backoff until ctx ends or
// Close is called.
func (p *Phoenix) Start(ctx context.Context) {
	go p.readLoop(ctx)
}

// Close stops the reader and interrupts a blocked read: it marks the source
// closed (so the reconnect loop won't redial) and CloseNows the active socket
// so an idle Read returns immediately — mirroring HL.Close.
func (p *Phoenix) Close() {
	p.closed.Store(true)
	if c := p.active.Load(); c != nil {
		_ = c.CloseNow()
	}
}

// readLoop dials, subscribes, and folds frames until the source closes; on any
// error it reports VenueDown and redials with bounded backoff. The terminal
// renders fine with this loop dead — the venue just stays empty.
func (p *Phoenix) readLoop(ctx context.Context) {
	backoff := time.Duration(0)
	for {
		if p.closed.Load() || ctx.Err() != nil {
			return
		}
		conn, err := p.dialAndSubscribe(ctx)
		if err != nil {
			p.emit(feed.VenueDown{Venue: PhoenixVenueName})
			backoff = nextBackoff(backoff)
			if !sleepBackoff(ctx, backoff) {
				return
			}
			continue
		}
		backoff = 0
		p.active.Store(conn)
		p.emit(feed.VenueUp{Venue: PhoenixVenueName})
		pingCtx, stopPing := context.WithCancel(ctx)
		go p.pingLoop(pingCtx, conn)
		p.drain(ctx, conn)
		stopPing()
		_ = conn.CloseNow()
		p.active.Store(nil)
		if p.closed.Load() || ctx.Err() != nil {
			return
		}
		p.emit(feed.VenueDown{Venue: PhoenixVenueName})
	}
}

// dialAndSubscribe opens the WS and subscribes the orderbook channel per
// symbol. (Trades are not subscribed — see TODO(phoenix-trades).)
func (p *Phoenix) dialAndSubscribe(ctx context.Context) (*websocket.Conn, error) {
	conn, _, err := websocket.Dial(ctx, phxWsURL, nil)
	if err != nil {
		return nil, err
	}
	for _, ins := range p.instruments {
		frame := fmt.Sprintf(`{"type":"subscribe","subscription":{"channel":"orderbook","symbol":%q}}`, ins.Symbol)
		wctx, cancel := context.WithTimeout(ctx, writeTimeout)
		err := conn.Write(wctx, websocket.MessageText, []byte(frame))
		cancel()
		if err != nil {
			_ = conn.CloseNow()
			return nil, err
		}
	}
	return conn, nil
}

// pingLoop keeps the socket alive ({"type":"ping"} every phxPingInterval).
func (p *Phoenix) pingLoop(ctx context.Context, conn *websocket.Conn) {
	ticker := time.NewTicker(phxPingInterval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			wctx, cancel := context.WithTimeout(ctx, writeTimeout)
			err := conn.Write(wctx, websocket.MessageText, []byte(`{"type":"ping"}`))
			cancel()
			if err != nil {
				return // the read loop observes the drop and reconnects
			}
		}
	}
}

// drain reads until the socket errors, emitting each decoded frame.
func (p *Phoenix) drain(ctx context.Context, conn *websocket.Conn) {
	for {
		_, data, err := conn.Read(ctx)
		if err != nil {
			return
		}
		for _, msg := range p.DecodePhxMessage(data) {
			p.emit(msg)
		}
	}
}

// emit forwards without ever blocking the reader: if the UI stalls long enough
// to fill the buffer, market data drops (books recover on the next snapshot —
// Phoenix orderbook frames are full snapshots).
func (p *Phoenix) emit(msg any) {
	select {
	case p.events <- msg:
	default:
	}
}

// DecodePhxMessage maps one WS frame to venue-tagged normalized messages.
// Unknown channels and unknown symbols decode to nothing; a malformed frame is
// skipped, never fatal. Trades frames currently decode to nothing (orderbook-
// only, see TODO(phoenix-trades)).
func (p *Phoenix) DecodePhxMessage(data []byte) []any {
	var frame phxFrame
	if err := json.Unmarshal(data, &frame); err != nil {
		return nil
	}
	switch frame.Channel {
	case "orderbook":
		if frame.Orderbook == nil {
			return nil
		}
		ins, ok := p.bySymbol[frame.Symbol]
		if !ok {
			return nil
		}
		snap := wire.Snapshot{
			SymbolID: ins.ID,
			Bids:     p.levels(frame.Orderbook.Bids, ins),
			Asks:     p.levels(frame.Orderbook.Asks, ins),
		}
		return []any{feed.VenueMsg{Venue: PhoenixVenueName, Msg: snap}}
	default: // trades (TODO(phoenix-trades)), subscribe acks, pong…
		return nil
	}
}

// levels maps one side of an orderbook frame, skipping unparseable entries.
// Phoenix carries no per-level order count, so Count stays 0.
func (p *Phoenix) levels(raw []phxLevel, ins PhxInstrument) []wire.Level {
	out := make([]wire.Level, 0, len(raw))
	for _, l := range raw {
		px, okPx := ParseHLFixed(string(l[0]), ins.PriceDec)
		qty, okQty := ParseHLFixed(string(l[1]), ins.QtyDec)
		if !okPx || !okQty || qty <= 0 {
			continue
		}
		out = append(out, wire.Level{Px: px, Qty: qty})
	}
	return out
}

// DefaultPhoenixSymbols is the curated default watch (breadth without
// subscribing all 50+ markets). RSX_TERM_PHX_SYMBOLS overrides; "all" watches
// everything.
var DefaultPhoenixSymbols = []string{
	"SOL", "BTC", "ETH", "XRP", "HYPE", "DOGE", "BNB", "SUI", "JUP", "WLD",
}
