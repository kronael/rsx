# rsx-tui Architecture

A testable core (`lib.rs`) plus a thin binary (`main.rs`). The binary
owns the terminal and the event loop; everything else — UI state, event
folding, key handling, rendering, and the gateway transport — lives in
the library so a full session runs headless under a ratatui `TestBackend`
and a `MockConn`.

## Modules

| Module | Role |
|--------|------|
| `conn.rs` | The `GatewayConn` trait (submit an order, poll inbound events), the domain types (`OrderReq`, `GwEvent`, `Side`, `Tif`), and `MockConn` (in-memory transport for tests and the offline demo). |
| `quic.rs` | `QuicConn` — the sole live transport. A quinn client on a background tokio runtime, bridged to the synchronous `GatewayConn` API with channels. |
| `wire.rs` | The protobuf frame codec: the `WireHello` auth first-frame, `OrderReq`↔`WireOrder` (with `cid` + `symbol`), `GwEvent`↔`WireEvent`, over length-delimited frames. |
| `app.rs` | `App` — pure UI state; `apply_event` folds one `GwEvent` in, `drain` pumps the transport each tick. No terminal, no socket. |
| `input.rs` | `handle_key` — pure over (`App`, key, `conn`). |
| `render.rs` | `draw` — pure over `&App` into any backend (book ladder, order entry, positions, trade tape, latency strip). |

## Transport: protobuf-over-QUIC

`GatewayConn` is synchronous (the UI drains it non-blocking each render
tick) but quinn is async, so `QuicConn` owns a single-worker tokio
runtime on a dedicated thread and bridges with channels:

- on connect the task sends the auth first-frame (`WireHello`: JWT +
  user id) before any order.
- `submit` pushes an `OrderReq` onto an unbounded channel; the async
  task drains it, stamps a correlation id + the session symbol, and
  writes an order frame.
- the async task reads event frames and pushes each `GwEvent` onto a std
  mpsc channel; `poll_event` drains it with `try_recv`.

One connection, one bidirectional stream, one framed read loop — no
multiplexing. `Connected`/`Disconnected` are synthesized locally by the
transport (stream open / any failure), never sent over the wire.

### Wire format

Length-delimited frames: a 4-byte big-endian length prefix, then a
protobuf body encoded by `prost`. The schema (`wire.rs`) is hand-derived
`prost::Message`/`prost::Oneof` structs — no `.proto`, no `prost-build`,
no `protoc`. `wire_test.rs` pins the exact encoded bytes of each message
so an accidental tag/field change fails a test. It is deliberately
minimal: exactly the fields the TUI submits and renders.

The client→server stream is: one auth first-frame, then order frames.

- **Auth first-frame (client → server):** `WireHello { jwt, user }` — an
  HS256 JWT minted for the session user id, sent once before any order.
  The client carries identity in-band; the gateway MUST validate it
  (server-side follow-up, see "Server side").
- **Client → server:** `WireOrder { cid, symbol, side, price, qty, tif }`.
  `cid` is a client correlation id (a per-connection monotonic counter);
  `symbol` is the instrument (the TUI is single-market, so it is a
  per-session constant, not part of the UI's `OrderReq`).
- **Server → client:** `WireEvent`, a `oneof` over the wire event kinds —
  `Book`, `Trade`, `Accepted`, `Fill`, `Done`, `Rejected`, `Position`,
  `Latency`. These mirror `GwEvent` one-to-one. Unknown `side`/`tif`
  values error the decode rather than coercing to a default.

The `Latency` frame echoes the order's `cid` and carries the
server-stamped `internal` (casting GW→Risk→ME→Risk→GW) and `engine`
(match + risk) legs; the client pairs it to the submitted order by `cid`
(exact even with several in flight) and fills the `net` (client↔gateway)
leg from its own measured round-trip.

## Server side (not built here)

No gateway speaks this protobuf-over-QUIC wire yet. This crate is the
**client half** only; it is proven end-to-end against a loopback quinn
server in `tests/quic_test.rs`, which is the reference for what a real
gateway endpoint would implement. Standing up that endpoint — a QUIC
listener that bridges these frames to the existing order-routing path,
**validates the `WireHello` auth first-frame** (the client already mints
and sends the JWT), and distributes the gateway cert — is a follow-up,
out of scope for the client crate.
