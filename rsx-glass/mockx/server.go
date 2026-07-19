package mockx

import (
	"context"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"sync"
	"time"

	"github.com/coder/websocket"
)

// Server is the mock market-data exchange: an HTTP/WebSocket server that
// behaves like the RSX marketdata daemon for one symbol. A subscriber dials it,
// sends the real `{"S":[symbol_id,channels]}` subscribe frame, and the server
// replays the scripted Scenario as BINARY MdFrame frames in the real wire
// format (see wire.go), interleaves heartbeats, and accepts the client's
// `{"H":[ts]}` heartbeat echo — exactly the handshake conn/live.go's readMd
// performs against the real feed.
//
// It is a test fixture: a single package-local server, not a second exchange.
type Server struct {
	// URL is the ws:// address a subscriber dials (ready once New returns).
	URL string

	opts     Options
	http     *http.Server
	listener net.Listener

	mu       sync.Mutex
	subs     []Sub
	echoes   int // count of {"H":[ts]} heartbeat echoes received from clients
	connSeen int // accepted connections so far (drives DropOnce)
}

// Sub is one recorded subscribe request: the symbol and channel mask a client
// asked for.
type Sub struct {
	SymbolID uint32
	Channels int
}

// Options configures the scripted scenario and the fault injection the
// integration suite drives.
type Options struct {
	// SymbolID is the symbol the scenario trades (0 -> DefaultSymbolID).
	SymbolID uint32
	// Seed varies the scenario's trade sizes (structure is seed-independent).
	Seed uint64
	// InjectMalformed sends one truncated (undecodable) binary frame right
	// after the opening snapshot, to exercise the untrusted-feed skip: a
	// malformed frame must be dropped, never fatal, and the stream continues.
	InjectMalformed bool
	// DropOnce closes the FIRST subscriber connection right after the scenario
	// replay (forcing a reconnect); later connections replay and stay open.
	DropOnce bool
	// KeepAliveHeartbeat, when > 0, sends a heartbeat every interval after the
	// scenario so the link stays up (and the echo path keeps exercising).
	KeepAliveHeartbeat time.Duration
}

// New binds a mock server on a free loopback port and starts serving. Close
// stops it.
func New(opts Options) (*Server, error) {
	if opts.SymbolID == 0 {
		opts.SymbolID = DefaultSymbolID
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return nil, fmt.Errorf("mockx: listen: %w", err)
	}
	s := &Server{
		URL:      "ws://" + listener.Addr().String() + "/md",
		opts:     opts,
		listener: listener,
	}
	mux := http.NewServeMux()
	mux.HandleFunc("/md", s.handle)
	s.http = &http.Server{Handler: mux}
	go s.serve()
	return s, nil
}

// serve runs the HTTP server until Close; a closed-listener error is the normal
// stop, not a failure.
func (s *Server) serve() {
	// Serve returns ErrServerClosed on Close — expected shutdown, not an error.
	_ = s.http.Serve(s.listener)
}

// Close shuts the server down, dropping any live connection.
func (s *Server) Close() {
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	// Best-effort shutdown; the process is exiting the test either way.
	_ = s.http.Shutdown(ctx)
}

// Subscriptions returns the subscribe frames clients have sent so far.
func (s *Server) Subscriptions() []Sub {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]Sub(nil), s.subs...)
}

// HeartbeatEchoes returns how many {"H":[ts]} echoes clients have sent back —
// proof the server's heartbeats reached a client that answered them.
func (s *Server) HeartbeatEchoes() int {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.echoes
}

// handle upgrades one connection, reads its subscribe frame, replays the
// scenario, and (unless dropped) keeps the link alive while draining the
// client's heartbeat echoes.
func (s *Server) handle(w http.ResponseWriter, r *http.Request) {
	conn, err := websocket.Accept(w, r, nil)
	if err != nil {
		return
	}
	ctx := r.Context()

	s.mu.Lock()
	s.connSeen++
	drop := s.opts.DropOnce && s.connSeen == 1
	s.mu.Unlock()

	sub, ok := s.readSubscribe(ctx, conn)
	if !ok {
		_ = conn.CloseNow()
		return
	}
	s.mu.Lock()
	s.subs = append(s.subs, sub)
	s.mu.Unlock()

	// Drain the client's frames (heartbeat echoes) concurrently with the
	// replay — coder/websocket allows one reader + one writer at a time.
	go s.drainEchoes(ctx, conn)

	s.replay(ctx, conn)

	if drop {
		_ = conn.CloseNow() // force the subscriber's reconnect path
		return
	}
	s.keepAlive(ctx, conn)
}

// readSubscribe reads the first client TEXT frame and parses the real
// `{"S":[symbol_id,channels]}` subscribe shape.
func (s *Server) readSubscribe(ctx context.Context, conn *websocket.Conn) (Sub, bool) {
	readCtx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	typ, data, err := conn.Read(readCtx)
	if err != nil || typ != websocket.MessageText {
		return Sub{}, false
	}
	var frame struct {
		S []int64 `json:"S"`
	}
	if err := json.Unmarshal(data, &frame); err != nil || len(frame.S) < 2 {
		return Sub{}, false
	}
	return Sub{SymbolID: uint32(frame.S[0]), Channels: int(frame.S[1])}, true
}

// replay sends every scenario frame as a BINARY MdFrame. With InjectMalformed
// set it slips one truncated frame in right after the opening snapshot.
func (s *Server) replay(ctx context.Context, conn *websocket.Conn) {
	for i, msg := range Scenario(s.opts.SymbolID, s.opts.Seed) {
		if err := s.send(ctx, conn, Encode(msg)); err != nil {
			return
		}
		if s.opts.InjectMalformed && i == 0 {
			// A 4-byte length prefix claiming 99 bytes over a 3-byte body:
			// wire.DecodeMd rejects it as truncated, and the subscriber must
			// skip it and keep folding the frames that follow.
			if err := s.send(ctx, conn, []byte{0, 0, 0, 99, 0x01, 0x02, 0x03}); err != nil {
				return
			}
		}
	}
}

// keepAlive holds the connection open after the replay, sending heartbeats at
// the configured interval so the subscriber stays MdUp; it returns when the
// context ends or a write fails (client gone).
func (s *Server) keepAlive(ctx context.Context, conn *websocket.Conn) {
	if s.opts.KeepAliveHeartbeat <= 0 {
		<-ctx.Done()
		return
	}
	ticker := time.NewTicker(s.opts.KeepAliveHeartbeat)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			hb := EncodeHeartbeat(uint64(time.Now().UnixMilli()))
			if err := s.send(ctx, conn, hb); err != nil {
				return
			}
		}
	}
}

// drainEchoes reads client frames until the connection ends, counting the
// `{"H":[ts]}` heartbeat echoes.
func (s *Server) drainEchoes(ctx context.Context, conn *websocket.Conn) {
	for {
		typ, data, err := conn.Read(ctx)
		if err != nil {
			return
		}
		if typ != websocket.MessageText {
			continue
		}
		var frame struct {
			H []int64 `json:"H"`
		}
		if err := json.Unmarshal(data, &frame); err == nil && len(frame.H) > 0 {
			s.mu.Lock()
			s.echoes++
			s.mu.Unlock()
		}
	}
}

// send writes one BINARY frame with a short deadline so a stalled peer never
// wedges the server.
func (s *Server) send(ctx context.Context, conn *websocket.Conn, data []byte) error {
	writeCtx, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	return conn.Write(writeCtx, websocket.MessageBinary, data)
}
