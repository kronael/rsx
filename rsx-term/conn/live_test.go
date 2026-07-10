package conn

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"

	"rsx-term/feed"
	"rsx-term/wire"
)

// putVarint appends v as a base-128 protobuf varint to buf.
func putVarint(buf []byte, v uint64) []byte {
	for v >= 0x80 {
		buf = append(buf, byte(v)|0x80)
		v >>= 7
	}
	return append(buf, byte(v))
}

// putTag appends a protobuf field tag (field<<3 | wireType).
func putTag(buf []byte, field, wireType uint64) []byte {
	return putVarint(buf, field<<3|wireType)
}

// encodeBboFrame builds a minimal MdFrame binary carrying a Bbo (oneof tag
// 1), matching wire.DecodeMd's [len:u32 BE][body] framing — a test double
// for the real Rust encoder (rsx-marketdata/src/wire.rs), not a general
// protobuf encoder.
func encodeBboFrame(symbolID uint32, bidPx, askPx int64, seq uint64) []byte {
	var inner []byte
	inner = putTag(inner, 1, 0)
	inner = putVarint(inner, uint64(symbolID))
	inner = putTag(inner, 2, 0)
	inner = putVarint(inner, uint64(bidPx))
	inner = putTag(inner, 5, 0)
	inner = putVarint(inner, uint64(askPx))
	inner = putTag(inner, 9, 0)
	inner = putVarint(inner, seq)
	return wrapMdFrame(1, inner)
}

// encodeHeartbeatFrame builds a minimal MdFrame binary carrying a
// Heartbeat (oneof tag 5).
func encodeHeartbeatFrame(tsMs uint64) []byte {
	var inner []byte
	inner = putTag(inner, 1, 0)
	inner = putVarint(inner, tsMs)
	return wrapMdFrame(5, inner)
}

// wrapMdFrame wraps inner as oneof field oneofTag on the MdFrame envelope,
// then applies the [len:u32 BE][body] outer framing.
func wrapMdFrame(oneofTag uint64, inner []byte) []byte {
	var outer []byte
	outer = putTag(outer, oneofTag, 2)
	outer = putVarint(outer, uint64(len(inner)))
	outer = append(outer, inner...)

	frame := make([]byte, 4, 4+len(outer))
	binary.BigEndian.PutUint32(frame, uint32(len(outer)))
	return append(frame, outer...)
}

// wsURL rewrites an httptest server's http:// URL to ws://.
func wsURL(httpURL string) string {
	return "ws" + strings.TrimPrefix(httpURL, "http")
}

// drainUntil reads the next event and fails the test unless it equals want.
func drainUntil(t *testing.T, events <-chan any, want any) {
	t.Helper()
	select {
	case got := <-events:
		if got != want {
			t.Fatalf("event = %#v, want %#v", got, want)
		}
	case <-time.After(2 * time.Second):
		t.Fatalf("timed out waiting for %#v", want)
	}
}

// recvTyped reads the next event and fails the test unless it has type T.
func recvTyped[T any](t *testing.T, events <-chan any) T {
	t.Helper()
	select {
	case got := <-events:
		v, ok := got.(T)
		if !ok {
			t.Fatalf("event = %#v, want type %T", got, *new(T))
		}
		return v
	case <-time.After(2 * time.Second):
		t.Fatalf("timed out waiting for event")
	}
	var zero T
	return zero
}

// TestLiveGatewayFoldsFrames drives a LiveGateway against two in-process
// httptest WS servers standing in for the real gateway and marketdata
// processes, and asserts the frame -> tea.Msg path matches what
// conn.DemoScript() replays for the mock: a B/BBO decode, a heartbeat
// echo on each socket, and a full order lifecycle (submit -> accepted ->
// fill). No live cluster required.
func TestLiveGatewayFoldsFrames(t *testing.T) {
	const symbolID = 7
	const oidHex = "00000000000000ff" // last 16 hex chars -> OidTo64 == 0xff (255)

	gwConnCh := make(chan *websocket.Conn, 1)
	gwAuthCh := make(chan string, 1)
	gwDone := make(chan struct{})
	gwSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gwAuthCh <- r.Header.Get("Authorization")
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			t.Errorf("gw accept: %v", err)
			return
		}
		gwConnCh <- c
		<-gwDone
	}))
	defer gwSrv.Close()
	defer close(gwDone)

	mdConnCh := make(chan *websocket.Conn, 1)
	mdDone := make(chan struct{})
	mdSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			t.Errorf("md accept: %v", err)
			return
		}
		mdConnCh <- c
		<-mdDone
	}))
	defer mdSrv.Close()
	defer close(mdDone)

	live := NewLiveGateway(wsURL(gwSrv.URL), wsURL(mdSrv.URL), "test-secret-at-least-32-bytes-long!", 42, symbolID)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	if err := live.Connect(ctx); err != nil {
		t.Fatalf("connect: %v", err)
	}
	defer live.Close()

	if auth := <-gwAuthCh; !strings.HasPrefix(auth, "Bearer ") {
		t.Fatalf("auth header = %q, want Bearer prefix", auth)
	}
	gwServerConn := <-gwConnCh
	mdServerConn := <-mdConnCh

	events := live.Events()
	drainUntil(t, events, feed.GwUp{})
	drainUntil(t, events, feed.MdUp{})

	// The client sent its subscribe control frame as part of Connect.
	typ, sub, err := mdServerConn.Read(ctx)
	if err != nil || typ != websocket.MessageText {
		t.Fatalf("md subscribe read: typ=%v err=%v", typ, err)
	}
	if string(sub) != `{"S":[7,7]}` {
		t.Fatalf("md subscribe = %s, want {\"S\":[7,7]}", sub)
	}

	// Bbo frame -> wire.Bbo event.
	if err := mdServerConn.Write(ctx, websocket.MessageBinary, encodeBboFrame(symbolID, 9_998, 10_002, 5)); err != nil {
		t.Fatalf("write bbo: %v", err)
	}
	bbo := recvTyped[wire.Bbo](t, events)
	if bbo.SymbolID != symbolID || bbo.BidPx != 9_998 || bbo.AskPx != 10_002 || bbo.Seq != 5 {
		t.Fatalf("bbo = %+v", bbo)
	}

	// Marketdata heartbeat -> client echoes {"H":[ts]} back on the md socket.
	if err := mdServerConn.Write(ctx, websocket.MessageBinary, encodeHeartbeatFrame(123)); err != nil {
		t.Fatalf("write md heartbeat: %v", err)
	}
	typ, echo, err := mdServerConn.Read(ctx)
	if err != nil || typ != websocket.MessageText || string(echo) != `{"H":[123]}` {
		t.Fatalf("md heartbeat echo: typ=%v echo=%s err=%v", typ, echo, err)
	}

	// Order lifecycle: Submit writes an "N" frame; the server accepts and
	// fills it; the client folds both into wire.Accepted / wire.Fill.
	if err := live.Submit(wire.OrderReq{Side: wire.Buy, Px: 9_998, Qty: 5, Tif: wire.Gtc}); err != nil {
		t.Fatalf("submit: %v", err)
	}
	typ, nFrame, err := gwServerConn.Read(ctx)
	if err != nil || typ != websocket.MessageText {
		t.Fatalf("gw read N frame: typ=%v err=%v", typ, err)
	}
	var decodedN map[string][]any
	if err := json.Unmarshal(nFrame, &decodedN); err != nil || decodedN["N"] == nil {
		t.Fatalf("N frame = %s (unmarshal err %v)", nFrame, err)
	}

	if err := gwServerConn.Write(ctx, websocket.MessageText, []byte(`{"U":["`+oidHex+`",1,0,5,0]}`)); err != nil {
		t.Fatalf("write accept: %v", err)
	}
	accepted := recvTyped[wire.Accepted](t, events)
	if accepted.Oid != 0xff {
		t.Fatalf("accepted.Oid = %#x, want 0xff", accepted.Oid)
	}
	if accepted.RttNs < 0 {
		t.Fatalf("accepted.RttNs = %d, want a measured (non-negative) RTT", accepted.RttNs)
	}

	if err := gwServerConn.Write(ctx, websocket.MessageText, []byte(`{"F":["`+oidHex+`","0000000000000000",9998,5,0,0]}`)); err != nil {
		t.Fatalf("write fill: %v", err)
	}
	fill := recvTyped[wire.Fill](t, events)
	if fill.Oid != 0xff || fill.Px != 9998 || fill.Qty != 5 || fill.Side != wire.Buy {
		t.Fatalf("fill = %+v", fill)
	}

	// Gateway heartbeat -> client echoes verbatim on the gw socket.
	if err := gwServerConn.Write(ctx, websocket.MessageText, []byte(`{"H":[42]}`)); err != nil {
		t.Fatalf("write gw heartbeat: %v", err)
	}
	typ, hbEcho, err := gwServerConn.Read(ctx)
	if err != nil || typ != websocket.MessageText || string(hbEcho) != `{"H":[42]}` {
		t.Fatalf("gw heartbeat echo: typ=%v echo=%s err=%v", typ, hbEcho, err)
	}
}

// TestLiveGatewayReconnectsGwWithBackoff drops the private socket after
// connect and asserts the reader redials: feed.GwDown fires, a second
// dial arrives with a freshly minted JWT (not the same token replayed),
// and feed.GwUp fires again once the new socket is up. Exercises the
// auto-reconnect path with real (if short) backoff delay — no live
// cluster required.
func TestLiveGatewayReconnectsGwWithBackoff(t *testing.T) {
	const symbolID = 7

	gwConnCh := make(chan *websocket.Conn, 4)
	gwAuthCh := make(chan string, 4)
	gwDone := make(chan struct{})
	gwSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gwAuthCh <- r.Header.Get("Authorization")
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			t.Errorf("gw accept: %v", err)
			return
		}
		gwConnCh <- c
		<-gwDone
	}))
	defer gwSrv.Close()
	defer close(gwDone)

	mdDone := make(chan struct{})
	mdSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			return
		}
		_, _, _ = c.Read(r.Context())
		<-mdDone
	}))
	defer mdSrv.Close()
	defer close(mdDone)

	live := NewLiveGateway(wsURL(gwSrv.URL), wsURL(mdSrv.URL), "test-secret-at-least-32-bytes-long!", 42, symbolID)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	if err := live.Connect(ctx); err != nil {
		t.Fatalf("connect: %v", err)
	}
	defer live.Close()

	auth1 := <-gwAuthCh
	firstConn := <-gwConnCh
	events := live.Events()
	drainUntil(t, events, feed.GwUp{})
	drainUntil(t, events, feed.MdUp{})

	// Simulate a dropped link: close the server's side of the socket.
	_ = firstConn.CloseNow()

	drainUntil(t, events, feed.GwDown{})
	auth2 := <-gwAuthCh
	<-gwConnCh
	drainUntil(t, events, feed.GwUp{})

	if auth1 == auth2 {
		t.Fatalf("reconnect replayed the same JWT %q, want a freshly minted token", auth1)
	}
}

// TestLiveGatewayReconnectsMdAndResubscribes drops the marketdata socket
// after connect and asserts the reader redials and re-sends the
// {"S":[...]} subscribe frame — a resubscribe is required since the
// gateway has no memory of a torn-down connection's subscription.
func TestLiveGatewayReconnectsMdAndResubscribes(t *testing.T) {
	const symbolID = 7

	gwDone := make(chan struct{})
	gwSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			return
		}
		<-gwDone
	}))
	defer gwSrv.Close()
	defer close(gwDone)

	mdConnCh := make(chan *websocket.Conn, 4)
	subCh := make(chan string, 4)
	mdDone := make(chan struct{})
	mdSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			t.Errorf("md accept: %v", err)
			return
		}
		_, sub, err := c.Read(r.Context())
		if err == nil {
			subCh <- string(sub)
		}
		mdConnCh <- c
		<-mdDone
	}))
	defer mdSrv.Close()
	defer close(mdDone)

	live := NewLiveGateway(wsURL(gwSrv.URL), wsURL(mdSrv.URL), "test-secret-at-least-32-bytes-long!", 42, symbolID)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	if err := live.Connect(ctx); err != nil {
		t.Fatalf("connect: %v", err)
	}
	defer live.Close()

	events := live.Events()
	drainUntil(t, events, feed.GwUp{})
	drainUntil(t, events, feed.MdUp{})

	sub1 := <-subCh
	firstConn := <-mdConnCh
	if sub1 != `{"S":[7,7]}` {
		t.Fatalf("initial subscribe = %s, want {\"S\":[7,7]}", sub1)
	}

	// Simulate a marketdata-only flap: the private socket is untouched.
	_ = firstConn.CloseNow()

	drainUntil(t, events, feed.MdDown{})
	sub2 := <-subCh
	<-mdConnCh
	drainUntil(t, events, feed.MdUp{})

	if sub2 != `{"S":[7,7]}` {
		t.Fatalf("resubscribe = %s, want {\"S\":[7,7]}", sub2)
	}
}

// TestLiveGatewayCloseStopsReconnect asserts Close() during an in-flight
// reconnect backoff ends the reader goroutine rather than redialing —
// Close must win a race with a pending reconnect attempt.
func TestLiveGatewayCloseStopsReconnect(t *testing.T) {
	const symbolID = 7

	gwConnCh := make(chan *websocket.Conn, 4)
	gwDialCount := make(chan struct{}, 8)
	gwSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gwDialCount <- struct{}{}
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			return
		}
		gwConnCh <- c
		<-r.Context().Done()
	}))
	defer gwSrv.Close()

	mdSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			return
		}
		_, _, _ = c.Read(r.Context())
		<-r.Context().Done()
	}))
	defer mdSrv.Close()

	live := NewLiveGateway(wsURL(gwSrv.URL), wsURL(mdSrv.URL), "test-secret-at-least-32-bytes-long!", 42, symbolID)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	if err := live.Connect(ctx); err != nil {
		t.Fatalf("connect: %v", err)
	}

	<-gwDialCount // initial dial
	firstConn := <-gwConnCh
	events := live.Events()
	drainUntil(t, events, feed.GwUp{})
	drainUntil(t, events, feed.MdUp{})

	_ = firstConn.CloseNow()
	drainUntil(t, events, feed.GwDown{})

	// Close immediately, before the backoff-delayed redial fires.
	live.Close()

	select {
	case <-gwDialCount:
		t.Fatalf("reconnect redialed after Close")
	case <-time.After(1 * time.Second):
	}
}

// TestLiveGatewaySubmitBeforeConnectErrors asserts Submit/Cancel fail
// (rather than nil-panicking) when the private socket never connected.
func TestLiveGatewaySubmitBeforeConnectErrors(t *testing.T) {
	live := NewLiveGateway("ws://127.0.0.1:0", "ws://127.0.0.1:0", "s", 1, 1)
	if err := live.Submit(wire.OrderReq{Px: 1, Qty: 1}); err == nil {
		t.Fatalf("submit before connect did not error")
	}
	if err := live.Cancel("c1"); err == nil {
		t.Fatalf("cancel before connect did not error")
	}
}

// TestLiveGatewayWatchSymbolsAndRouting: WatchSymbols adds peer md
// subscriptions (one S frame each), and Submit routes o.Symbol into the N
// frame (0 falls back to the connection's configured symbol).
func TestLiveGatewayWatchSymbolsAndRouting(t *testing.T) {
	gwConnCh := make(chan *websocket.Conn, 1)
	gwDone := make(chan struct{})
	gwSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			t.Errorf("gw accept: %v", err)
			return
		}
		gwConnCh <- c
		<-gwDone
	}))
	defer gwSrv.Close()
	defer close(gwDone)

	mdConnCh := make(chan *websocket.Conn, 1)
	mdDone := make(chan struct{})
	mdSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			t.Errorf("md accept: %v", err)
			return
		}
		mdConnCh <- c
		<-mdDone
	}))
	defer mdSrv.Close()
	defer close(mdDone)

	live := NewLiveGateway(wsURL(gwSrv.URL), wsURL(mdSrv.URL), "test-secret-at-least-32-bytes-long!", 42, 7)
	live.WatchSymbols([]uint32{11, 7}) // 7 dedupes (already primary)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	if err := live.Connect(ctx); err != nil {
		t.Fatalf("connect: %v", err)
	}
	defer live.Close()

	gwServerConn := <-gwConnCh
	mdServerConn := <-mdConnCh
	for _, want := range []string{`{"S":[7,7]}`, `{"S":[11,7]}`} {
		_, sub, err := mdServerConn.Read(ctx)
		if err != nil {
			t.Fatalf("md subscribe read: %v", err)
		}
		if string(sub) != want {
			t.Fatalf("md subscribe = %s, want %s", sub, want)
		}
	}

	if err := live.Submit(wire.OrderReq{Symbol: 11, Side: wire.Buy, Px: 5, Qty: 6}); err != nil {
		t.Fatalf("submit: %v", err)
	}
	_, frame, err := gwServerConn.Read(ctx)
	if err != nil {
		t.Fatalf("gw read: %v", err)
	}
	if !strings.HasPrefix(string(frame), `{"N":[11,`) {
		t.Fatalf("order should route to symbol 11: %s", frame)
	}

	if err := live.Submit(wire.OrderReq{Side: wire.Sell, Px: 5, Qty: 6}); err != nil {
		t.Fatalf("submit default: %v", err)
	}
	_, frame, err = gwServerConn.Read(ctx)
	if err != nil {
		t.Fatalf("gw read: %v", err)
	}
	if !strings.HasPrefix(string(frame), `{"N":[7,`) {
		t.Fatalf("symbol 0 should fall back to the configured symbol: %s", frame)
	}
}
