package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"os"
	"sync"
	"time"

	"github.com/coder/websocket"
)

// sideBuy and sideSell are the wire side codes (0/1) the gateway expects.
const (
	sideBuy  = 0
	sideSell = 1
	tifGTC   = 0
)

// symbolSpec is the tick and lot size for one symbol, fetched from the
// gateway's /v1/symbols catalog. Prices must align to tick, qty to lot.
type symbolSpec struct {
	tick int64
	lot  int64
}

// Maker quotes both sides at a spread around a reference price and
// cancel-replaces every refresh interval.
type Maker struct {
	cfg    Config
	prices PriceSource

	specs      map[uint32]symbolSpec
	sessionID  string
	orderCount int
	active     []string
}

func newMaker(cfg Config, prices PriceSource) *Maker {
	return &Maker{
		cfg:       cfg,
		prices:    prices,
		specs:     make(map[uint32]symbolSpec),
		sessionID: randomHex(2),
	}
}

// Run drives the maker: wait for the gateway, load symbol specs, then
// cancel-replace quotes every refresh interval until ctx is cancelled.
// On shutdown it cancels all resting orders so the book is left clean.
func (m *Maker) Run(ctx context.Context) {
	if err := m.awaitGateway(ctx); err != nil {
		if ctx.Err() == nil {
			log.Printf("maker: gateway unreachable: %v", err)
		}
		return
	}
	m.fetchSymbols(ctx)

	ticker := time.NewTicker(m.cfg.Refresh)
	defer ticker.Stop()
	for {
		if err := m.quoteCycle(ctx); err != nil && ctx.Err() == nil {
			log.Printf("maker: quote cycle: %v", err)
		}
		select {
		case <-ctx.Done():
			m.cancelAll()
			return
		case <-ticker.C:
		}
	}
}

// dialTimeout bounds each gateway WS handshake so a half-open or stalled dial
// can't wedge the quote loop indefinitely (mirrors the Python maker's
// ClientTimeout=3s, and cancelAll's own timeout). Only the dial is bounded;
// writes on the returned conn use the run ctx.
const dialTimeout = 3 * time.Second

// awaitGateway blocks until a gateway connection succeeds, backing off
// on failure. Returns an error only if ctx is cancelled first.
func (m *Maker) awaitGateway(ctx context.Context) error {
	delay := time.Second
	const maxDelay = 16 * time.Second
	for {
		dctx, cancel := context.WithTimeout(ctx, dialTimeout)
		conn, err := dialGateway(dctx, m.cfg)
		cancel()
		if err == nil {
			conn.Close(websocket.StatusNormalClosure, "")
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(delay):
		}
		delay *= 2
		if delay > maxDelay {
			delay = maxDelay
		}
	}
}

// symbolsResponse mirrors the /v1/symbols JSON catalog.
type symbolsResponse struct {
	Symbols []struct {
		ID       uint32 `json:"id"`
		TickSize int64  `json:"tick_size"`
		LotSize  int64  `json:"lot_size"`
	} `json:"symbols"`
}

// fetchSymbols loads tick/lot sizes from the gateway catalog. On any
// failure every symbol defaults to tick=1, lot=1 (matching the Python
// maker's best-effort fetch).
func (m *Maker) fetchSymbols(ctx context.Context) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, m.cfg.SymbolsURL, nil)
	if err != nil {
		return
	}
	client := &http.Client{Timeout: 3 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		log.Printf("maker: symbols fetch: %v", err)
		return
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return
	}
	var parsed symbolsResponse
	if err := json.NewDecoder(resp.Body).Decode(&parsed); err != nil {
		return
	}
	for _, s := range parsed.Symbols {
		m.specs[s.ID] = symbolSpec{tick: s.TickSize, lot: s.LotSize}
	}
}

// specFor returns the tick/lot for a symbol, defaulting to 1/1.
func (m *Maker) specFor(symbol uint32) symbolSpec {
	s := m.specs[symbol]
	if s.tick < 1 {
		s.tick = 1
	}
	if s.lot < 1 {
		s.lot = 1
	}
	return s
}

// nextCID returns a fresh 20-char client order id, unique across
// restarts via the per-session prefix.
func (m *Maker) nextCID() string {
	m.orderCount++
	cid := fmt.Sprintf("m%s%d", m.sessionID, m.orderCount)
	if len(cid) > 20 {
		return cid[:20]
	}
	for len(cid) < 20 {
		cid += "0"
	}
	return cid
}

// midFor resolves the reference price for a symbol by precedence:
// override (env or config file) > PriceSource (mark, then BBO mid) >
// per-symbol default.
func (m *Maker) midFor(symbol uint32) int64 {
	if px, ok := m.overrideMid(); ok {
		return px
	}
	if px, ok := m.prices.Ref(symbol); ok {
		return px
	}
	return defaultMid(symbol)
}

// overrideMid returns an active manual mid override, checking the live
// config file first (the dashboard writes mid_override there) and then
// the env var set at construction.
func (m *Maker) overrideMid() (int64, bool) {
	if px, ok := readConfigMid(m.cfg.ConfigFile); ok {
		return px, true
	}
	if m.cfg.HasMid {
		return m.cfg.MidOverride, true
	}
	return 0, false
}

// quoteCycle cancels the previous quotes and places a fresh ladder on
// both sides of the reference for every symbol, over one connection.
func (m *Maker) quoteCycle(ctx context.Context) error {
	dctx, cancel := context.WithTimeout(ctx, dialTimeout)
	conn, err := dialGateway(dctx, m.cfg)
	cancel()
	if err != nil {
		return err
	}
	defer conn.CloseNow()

	for _, cid := range m.active {
		if err := sendCancel(ctx, conn, cid); err != nil {
			return err
		}
	}
	m.active = m.active[:0]

	// No drain here: drainFor bounds its read with a deadline context, and
	// coder/websocket CLOSES the whole conn when a Read's context is
	// cancelled. Draining mid-cycle would kill the conn before the order
	// writes below, so every sendNewOrder would fail "use of closed
	// network connection". All writes go out on the live conn first; the
	// single drain at the end reads acks, then the conn is closed anyway.
	for _, sym := range m.cfg.Symbols {
		mid := m.midFor(sym)
		spec := m.specFor(sym)
		qty := orderQty(m.cfg.QtyPerLevel, spec.lot)
		for level := 0; level < m.cfg.Levels; level++ {
			bidPx, askPx := quote(mid, m.cfg.SpreadBps, spec.tick, level)
			if bidPx > 0 {
				cid := m.nextCID()
				if err := sendNewOrder(ctx, conn, sym, sideBuy, bidPx, qty, cid, tifGTC); err != nil {
					return err
				}
				m.active = append(m.active, cid)
			}
			cid := m.nextCID()
			if err := sendNewOrder(ctx, conn, sym, sideSell, askPx, qty, cid, tifGTC); err != nil {
				return err
			}
			m.active = append(m.active, cid)
		}
	}
	drainFor(ctx, conn, 200*time.Millisecond)
	return nil
}

// cancelAll best-effort cancels every resting order on shutdown so the
// maker leaves no stale liquidity behind. Uses a short fresh context
// because the run context is already cancelled by this point.
func (m *Maker) cancelAll() {
	if len(m.active) == 0 {
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	conn, err := dialGateway(ctx, m.cfg)
	if err != nil {
		return
	}
	defer conn.CloseNow()
	for _, cid := range m.active {
		_ = sendCancel(ctx, conn, cid)
	}
	m.active = m.active[:0]
}

// defaultMid is the fallback reference when no live or override price
// is available, matching the Python maker's per-symbol defaults.
func defaultMid(symbol uint32) int64 {
	switch symbol {
	case 1:
		return 30000
	case 2:
		return 2000
	case 3:
		return 100
	default:
		return 50000
	}
}

// configMidGuard serialises config-file reads; the file is tiny and
// polled once per cycle, so a plain read under a mutex is enough.
var configMidGuard sync.Mutex

// readConfigMid polls the maker config file for a mid_override key. Any
// error (missing file, bad JSON, absent key) yields ok=false.
func readConfigMid(path string) (int64, bool) {
	if path == "" {
		return 0, false
	}
	configMidGuard.Lock()
	defer configMidGuard.Unlock()
	data, err := os.ReadFile(path)
	if err != nil {
		return 0, false
	}
	var cfg struct {
		MidOverride *int64 `json:"mid_override"`
	}
	if err := json.Unmarshal(data, &cfg); err != nil || cfg.MidOverride == nil {
		return 0, false
	}
	return *cfg.MidOverride, true
}
