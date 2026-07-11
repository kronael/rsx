package news

import (
	"context"
	"encoding/json"
	"strings"
	"sync"
	"time"

	"github.com/coder/websocket"
)

// TreeOfAlphaURL is the Tree of Alpha news websocket. On connect the server
// streams JSON headline objects roughly shaped like:
//
//	{"title":"...","source":"Twitter","symbols":["BTC"],"time":1700000000000,...}
//
// where "time" is Unix epoch milliseconds.
const TreeOfAlphaURL = "wss://news.treeofalpha.com/ws"

// bufferCap bounds the in-memory headline ring.
const bufferCap = 512

// backoffInitial / backoffMax bound the reader's reconnect delay.
const backoffInitial = time.Second
const backoffMax = time.Minute

// TreeOfAlpha is the live Tree of Alpha client. It is OFF until Start is
// called (RSX_TERM_NEWS=1 — the default terminal never dials). The reader is
// a named background goroutine feeding an in-memory ring; Markers/All/Search
// only ever read that ring, never the network, so rendering can never block
// on this feed and an unreachable server just means an empty rail.
type TreeOfAlpha struct {
	mu      sync.Mutex
	markers []Marker // newest last
	enabled bool
}

// NewTreeOfAlpha builds the client (no I/O).
func NewTreeOfAlpha() *TreeOfAlpha {
	return &TreeOfAlpha{}
}

// Start is the explicit opt-in: it marks the source enabled and launches the
// named background reader (the ONLY place this package dials). Never blocks.
func (t *TreeOfAlpha) Start(ctx context.Context) {
	t.mu.Lock()
	t.enabled = true
	t.mu.Unlock()
	go t.readLoop(ctx)
}

// Enabled reports whether Start has been called.
func (t *TreeOfAlpha) Enabled() bool {
	t.mu.Lock()
	defer t.mu.Unlock()
	return t.enabled
}

// Markers returns buffered markers within [sinceNs, untilNs].
func (t *TreeOfAlpha) Markers(sinceNs, untilNs int64) []Marker {
	t.mu.Lock()
	defer t.mu.Unlock()
	if !t.enabled {
		return nil
	}
	var out []Marker
	for _, m := range t.markers {
		if m.TsNs >= sinceNs && m.TsNs <= untilNs {
			out = append(out, m)
		}
	}
	return out
}

// All returns every buffered headline, newest first.
func (t *TreeOfAlpha) All() []Marker {
	t.mu.Lock()
	defer t.mu.Unlock()
	out := make([]Marker, len(t.markers))
	for i, m := range t.markers {
		out[len(t.markers)-1-i] = m
	}
	return out
}

// push appends one headline, trimming the ring.
func (t *TreeOfAlpha) push(m Marker) {
	t.mu.Lock()
	defer t.mu.Unlock()
	t.markers = append(t.markers, m)
	if len(t.markers) > bufferCap {
		t.markers = t.markers[len(t.markers)-bufferCap:]
	}
}

// readLoop dials TreeOfAlphaURL and appends each decoded headline to the
// ring, reconnecting with bounded backoff on any error, until ctx ends. All
// failures are silent-by-design: news is an overlay, never a dependency.
func (t *TreeOfAlpha) readLoop(ctx context.Context) {
	backoff := time.Duration(0)
	for ctx.Err() == nil {
		conn, _, err := websocket.Dial(ctx, TreeOfAlphaURL, nil)
		if err != nil {
			backoff = nextBackoff(backoff)
			if !sleep(ctx, backoff) {
				return
			}
			continue
		}
		backoff = 0
		t.drain(ctx, conn)
		_ = conn.CloseNow()
	}
}

// drain reads frames until the socket errors.
func (t *TreeOfAlpha) drain(ctx context.Context, conn *websocket.Conn) {
	for {
		_, data, err := conn.Read(ctx)
		if err != nil {
			return
		}
		if m, ok := DecodeHeadline(data); ok {
			t.push(m)
		}
	}
}

// toaFrame is the Tree of Alpha headline shape (fields we read).
type toaFrame struct {
	Title   string   `json:"title"`
	Body    string   `json:"body"`
	Source  string   `json:"source"`
	Symbols []string `json:"symbols"`
	TimeMs  int64    `json:"time"`
}

// DecodeHeadline parses one Tree of Alpha frame into a Marker (false for
// frames without a title/timestamp — pings, receipts).
func DecodeHeadline(data []byte) (Marker, bool) {
	var f toaFrame
	if err := json.Unmarshal(data, &f); err != nil {
		return Marker{}, false
	}
	text := f.Title
	if text == "" {
		text = f.Body
	}
	if text == "" || f.TimeMs <= 0 {
		return Marker{}, false
	}
	symbols := make([]string, 0, len(f.Symbols))
	for _, s := range f.Symbols {
		// ToA tags pairs like "BTCUSDT" alongside bare coins; keep the raw
		// tag — matching against a venue's coin is a prefix check upstream.
		if s != "" {
			symbols = append(symbols, strings.ToUpper(s))
		}
	}
	return Marker{
		TsNs:    f.TimeMs * int64(time.Millisecond),
		Text:    text,
		Source:  f.Source,
		Symbols: symbols,
		Tier:    DeriveSeverity(text, symbols),
	}, true
}

// DeriveSeverity grades a headline 0 (routine) … 3 (critical). Tree of Alpha
// carries no severity field, so this is a keyword heuristic: market-halting
// events read critical, market-moving macro/regulatory reads high, anything
// symbol-tagged reads medium, the rest stays quiet. Best-effort by design —
// the rail marker hue is a triage cue, not a verdict.
func DeriveSeverity(text string, symbols []string) int {
	lower := strings.ToLower(text)
	for _, w := range criticalWords {
		if strings.Contains(lower, w) {
			return 3
		}
	}
	for _, w := range highWords {
		if strings.Contains(lower, w) {
			return 2
		}
	}
	if len(symbols) > 0 {
		return 1
	}
	return 0
}

// criticalWords flag market-halting / capital-at-risk events.
var criticalWords = []string{
	"hack", "exploit", "halt", "bankrupt", "insolven", "depeg",
	"emergency", "attack", "drained", "rug", "seized",
}

// highWords flag market-moving macro / regulatory / listing events.
var highWords = []string{
	"sec ", "etf", "fed ", "rate", "lawsuit", "sues", "delist",
	"listing", "lists", "acquisition", "acquire", "partnership",
	"upgrade", "fork", "liquidat",
}

// nextBackoff doubles toward backoffMax (backoffInitial on the first call).
func nextBackoff(cur time.Duration) time.Duration {
	if cur <= 0 {
		return backoffInitial
	}
	next := cur * 2
	if next > backoffMax {
		return backoffMax
	}
	return next
}

// sleep waits d or until ctx ends (false = stop).
func sleep(ctx context.Context, d time.Duration) bool {
	timer := time.NewTimer(d)
	defer timer.Stop()
	select {
	case <-timer.C:
		return true
	case <-ctx.Done():
		return false
	}
}
