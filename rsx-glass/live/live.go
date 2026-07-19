// Package live is rsx-glass's market-data connector: it dials the public feed,
// subscribes, and turns the exchange's binary protobuf frames into the same
// normalized wire.* events rsx-term's model folds. It reuses rsx-term's REAL
// decoder (wire.DecodeMd) and speaks the REAL feed handshake — the JSON
// `{"S":[id,channels]}` subscribe, the binary MdFrame stream, the `{"H":[ts]}`
// heartbeat echo — so this is the same protocol conn/live.go's readMd runs
// against the live daemon, narrowed to the read-only market-data path the
// heatmap viewer needs (no order socket).
//
// The connector does no folding and holds no model: it emits decoded events on
// Events(), and the caller pumps them into a glass.Feed. That keeps transport
// (this package, with the network dependency) cleanly separate from the model
// (package glass, dependency-light) — the same split conn/feed/ui keep in
// rsx-term.
package live

import (
	"context"
	"fmt"
	"sync/atomic"
	"time"

	"github.com/coder/websocket"

	"rsx-term/feed"
	"rsx-term/wire"
)

// DefaultChannels subscribes every public channel: 1=bbo | 2=depth | 4=trades
// (specs/2/49-webproto.md), i.e. 7 — the mask conn/live.go uses.
const DefaultChannels = 7

const (
	backoffInitial = 100 * time.Millisecond
	backoffMax     = 2 * time.Second
	eventBuffer    = 256
)

// Client is a single market-data subscription. Construct with New, drain
// Events, Start the reader, and Close to stop.
type Client struct {
	url      string
	symbolID uint32
	channels int

	events chan any
	closed atomic.Bool
	done   chan struct{}
	active atomic.Pointer[websocket.Conn]
}

// New builds a subscriber for symbolID on url (subscribing every channel). Call
// Start to begin reading; drain Events concurrently.
func New(url string, symbolID uint32) *Client {
	return &Client{
		url:      url,
		symbolID: symbolID,
		channels: DefaultChannels,
		events:   make(chan any, eventBuffer),
		done:     make(chan struct{}),
	}
}

// Events is the decoded market-data stream: feed.MdUp / feed.MdDown link
// transitions and wire.Snapshot / Delta / Bbo / MdTrade frames, in the shapes
// glass.Feed.Apply folds. Heartbeats are answered internally and not surfaced.
func (c *Client) Events() <-chan any { return c.events }

// Start launches the reader goroutine (a named method, so its lifetime is
// legible): it dials, subscribes, drains, and reconnects with bounded backoff
// until ctx ends or Close is called.
func (c *Client) Start(ctx context.Context) { go c.readLoop(ctx) }

// Close stops the reader and interrupts a blocked read by closing the active
// socket, so an idle Read returns at once.
func (c *Client) Close() {
	if c.closed.CompareAndSwap(false, true) {
		close(c.done)
	}
	if conn := c.active.Load(); conn != nil {
		_ = conn.CloseNow()
	}
}

// readLoop dials + subscribes, then drains until the socket errors; on any drop
// it reports feed.MdDown and redials with bounded backoff, re-subscribing each
// time (mirrors conn/live.go's reconnectMd). Returns once Close / ctx-cancel
// stops the attempts.
func (c *Client) readLoop(ctx context.Context) {
	backoff := time.Duration(0)
	for {
		if c.stopping(ctx) {
			return
		}
		conn, err := c.dialAndSubscribe(ctx)
		if err != nil {
			c.emit(ctx, feed.MdDown{})
			backoff = nextBackoff(backoff)
			if !sleepBackoff(ctx, backoff) {
				return
			}
			continue
		}
		backoff = 0
		c.active.Store(conn)
		c.emit(ctx, feed.MdUp{})
		c.drain(ctx, conn)
		_ = conn.CloseNow()
		c.active.Store(nil)
		if c.stopping(ctx) {
			return
		}
		c.emit(ctx, feed.MdDown{})
	}
}

// dialAndSubscribe opens the WS and sends the real subscribe frame for the
// symbol and channel mask.
func (c *Client) dialAndSubscribe(ctx context.Context) (*websocket.Conn, error) {
	conn, _, err := websocket.Dial(ctx, c.url, nil)
	if err != nil {
		return nil, err
	}
	frame := fmt.Sprintf(`{"S":[%d,%d]}`, c.symbolID, c.channels)
	writeCtx, cancel := context.WithTimeout(ctx, 5*time.Second)
	err = conn.Write(writeCtx, websocket.MessageText, []byte(frame))
	cancel()
	if err != nil {
		_ = conn.CloseNow()
		return nil, err
	}
	return conn, nil
}

// drain reads binary MdFrames until the socket errors, decoding each with
// rsx-term's real decoder. A malformed frame is skipped, never fatal — the
// public feed is untrusted (specs/2/4-cast.md §10.4). A heartbeat is echoed
// back as the server's liveness signal; every other frame is emitted.
func (c *Client) drain(ctx context.Context, conn *websocket.Conn) {
	for {
		_, data, err := conn.Read(ctx)
		if err != nil {
			return
		}
		decoded, err := wire.DecodeMd(data)
		if err != nil {
			continue // untrusted feed: drop the malformed frame, keep reading
		}
		if hb, ok := decoded.(wire.MdHeartbeat); ok {
			c.echo(ctx, conn, hb.TsMs)
			continue
		}
		c.emit(ctx, decoded)
	}
}

// echo answers a server heartbeat with the `{"H":[ts]}` liveness reply — a
// best-effort write; a failure is observed a moment later by drain's own Read.
func (c *Client) echo(ctx context.Context, conn *websocket.Conn, tsMs uint64) {
	writeCtx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	_ = conn.Write(writeCtx, websocket.MessageText, []byte(fmt.Sprintf(`{"H":[%d]}`, tsMs)))
}

// emit delivers ev on Events, releasing if the client closes or ctx ends so a
// stalled consumer can never wedge the reader.
func (c *Client) emit(ctx context.Context, ev any) {
	select {
	case c.events <- ev:
	case <-ctx.Done():
	case <-c.done:
	}
}

// stopping reports whether the reader should give up (Close called or ctx
// ended).
func (c *Client) stopping(ctx context.Context) bool {
	return c.closed.Load() || ctx.Err() != nil
}

// sleepBackoff waits d, returning false early if ctx ends first.
func sleepBackoff(ctx context.Context, d time.Duration) bool {
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-t.C:
		return true
	case <-ctx.Done():
		return false
	}
}

// nextBackoff advances the reconnect delay: backoffInitial after a reset,
// doubling to backoffMax.
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
