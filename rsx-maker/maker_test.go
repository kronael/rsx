package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
)

func writeConfig(t *testing.T, body string) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "maker-config.json")
	if err := os.WriteFile(path, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestReadConfigOverridesGlobalMid(t *testing.T) {
	path := writeConfig(t, `{"mid_override": 51000}`)
	got := readConfigOverrides(path)
	if got.mid == nil || *got.mid != 51000 {
		t.Fatalf("mid = %v, want 51000", got.mid)
	}
	if len(got.mids) != 0 || got.depthProfile != "" {
		t.Errorf("unexpected extras: %+v", got)
	}
}

func TestReadConfigOverridesPerSymbol(t *testing.T) {
	path := writeConfig(t, `{"mids": {"10": 36000, "3": 1500000}, "depth_profile": "walls"}`)
	got := readConfigOverrides(path)
	if got.mid != nil {
		t.Errorf("global mid = %v, want nil", got.mid)
	}
	if got.mids[10] != 36000 || got.mids[3] != 1500000 {
		t.Errorf("mids = %v, want {10:36000, 3:1500000}", got.mids)
	}
	if got.depthProfile != "walls" {
		t.Errorf("depthProfile = %q, want walls", got.depthProfile)
	}
}

func TestReadConfigOverridesBadInputsAbstain(t *testing.T) {
	// Missing file, bad JSON, junk symbol keys: zero value, never a panic.
	if got := readConfigOverrides(""); got.mid != nil || got.mids != nil {
		t.Errorf("empty path: %+v", got)
	}
	if got := readConfigOverrides(writeConfig(t, `{nope`)); got.mid != nil {
		t.Errorf("bad json: %+v", got)
	}
	got := readConfigOverrides(writeConfig(t, `{"mids": {"pengu": 1, "10": 2}}`))
	if len(got.mids) != 1 || got.mids[10] != 2 {
		t.Errorf("junk keys not skipped: %v", got.mids)
	}
}

func TestMidForPrecedence(t *testing.T) {
	// Per-symbol override beats global; global beats the price source;
	// an uncovered symbol falls through to the source.
	m := newMaker(Config{}, newBBOSource())
	m.prices.(*bboSource).update(10, 29000, 31000) // mid 30000
	m.prices.(*bboSource).update(3, 1400000, 1600000)

	global := int64(42000)
	ov := configOverrides{mid: &global, mids: map[uint32]int64{10: 36000}}
	if got := m.midFor(10, ov); got != 36000 {
		t.Errorf("per-symbol: %d, want 36000", got)
	}
	if got := m.midFor(3, ov); got != 42000 {
		t.Errorf("global: %d, want 42000", got)
	}
	if got := m.midFor(3, configOverrides{}); got != 1500000 {
		t.Errorf("source: %d, want 1500000", got)
	}
}

// cidShape is the spec 61 client form: 1-20 ASCII chars of [0-9A-Za-z-_].
var cidShape = regexp.MustCompile(`^[0-9A-Za-z_-]{1,20}$`)

// TestNextCIDUniqueWithinSession: the gateway binds a cid to its order per
// user for an hour and refuses any repeat with E[1011], so no two
// orders of one session may share a cid. A scheme that pads cids with "0"
// makes counters 1, 10 and 100 the same string.
func TestNextCIDUniqueWithinSession(t *testing.T) {
	maker := newMaker(Config{}, nil)
	seen := make(map[string]int)
	for count := 1; count <= 100_000; count++ {
		cid := maker.nextCID()
		if prior, dup := seen[cid]; dup {
			t.Fatalf("order %d reuses cid %q of order %d", count, cid, prior)
		}
		seen[cid] = count
	}
}

// TestNextCIDDistinctAcrossRestarts: two simulated process starts within
// the gateway's one-hour window must not mint a common cid.
func TestNextCIDDistinctAcrossRestarts(t *testing.T) {
	first := newMaker(Config{}, nil)
	second := newMaker(Config{}, nil)
	minted := make(map[string]struct{})
	for count := 0; count < 20_000; count++ {
		minted[first.nextCID()] = struct{}{}
	}
	for count := 0; count < 20_000; count++ {
		cid := second.nextCID()
		if _, dup := minted[cid]; dup {
			t.Fatalf("restart reuses cid %q", cid)
		}
	}
}

// TestNextCIDShape: every cid is 1-20 chars of the spec alphabet, carries a
// session part of at least 8 chars (40 random bits) and is never padded.
func TestNextCIDShape(t *testing.T) {
	maker := newMaker(Config{}, nil)
	for count := 1; count <= 5_000; count++ {
		cid := maker.nextCID()
		if !cidShape.MatchString(cid) {
			t.Fatalf("cid %q is not 1-20 chars of [0-9A-Za-z_-]", cid)
		}
		if len(cid) <= sessionChars {
			t.Fatalf("cid %q has no counter after the %d-char session", cid, sessionChars)
		}
		counter := strings.TrimPrefix(cid, maker.sessionID)
		if want := strconv.FormatUint(uint64(count), 36); counter != want {
			t.Fatalf("cid %q: counter %q, want %q", cid, counter, want)
		}
	}
}

// TestFetchSymbolsReadsTheMinimum: the maker takes tick, lot and the
// minimum notional from the catalog, and falls back to 1 USD from the
// decimals when the catalog omits the minimum.
func TestFetchSymbolsReadsTheMinimum(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		fmt.Fprint(w, `{"symbols":[
			{"id":1,"tick_size":1,"lot_size":1000,"price_decimals":2,"qty_decimals":8,"min_notional":30000000000},
			{"id":10,"tick_size":1,"lot_size":100000,"price_decimals":6,"qty_decimals":4}]}`)
	}))
	defer server.Close()
	maker := newMaker(Config{SymbolsURL: server.URL}, nil)
	maker.fetchSymbols(context.Background())
	if got := maker.specFor(1).minNotional; got != 30_000_000_000 {
		t.Errorf("symbol 1 minNotional = %d, want 3e10", got)
	}
	if got := maker.specFor(10).minNotional; got != 10_000_000_000 {
		t.Errorf("symbol 10 minNotional = %d, want the 1 USD fallback 1e10", got)
	}
}

// TestQuoteCycleOrdersMeetTheMinimum drives quoteCycle against a fake
// gateway socket: every order frame carries a lot-multiple quantity whose
// notional at its own limit price reaches the symbol minimum, and no cid
// repeats across cycles.
func TestQuoteCycleOrdersMeetTheMinimum(t *testing.T) {
	const cycles = 6
	// 10 USD on the 1 USD presets: above what the default 10-lot quotes
	// reach, so the sizing has to raise them.
	const floor = 100_000_000_000
	frames := make(chan []byte, 1024)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		for {
			_, data, err := conn.Read(context.Background())
			if err != nil {
				return
			}
			frames <- data
		}
	}))
	defer server.Close()
	url := "ws" + strings.TrimPrefix(server.URL, "http")

	cfg := Config{
		GatewayURL: url, JWTSecret: "secret", Symbols: []uint32{1, 10},
		SpreadBps: 10, QtyPerLevel: 10, Levels: 5,
	}
	maker := newMaker(cfg, staticPrices{})
	maker.specs[1] = symbolSpec{tick: 1, lot: 1000, minNotional: floor}
	maker.specs[10] = symbolSpec{tick: 1, lot: 100_000, minNotional: floor}
	ctx := context.Background()
	conn, err := dialGateway(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.CloseNow()
	maker.conn = conn

	for cycle := 0; cycle < cycles; cycle++ {
		if err := maker.quoteCycle(ctx); err != nil {
			t.Fatal(err)
		}
	}
	wantOrders := cycles * 2 * 5 * 2
	seen := make(map[string]bool)
	orders, raised := 0, 0
	for orders < wantOrders {
		var frame map[string][]any
		select {
		case data := <-frames:
			if err := json.Unmarshal(data, &frame); err != nil {
				t.Fatal(err)
			}
		case <-time.After(5 * time.Second):
			t.Fatalf("saw %d of %d orders", orders, wantOrders)
		}
		fields, ok := frame["N"]
		if !ok {
			continue
		}
		orders++
		sym := uint32(fields[0].(float64))
		px := int64(fields[2].(float64))
		qty := int64(fields[3].(float64))
		cid := fields[4].(string)
		spec := maker.specs[sym]
		if px*qty < spec.minNotional {
			t.Errorf("sym %d px %d qty %d: notional %d below %d", sym, px, qty, px*qty, spec.minNotional)
		}
		if qty > 10*spec.lot {
			raised++
		}
		if qty%spec.lot != 0 {
			t.Errorf("sym %d qty %d is not a multiple of lot %d", sym, qty, spec.lot)
		}
		if seen[cid] {
			t.Errorf("cid %q sent twice", cid)
		}
		seen[cid] = true
	}
	if raised == 0 {
		t.Error("no order was raised to the minimum; the floor was not exercised")
	}
}

// staticPrices answers no reference price, so midFor uses defaultMid.
type staticPrices struct{}

func (staticPrices) Ref(uint32) (int64, bool) { return 0, false }

// TestJitterNeverTouchesThePreviousCycle: the cancels of cycle n may land
// after the orders of cycle n+1, so with a jitter far wider than the spread
// the new top bid stays below the previous top ask and the new top ask above
// the previous top bid.
func TestJitterNeverTouchesThePreviousCycle(t *testing.T) {
	const cycles = 200
	const levels = 3
	frames := make(chan []byte, 4096)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		for {
			_, data, err := conn.Read(context.Background())
			if err != nil {
				return
			}
			frames <- data
		}
	}))
	defer server.Close()
	url := "ws" + strings.TrimPrefix(server.URL, "http")

	cfg := Config{
		GatewayURL: url, JWTSecret: "secret", Symbols: []uint32{1},
		SpreadBps: 10, QtyPerLevel: 10, Levels: levels, JitterBps: 500,
	}
	maker := newMaker(cfg, staticPrices{})
	ctx := context.Background()
	conn, err := dialGateway(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.CloseNow()
	maker.conn = conn

	for cycle := 0; cycle < cycles; cycle++ {
		if err := maker.quoteCycle(ctx); err != nil {
			t.Fatal(err)
		}
	}
	var tops []topQuote
	var orders int
	for orders < cycles*2*levels {
		var frame map[string][]any
		select {
		case data := <-frames:
			if err := json.Unmarshal(data, &frame); err != nil {
				t.Fatal(err)
			}
		case <-time.After(5 * time.Second):
			t.Fatalf("saw %d of %d orders", orders, cycles*2*levels)
		}
		fields, ok := frame["N"]
		if !ok {
			continue
		}
		px := int64(fields[2].(float64))
		switch orders % (2 * levels) {
		case 0:
			tops = append(tops, topQuote{bid: px})
		case 1:
			tops[len(tops)-1].ask = px
		}
		orders++
	}
	moved := false
	for i := 1; i < len(tops); i++ {
		if tops[i].bid >= tops[i-1].ask {
			t.Errorf("cycle %d: bid %d touches the previous ask %d", i, tops[i].bid, tops[i-1].ask)
		}
		if tops[i].ask <= tops[i-1].bid {
			t.Errorf("cycle %d: ask %d touches the previous bid %d", i, tops[i].ask, tops[i-1].bid)
		}
		moved = moved || tops[i].bid != tops[0].bid
	}
	if !moved {
		t.Error("the jitter never moved the ladder")
	}
}

// TestJitterLetsARealReferenceMoveThrough: a reference that jumped by more
// than the jitter amplitude is not held back by the previous cycle's quotes.
func TestJitterLetsARealReferenceMoveThrough(t *testing.T) {
	maker := newMaker(Config{SpreadBps: 10, JitterBps: 10}, nil)
	maker.last[1] = lastQuote{mid: 1_000_000, top: topQuote{bid: 999_000, ask: 1_001_000}}
	const mid = 2_000_000
	const amp = mid * 10 / 10000
	for range 50 {
		shift := maker.jitter(1, mid, 1)
		if shift < -amp || shift > amp {
			t.Fatalf("shift %d outside ±%d after a reference jump", shift, amp)
		}
	}
}
