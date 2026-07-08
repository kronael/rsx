# 58 — HTTP/3 (QUIC) Transport Binding

Status: **spec** (not implemented). The QUIC / HTTP-3 transport binding for
the [49-webproto.md](49-webproto.md) protobuf protocol — the gateway leg of
README §Roadmap step 4 (native terminal QUIC needs a gateway QUIC
listener). **Transport only:** this spec maps the target protobuf messages
onto QUIC; it does not define or redefine any message (49 owns the schema).

## Why

The protocol is protobuf; WebSocket is the interoperable floor (49
§Transports). QUIC adds properties the order path wants that WS cannot give:

- **Connection migration.** A QUIC connection survives NAT rebinding and
  IP changes (roaming/mobile traders) without a reconnect + re-auth.
- **1-RTT / 0-RTT auth.** Session resumption lets a returning client
  re-establish with the auth first-frame in the first flight.
- **Unreliable datagrams.** Droppable feed data (BBO / L2 / trade) can ride
  QUIC DATAGRAM frames instead of a reliable stream — drops are safe under
  supersession (`GUARANTEES.md` §1.4), and head-of-line blocking on the
  reliable stream no longer stalls fresh quotes.
- **Browser reach via WebTransport.** Browsers reach the same protobuf
  protocol over HTTP/3 WebTransport, with no JSON/WS shim.

`rsx-term` currently speaks WebSocket: JSON for private orders/events and
protobuf for public marketdata. The gap is both the terminal QUIC client
binding and a gateway QUIC listener that terminates it and validates the
auth first-frame.

## Scope

**In scope:** the QUIC stream / datagram mapping for the 49 frames; the
auth-first-frame flow over QUIC including 0-RTT interplay; WebTransport over
HTTP/3 for browsers; the gateway QUIC listener (quinn/tokio) that validates
`WireHello` and binds the connection to `user`.

**Out of scope:** the message schema (owned by 49 — do NOT redefine it
here); the WebSocket binding (49 §Transports); kernel-bypass / SQPOLL edge
scaling ([56-network-edge-scaling.md](56-network-edge-scaling.md)); multi-DC
QUIC; a full marketdata QUIC leg (noted only where it shares this spec's
runtime question).

## Design

- **One connection per client.** A QUIC connection mirrors one WS
  connection. A single client-initiated **bidirectional stream** carries
  the 49 frame sequence exactly as framed for WS:
  `[u32 BE len][protobuf body]`. QUIC delimits at the stream layer, but the
  length prefix stays — the frame format is transport-independent, so the
  same `rsx-tui/src/wire.rs` `read_frame`/`write_frame` parser serves both
  transports unchanged.
- **Auth first-frame.** The first frame on the bidi stream is `WireHello`
  (49). The gateway validates the JWT (HS256, claims per `11-gateway.md`)
  and binds the connection to `user` **before** accepting any `WireOrder`.
  This is the `rsx-tui/src/quic.rs` + `54-tui-access.md` identity model,
  transport-bound to QUIC.
- **0-RTT interplay.** 0-RTT / resumption is opt-in. Only `WireHello` may
  ride a 0-RTT flight (auth is idempotent — a replayed hello is harmless).
  A `WireOrder` MUST NOT ride 0-RTT: 0-RTT data is replayable, and while ME
  WAL dedup (`GUARANTEES.md` §1.0) would collapse a replayed order, keeping
  orders off 0-RTT removes the hazard at the transport instead of leaning
  on the dedup as the only guard. Orders wait for the handshake to
  complete (1-RTT).
- **Streams vs datagrams.** Order/event frames (`accepted` / `fill` /
  `done` — not drop-safe, `GUARANTEES.md` §1.0) stay on the reliable bidi
  stream. Droppable feed data (BBO / L2 delta / trade — best-effort,
  §1.4) MAY ride unreliable QUIC DATAGRAM frames, since supersession makes
  a lost datagram safe and avoids stream head-of-line blocking. This is the
  same reliable/best-effort split the marketdata feed could adopt.
- **WebTransport for browsers.** A browser cannot open a raw QUIC bidi
  stream; it opens a **WebTransport session over HTTP/3**, and each
  WebTransport bidi stream carries the same length-prefixed 49 frames. This
  is how the deferred web terminal (`54-tui-access.md` web path) and any
  browser client reach the protobuf protocol directly.
- **Gateway QUIC listener — the runtime question.** The listener is
  **quinn on tokio** (TLS via rustls; cert/name per `54-tui-access.md`
  `RSX_GW_CERT` / `RSX_GW_SERVER_NAME`). The gateway's monoio reactor owns
  the WS edge (`11-gateway.md` Runtime Model), but **monoio has no native
  QUIC**, so the QUIC endpoint runs on a quinn/tokio runtime alongside it.
  This is the **same quinn/tokio-vs-monoio question as the marketdata QUIC
  leg** — resolve it once for both edges.

## Success criteria

- A native client (`rsx-tui`) and a browser (WebTransport) drive the **same
  gateway endpoint** with **byte-identical** 49 frames — the `wire_test.rs`
  golden bytes are unchanged by adding this transport.
- The auth first-frame gates the connection: an invalid/missing `WireHello`
  closes the stream/connection before any order is processed.
- 0-RTT carries only `WireHello`; a `WireOrder` presented in a 0-RTT flight
  is refused (accepted only after 1-RTT).
- Feed data over datagrams tolerates loss without a client-visible gap
  error; order/event frames over the reliable stream are never dropped.
- No change to the 49 message schema — this spec adds a transport binding
  only.

## Current state baseline

- **Client:** `rsx-term` currently uses WebSocket
  (`rsx-term/conn/live.go`, `rsx-term/wire/order.go`,
  `rsx-term/wire/md.go`). No gateway terminates QUIC yet.
- **Gateway:** monoio WS reactor only (`11-gateway.md`). No QUIC listener,
  no WebTransport, no datagram path.
- **Marketdata:** WS feed only (`16-marketdata.md`); any QUIC/datagram leg
  is the same quinn/tokio runtime question as above.
