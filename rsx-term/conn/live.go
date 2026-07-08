// This file is the live network transport: two coder/websocket
// connections (the private gateway order channel and the public
// marketdata channel), each drained by its own reader goroutine. It
// implements feed.Submitter (same as MockGateway) and decodes/folds
// frames through the existing wire/book logic — the SAME fold the mock
// exercises — delivering the resulting messages on a channel the caller
// pumps into the running tea.Program. See specs/2/49-webproto.md and
// specs/2/55-terminal.md.
package conn

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"sync"
	"sync/atomic"
	"time"

	"github.com/coder/websocket"

	"rsx-term/feed"
	"rsx-term/wire"
)

// writeTimeout bounds any single frame write so a stalled peer cannot hang
// the caller (Submit is called from the UI's Update, which must not block).
const writeTimeout = 5 * time.Second

// eventBuffer sizes the events channel so the two connect-time link-up
// events never block before the caller starts draining Events().
const eventBuffer = 64

// mdChannels subscribes to every public feed channel: 1=bbo, 2=depth,
// 4=trades (specs/2/49-webproto.md).
const mdChannels = 7

// errGwDown is returned by Submit/Cancel when the private socket is not
// connected (dial never completed, or the read loop observed a hangup).
var errGwDown = errors.New("conn: gateway link down")

// LiveGateway is the live gateway + marketdata transport. It satisfies
// feed.Submitter so the UI drives it exactly like MockGateway.
type LiveGateway struct {
	gwURL     string
	mdURL     string
	jwtSecret string
	userID    uint32
	symbolID  uint32

	// baseCtx bounds every read/write for the lifetime of the connection;
	// canceling it (via Close's caller) tears down both sockets. Storing
	// it is the pragmatic choice here: feed.Submitter's Submit/Cancel
	// take no context (the UI's Update can't thread one through), so the
	// connection's own lifetime context is the only one available.
	baseCtx context.Context

	gwConn *websocket.Conn
	mdConn *websocket.Conn

	// folder is the stateful cid/oid pairing fold (rsx-term/wire.Folder).
	// It is touched by Submit (records a sent order) and by the gw read
	// loop (folds each incoming frame against it) concurrently.
	folderMu sync.Mutex
	folder   *wire.Folder

	cidCounter uint64

	events chan any
}

// NewLiveGateway builds a LiveGateway. Call Connect before Submit/Cancel or
// draining Events.
func NewLiveGateway(gwURL, mdURL, jwtSecret string, userID, symbolID uint32) *LiveGateway {
	return &LiveGateway{
		gwURL:     gwURL,
		mdURL:     mdURL,
		jwtSecret: jwtSecret,
		userID:    userID,
		symbolID:  symbolID,
		folder:    wire.NewFolder(),
		events:    make(chan any, eventBuffer),
	}
}

// Events returns the channel of messages this connection delivers: link
// transitions (feed.GwUp/GwDown/MdUp/MdDown) and wire.* market-data/private
// events, in the same shapes conn.DemoScript() replays. The caller sends
// each one into the running tea.Program.
func (g *LiveGateway) Events() <-chan any {
	return g.events
}

// Connect dials the private gateway socket (fatal on failure — there is no
// terminal without an order path) and the public marketdata socket
// (independent: a marketdata dial/subscribe failure is reported as
// feed.MdDown on Events and does not fail Connect). ctx bounds the whole
// connection lifetime; canceling it after Connect tears down both sockets.
func (g *LiveGateway) Connect(ctx context.Context) error {
	g.baseCtx = ctx

	gwConn, _, err := websocket.Dial(ctx, g.gwURL, &websocket.DialOptions{
		HTTPHeader: http.Header{
			"Authorization": []string{"Bearer " + mintJWT(g.userID, g.jwtSecret)},
		},
	})
	if err != nil {
		return fmt.Errorf("conn: dial gateway %s: %w", g.gwURL, err)
	}
	g.gwConn = gwConn
	g.events <- feed.GwUp{}
	go g.readGw(ctx)

	g.connectMd(ctx)
	return nil
}

// connectMd dials the marketdata socket and sends the subscribe frame. A
// failure at either step is a degraded-but-running terminal (order path is
// unaffected), reported as feed.MdDown rather than returned to the caller.
func (g *LiveGateway) connectMd(ctx context.Context) {
	mdConn, _, err := websocket.Dial(ctx, g.mdURL, nil)
	if err != nil {
		g.events <- feed.MdDown{}
		return
	}
	frame := fmt.Sprintf(`{"S":[%d,%d]}`, g.symbolID, mdChannels)
	wctx, cancel := context.WithTimeout(ctx, writeTimeout)
	defer cancel()
	if err := mdConn.Write(wctx, websocket.MessageText, []byte(frame)); err != nil {
		_ = mdConn.CloseNow()
		g.events <- feed.MdDown{}
		return
	}
	g.mdConn = mdConn
	g.events <- feed.MdUp{}
	go g.readMd(ctx)
}

// Close tears down both sockets immediately (CloseNow, no close-handshake
// wait): the terminal quitting should exit at once rather than block on a
// peer that may not be reading anymore. Safe to call even if a socket
// never connected (connectMd leaves mdConn nil on failure).
func (g *LiveGateway) Close() {
	if g.gwConn != nil {
		_ = g.gwConn.CloseNow()
	}
	if g.mdConn != nil {
		_ = g.mdConn.CloseNow()
	}
}

// Submit satisfies feed.Submitter: encodes o as a webproto-49 "N" frame and
// writes it to the private socket. It records the send time against the
// generated cid first (Folder.Sent) so the RTT on the paired "U" accept is
// measured from the actual wire write, not from whenever the caller reads
// the resulting event.
func (g *LiveGateway) Submit(o wire.OrderReq) error {
	if g.gwConn == nil {
		return errGwDown
	}
	cid := wire.Cid(atomic.AddUint64(&g.cidCounter, 1))
	now := time.Now()

	g.folderMu.Lock()
	g.folder.Sent(o, cid, now)
	g.folderMu.Unlock()

	frame := wire.EncodeNew(g.symbolID, cid, o)
	ctx, cancel := context.WithTimeout(g.baseCtx, writeTimeout)
	defer cancel()
	if err := g.gwConn.Write(ctx, websocket.MessageText, []byte(frame)); err != nil {
		return fmt.Errorf("conn: submit: %w", err)
	}
	return nil
}

// Cancel satisfies feed.Submitter: encodes cid as a webproto-49 "C" frame
// and writes it to the private socket.
func (g *LiveGateway) Cancel(cid string) error {
	if g.gwConn == nil {
		return errGwDown
	}
	frame := wire.EncodeCancel(cid)
	ctx, cancel := context.WithTimeout(g.baseCtx, writeTimeout)
	defer cancel()
	if err := g.gwConn.Write(ctx, websocket.MessageText, []byte(frame)); err != nil {
		return fmt.Errorf("conn: cancel: %w", err)
	}
	return nil
}

// readGw is the private socket's sole reader (coder/websocket requires a
// single reader per connection). It echoes heartbeats, folds every other
// frame through the shared Folder, and emits feed.GwDown once the read
// errors (peer closed, ctx canceled, or a protocol violation).
func (g *LiveGateway) readGw(ctx context.Context) {
	for {
		_, data, err := g.gwConn.Read(ctx)
		if err != nil {
			g.events <- feed.GwDown{}
			return
		}
		text := string(data)
		if wire.IsHeartbeat(text) {
			g.writeGwText(ctx, text)
			continue
		}

		g.folderMu.Lock()
		event, ok := g.folder.Fold(text, time.Now())
		g.folderMu.Unlock()
		if ok {
			g.events <- event
		}
	}
}

// readMd is the marketdata socket's sole reader. It decodes each binary
// protobuf MdFrame, echoes the server's heartbeat back as a "H" control
// frame (the marketdata server treats that echo as this client's liveness
// signal — records.rs update_heartbeat), and emits every other decoded
// frame (Bbo/Snapshot/Delta/MdTrade) directly, matching the shapes
// ui/update.go already folds. A malformed frame is skipped, never fatal —
// the feed is best-effort and un-authenticated (specs/2/4-cast.md §10.4
// trust-boundary rationale extends to its public sibling here).
func (g *LiveGateway) readMd(ctx context.Context) {
	for {
		_, data, err := g.mdConn.Read(ctx)
		if err != nil {
			g.events <- feed.MdDown{}
			return
		}
		decoded, err := wire.DecodeMd(data)
		if err != nil {
			continue
		}
		if hb, ok := decoded.(wire.MdHeartbeat); ok {
			g.writeMdText(ctx, fmt.Sprintf(`{"H":[%d]}`, hb.TsMs))
			continue
		}
		g.events <- decoded
	}
}

// writeGwText is a best-effort write on the private socket (heartbeat
// echo): a failure here is silently observed a moment later by readGw's
// own Read returning an error, which is the single place link-down is
// reported.
func (g *LiveGateway) writeGwText(ctx context.Context, text string) {
	wctx, cancel := context.WithTimeout(ctx, writeTimeout)
	defer cancel()
	_ = g.gwConn.Write(wctx, websocket.MessageText, []byte(text))
}

// writeMdText is writeGwText's marketdata-socket counterpart.
func (g *LiveGateway) writeMdText(ctx context.Context, text string) {
	wctx, cancel := context.WithTimeout(ctx, writeTimeout)
	defer cancel()
	_ = g.mdConn.Write(wctx, websocket.MessageText, []byte(text))
}
