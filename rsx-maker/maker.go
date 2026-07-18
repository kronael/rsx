package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"math/rand"
	"net/http"
	"os"
	"strconv"
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

	// conn is the single long-lived gateway connection quoteCycle writes
	// over; readerDone closes when its reader goroutine exits. Both are
	// owned solely by the Run goroutine.
	conn       *websocket.Conn
	readerDone chan struct{}
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
		// Ensure a live connection: the initial dial, or a reconnect after
		// a prior write error dropped it. connect blocks with backoff until
		// it succeeds, returning an error only if ctx is cancelled first.
		if m.conn == nil {
			if err := m.connect(ctx); err != nil {
				return
			}
		}
		if err := m.quoteCycle(ctx); err != nil && ctx.Err() == nil {
			log.Printf("maker: quote cycle: %v", err)
			m.dropConn() // reconnect on the next iteration
		}
		select {
		case <-ctx.Done():
			m.shutdown()
			return
		case <-ticker.C:
		}
	}
}

// connect dials the gateway, keeps the connection on the Maker, and starts
// the reader goroutine that drains inbound frames for its lifetime. It
// backs off on dial failure (matching awaitGateway) so a reconnect can't
// wedge the loop, and returns an error only if ctx is cancelled first.
func (m *Maker) connect(ctx context.Context) error {
	delay := time.Second
	const maxDelay = 16 * time.Second
	for {
		dctx, cancel := context.WithTimeout(ctx, dialTimeout)
		conn, err := dialGateway(dctx, m.cfg)
		cancel()
		if err == nil {
			m.conn = conn
			m.readerDone = make(chan struct{})
			go readLoop(conn, m.readerDone)
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

// readLoop drains and discards inbound gateway frames (acks, rejects,
// heartbeats) for the life of conn. coder/websocket buffers unread frames,
// so without a live reader the socket fills and the next Write stalls;
// draining here keeps the persistent connection writable. It exits when
// conn is closed — by dropConn on a write error, or by shutdown's clean
// close.
//
// Read runs on context.Background(), never the run ctx: coder/websocket
// closes the whole connection the instant a Read's context is cancelled
// (see the old drainFor), which would abruptly kill the socket on shutdown
// and race shutdown's clean cancel-and-close. Teardown is always explicit,
// and closing the conn unblocks this Read.
func readLoop(conn *websocket.Conn, done chan struct{}) {
	defer close(done)
	for {
		if _, _, err := conn.Read(context.Background()); err != nil {
			return
		}
	}
}

// dropConn tears down the current connection after a write error so the
// next loop iteration reconnects. CloseNow unblocks readLoop. Resting
// orders survive the disconnect (the gateway does not cancel on close), so
// m.active is kept and its cids are cancelled on the next cycle.
func (m *Maker) dropConn() {
	if m.conn == nil {
		return
	}
	_ = m.conn.CloseNow()
	<-m.readerDone
	m.conn = nil
	m.readerDone = nil
}

// dialTimeout bounds each gateway WS handshake so a half-open or stalled dial
// can't wedge the quote loop indefinitely (mirrors the Python maker's
// ClientTimeout=3s, and shutdown's own timeout). Only the dial is bounded;
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
// per-symbol config-file override > global override (config file, then
// env) > PriceSource (mark, then BBO mid) > per-symbol default.
func (m *Maker) midFor(symbol uint32, overrides configOverrides) int64 {
	if px, ok := overrides.mids[symbol]; ok {
		return px
	}
	if px, ok := m.overrideMid(overrides); ok {
		return px
	}
	if px, ok := m.prices.Ref(symbol); ok {
		return px
	}
	return defaultMid(symbol)
}

// jittered nudges the reference by a random amount within ±JitterBps so the
// demo books visibly move each cycle. 0 (the default) leaves quotes static.
func (m *Maker) jittered(mid int64) int64 {
	if m.cfg.JitterBps <= 0 || mid <= 0 {
		return mid
	}
	amp := mid * m.cfg.JitterBps / 10000
	if amp <= 0 {
		return mid
	}
	return mid + rand.Int63n(2*amp+1) - amp
}

// overrideMid returns an active manual global mid override: the live
// config file's mid_override (the dashboard writes it there) beats the
// env var set at construction.
func (m *Maker) overrideMid(overrides configOverrides) (int64, bool) {
	if overrides.mid != nil {
		return *overrides.mid, true
	}
	if m.cfg.HasMid {
		return m.cfg.MidOverride, true
	}
	return 0, false
}

// quoteCycle cancels the previous quotes and places a fresh ladder on
// both sides of the reference for every symbol, over the persistent
// connection. Inbound acks are drained by readLoop, not here — so the
// cancel-replace writes actually land instead of being lost to a
// per-cycle close.
func (m *Maker) quoteCycle(ctx context.Context) error {
	for _, cid := range m.active {
		if err := sendCancel(ctx, m.conn, cid); err != nil {
			return err
		}
	}
	m.active = m.active[:0]

	overrides := readConfigOverrides(m.cfg.ConfigFile)
	profile := m.depthProfile(overrides)
	for _, sym := range m.cfg.Symbols {
		mid := m.jittered(m.midFor(sym, overrides))
		spec := m.specFor(sym)
		for level := 0; level < m.cfg.Levels; level++ {
			bidPx, askPx := quote(mid, m.cfg.SpreadBps, spec.tick, level)
			if bidPx > 0 {
				cid := m.nextCID()
				qty := m.levelQty(profile, spec.lot, level)
				if err := sendNewOrder(ctx, m.conn, sym, sideBuy, bidPx, qty, cid, tifGTC); err != nil {
					return err
				}
				m.active = append(m.active, cid)
			}
			cid := m.nextCID()
			qty := m.levelQty(profile, spec.lot, level)
			if err := sendNewOrder(ctx, m.conn, sym, sideSell, askPx, qty, cid, tifGTC); err != nil {
				return err
			}
			m.active = append(m.active, cid)
		}
	}
	return nil
}

// depthProfile resolves the active depth profile: the live config file
// (polled every cycle, so a demo scene can flip it without a restart)
// beats the env var set at construction. Empty = uniform (production).
func (m *Maker) depthProfile(overrides configOverrides) string {
	if overrides.depthProfile != "" {
		return overrides.depthProfile
	}
	return m.cfg.DepthProfile
}

// levelQty returns the order size for one side of one depth level. The
// uniform profile is exact (byte-for-byte the old behavior); a shaped
// profile adds ±25% per-order jitter so the two sides and successive
// cycles differ — real orders with real, varied sizes, which is what
// paints a liquidity heatmap with texture instead of one flat tier.
func (m *Maker) levelQty(profile string, lot int64, level int) int64 {
	q := depthQty(profile, m.cfg.QtyPerLevel, lot, level)
	if profile == "" {
		return q
	}
	jit := q * (rand.Int63n(51) - 25) / 100
	q += jit / lot * lot
	if q < lot {
		q = lot
	}
	return q
}

// shutdown cancels every resting order over the persistent connection then
// closes it cleanly, so the maker leaves no stale liquidity and the gateway
// sees a proper close frame (no truncated-read churn). The run ctx is
// already cancelled here, so a fresh short context bounds the final writes.
func (m *Maker) shutdown() {
	if m.conn == nil {
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	for _, cid := range m.active {
		_ = sendCancel(ctx, m.conn, cid)
	}
	m.active = m.active[:0]
	_ = m.conn.Close(websocket.StatusNormalClosure, "")
	<-m.readerDone
	m.conn = nil
	m.readerDone = nil
}

// defaultMid is the fallback reference when no live or override price
// is available, matching the Python maker's per-symbol defaults.
func defaultMid(symbol uint32) int64 {
	// Realistic demo mids, raw units matching each symbol's price_decimals
	// (BTC $60000, ETH $3000, SOL $150, PENGU $0.03) so the demo books read
	// like a real market instead of placeholder round numbers.
	switch symbol {
	case 1: // BTC, dec 2
		return 6_000_000
	case 2: // ETH, dec 2
		return 300_000
	case 3: // SOL, dec 4
		return 1_500_000
	case 10: // PENGU, dec 6
		return 30_000
	default:
		return 50_000
	}
}

// configOverrides is the live-tunable slice of the maker config file,
// re-read every quote cycle: a global mid override (dashboard), optional
// per-symbol mid overrides (demo scene price paths — other symbols keep
// their live refs), and the depth profile.
type configOverrides struct {
	mid          *int64
	mids         map[uint32]int64
	depthProfile string
}

// configMidGuard serialises config-file reads; the file is tiny and
// polled once per cycle, so a plain read under a mutex is enough.
var configMidGuard sync.Mutex

// readConfigOverrides polls the maker config file for the live-tunable
// keys (mid_override, mids, depth_profile). Any error (missing file,
// bad JSON, absent keys) yields the zero value — env/live sources win.
func readConfigOverrides(path string) configOverrides {
	if path == "" {
		return configOverrides{}
	}
	configMidGuard.Lock()
	defer configMidGuard.Unlock()
	data, err := os.ReadFile(path)
	if err != nil {
		return configOverrides{}
	}
	var cfg struct {
		MidOverride  *int64           `json:"mid_override"`
		Mids         map[string]int64 `json:"mids"`
		DepthProfile string           `json:"depth_profile"`
	}
	if err := json.Unmarshal(data, &cfg); err != nil {
		return configOverrides{}
	}
	out := configOverrides{mid: cfg.MidOverride, depthProfile: cfg.DepthProfile}
	if len(cfg.Mids) > 0 {
		out.mids = make(map[uint32]int64, len(cfg.Mids))
		for k, v := range cfg.Mids {
			if n, err := strconv.ParseUint(k, 10, 32); err == nil {
				out.mids[uint32(n)] = v
			}
		}
	}
	return out
}
