---
status: partial
---

# Client Wire Protocol

Current client state: `rsx-term` sends private order/event JSON text over
WebSocket to the gateway, and consumes public marketdata protobuf binary
frames over WebSocket from marketdata. A unified protobuf order channel and
QUIC / HTTP-3 bindings are roadmap items; this spec captures that target
schema and the marketdata protobuf that exists today.

Source of truth is the proven, hand-derived prost schema in the client and
feed codecs — not this prose. When they disagree, the code wins and this
spec is the bug:

- Current order / event channel: `rsx-term/wire/order.go`.
- Target unified protobuf order/event schema: this document.
- Public market-data feed: `rsx-marketdata/marketdata.proto` (canonical
  schema), encoder `rsx-marketdata/src/wire.rs`, Python decoder
  `rsx-playground/md_wire.py`, Go decoder `rsx-term/wire/md.go`, live
  connector `rsx-term/conn/live.go`.

## Table of Contents

- [Protocol vs transport](#protocol-vs-transport)
- [Encoding rules](#encoding-rules)
- [Enums](#enums)
- [Messages](#messages)
  - [Client → server](#client--server)
  - [Server → client (order / event channel)](#server--client-order--event-channel)
  - [Public market-data feed](#public-market-data-feed)
  - [Post-MVP query messages](#post-mvp-query-messages)
- [Transports](#transports)
  - [WebSocket (live)](#websocket-live)
  - [QUIC / HTTP-3 (roadmap)](#quic--http-3-roadmap)
- [ACK / retry semantics](#ack--retry-semantics)
- [Removed: JSON single-letter frames](#removed-json-single-letter-frames)
- [Cross-references](#cross-references)

---

## Protocol vs transport

**The protobuf messages are the protocol. WebSocket and QUIC/HTTP-3 are
interchangeable transports.** A client that can encode/decode the messages
below and frame them speaks RSX regardless of which pipe carries the bytes.

**One framing, every transport:**

```
[u32 big-endian length][protobuf body]
```

- Length is the body length in bytes, big-endian, excluding the 4-byte
  prefix (`rsx-tui/src/wire.rs` `write_frame`/`read_frame`).
- Bodies over `MAX_FRAME` (1 MiB) are rejected on both read and write — a
  corrupt/hostile length never triggers a large allocation.
- WebSocket and QUIC already delimit messages, so the prefix is redundant
  on both; it is kept deliberately so the frame format is
  transport-independent (one parser, any transport).

The schema is hand-derived `prost::Message` / `prost::Oneof` structs — no
`.proto` compiled by `protoc`, no `prost-build` on the build path. Both
peers implement matching structs; `wire_test.rs` and the marketdata
`wire_test.rs` pin the exact encoded bytes so an accidental tag or field
change fails a test rather than silently breaking the wire.

## Encoding rules

- **Fixed-point i64.** All prices and quantities are `int64` in tick / lot
  units (`price`, `qty`, `px`). Conversion to human units happens at the
  client's display boundary using the symbol's tick/lot size — never on
  the wire, never a float.
- **proto3 zero-omission.** A zero-valued scalar is omitted on the wire; a
  decoder MUST treat an absent scalar as `0`. The feed decoders rely on
  this (`Delta.side == 0` ⇒ bid, `Delta.qty == 0` ⇒ level removal,
  `Bbo.bid_px == 0` ⇒ empty side). See `marketdata.proto` header comment.
- **Strict enum decode.** `side` and `tif` are decoded through explicit
  match arms that **error on an unknown value** rather than coercing to a
  default (`rsx-tui/src/wire.rs` `side_from_i32` / `tif_from_i32`). A
  garbled or future-versioned frame fails the read instead of silently
  trading the wrong direction.
- **Tags are stable, append-only.** Appending a field is compatible;
  renumbering a tag is a breaking change. New message kinds are additive
  oneof variants (new tags), never a renumber.
- **Identifiers.** `cid` is the client correlation id (client-assigned,
  echoed back so the client pairs a response to its order); `oid` is the
  server order id. Canonical semantics: `cid` = 20-char string, `oid` =
  UUIDv7 (16 bytes). The proven `rsx-tui` schema currently narrows **both
  to `uint64`** (a single-market demo-client simplification) — see the
  reconciliation note under [Client → server](#client--server).

## Enums

Wire values are the small integers the schema actually encodes.

### Side
- `0` = BUY
- `1` = SELL

Used by `WireOrder.side`, `Trade.side`, `Fill.side` (order channel) and
`Delta.side` / `Trade.taker_side` (feed).

### Time in Force (`tif`)
- `0` = GTC
- `1` = IOC
- `2` = FOK

### Order Status
- `0` = FILLED
- `1` = RESTING
- `2` = CANCELLED
- `3` = FAILED

The order/event channel encodes terminal status as the **event kind**, not
a status int: `Accepted` (resting/accepted), `Done` (terminal:
filled/cancelled), `Rejected` (failed). This enum is the canonical status
vocabulary those events map onto.

### Failure Reason
- `0` = INVALID_TICK_SIZE
- `1` = INVALID_LOT_SIZE
- `2` = SYMBOL_NOT_FOUND
- `3` = DUPLICATE_ORDER_ID
- `4` = INSUFFICIENT_MARGIN
- `5` = OVERLOADED
- `6` = INTERNAL_ERROR
- `7` = REDUCE_ONLY_VIOLATION
- `8` = POST_ONLY_REJECT
- `9` = RATE_LIMIT
- `10` = TIMEOUT
- `11` = USER_IN_LIQUIDATION
- `12` = WRONG_SHARD

Risk-reject mapping (casting → wire): `InsufficientMargin` →
INSUFFICIENT_MARGIN, `UserInLiquidation` → USER_IN_LIQUIDATION, `NotInShard`
→ WRONG_SHARD. This is the canonical reason set; the `Rejected` event
currently carries the reason as a **free string** (see that message), not
this enum tag.

## Messages

### Client → server

**`WireHello` — auth first-frame.** Sent once, before any order. Carries
identity in-band; the gateway validates the JWT and binds the connection to
`user` before accepting any `WireOrder` (`rsx-tui/src/wire.rs` `write_hello`
/ `read_hello`; identity model in `54-tui-access.md`).

```proto
message WireHello {
  string jwt  = 1;   // HS256 session token
  uint32 user = 2;   // user id the token claims
}
```

**`WireOrder` — new order.** `cid` is the client correlation id (echoed on
`Latency`); `symbol` is the instrument (stamped per order — the TUI is
single-market, so it is a per-session constant there).

```proto
message WireOrder {
  uint64 cid    = 1;   // client correlation id
  uint32 symbol = 2;   // symbol id
  int32  side   = 3;   // Side enum
  int64  price  = 4;   // tick units
  int64  qty    = 5;   // lot units
  int32  tif    = 6;   // Time in Force enum
}
```

> **Reconciliation (owner decision).** The canonical protocol has `cid` =
> 20-char string and `oid` = UUIDv7 (16 bytes), and a new-order frame with
> **reduce-only** / **post-only** attributes. The proven `WireOrder`
> encodes `cid` as `uint64` and has **no `ro`/`po` fields** (tags 7/8 are
> free for them). Reduce-only/post-only are referenced by `55-terminal.md`
> as existing order attributes; they are additive fields the gateway order
> endpoint must define (widen `cid`/`oid`, add `ro`/`po`) or map onto the
> u64 form. Decide the wire types when the gateway terminates this channel.

**Cancel — target, not yet in `wire.rs`.** Cancel an order by client id or
server id (the server distinguishes by identifier type). The proven client
schema does not yet include a cancel message; `55-terminal.md` lists the
`c` cancel key as near-term. Target shape:

```proto
message WireCancel {
  uint64 cid = 1;   // cancel by client correlation id, OR
  uint64 oid = 2;   // cancel by server order id (whichever is set)
}
```

**Market-data subscribe / unsubscribe.** The subscribe control selects
channels via a bitmask (`1` = bbo, `2` = depth, `4` = trades). Target
protobuf shape:

```proto
message MdSubscribe   { uint32 symbol_id = 1; uint32 channels = 2; }
message MdUnsubscribe { uint32 symbol_id = 1; uint32 channels = 2; }
```

> **Current state.** The public feed's **inbound** control frame is still
> **JSON text** (`{S:[sym,ch]}` / `{X:[sym,ch]}` / `{H:[ts]}`), parsed by
> `rsx-marketdata/src/records.rs` `parse_client_frame`. Only the
> **outbound** feed is protobuf today. Moving the control frame to the
> protobuf messages above is the follow-up that makes the feed
> protobuf-in-both-directions.

### Server → client (order / event channel)

One `WireEvent` envelope wraps a `oneof` over the event kinds
(`rsx-tui/src/wire.rs`, tags 1–8). Exactly one variant is set per frame.

```proto
message BookLevel { int64 px = 1; int64 qty = 2; }
message Book      { repeated BookLevel bids = 1; repeated BookLevel asks = 2; }
message Trade     { int32 side = 1; int64 px = 2; int64 qty = 3; }
message OrderId   { uint64 oid = 1; }                          // Accepted, Done
message Fill      { uint64 oid = 1; int64 px = 2; int64 qty = 3; int32 side = 4; }
message Rejected  { string reason = 1; }
message Position  { string symbol = 1; int64 net_qty = 2; int64 entry_px = 3; int64 upnl = 4; }
message Latency   { uint64 cid = 1; optional uint64 net_ns = 2;
                    uint64 internal_ns = 3; uint64 engine_ns = 4; }

message WireEvent {
  oneof event {
    Book     book     = 1;   // single-market L2 projection (bids/asks)
    Trade    trade    = 2;   // a print on the session's symbol
    OrderId  accepted = 3;   // order accepted / resting (oid assigned)
    Fill     fill     = 4;   // one fill on the recipient's order
    OrderId  done     = 5;   // terminal: filled or cancelled
    Rejected rejected = 6;   // order failed (reason string)
    Position position = 7;   // pushed position update
    Latency  latency  = 8;   // server-stamped latency breakdown
  }
}
```

Semantics:

- **`Book` / `Trade`** are a **simplified single-market projection** the
  gateway multiplexes onto this one connection for the terminal — no
  `count`, `seq`, `ts`, or `symbol_id`. A full client instead consumes the
  richer [public market-data feed](#public-market-data-feed). The two are
  distinct schemas by design.
- **`accepted` / `done`** carry only an `oid`; the event kind encodes
  status (see the Order Status enum). There is no combined
  status/filled/remaining update frame.
- **`Fill`** is the recipient's own fill: their `oid`, the `px`/`qty`, and
  their `side`. It carries **no counterparty oid, timestamp, or fee**.
- **`Rejected.reason`** is a free string today, not a Failure Reason tag
  (see enum note).
- **`Position.symbol`** is a string **name** (e.g. `"PENGU-PERP"`), not the
  `uint32 symbol_id` used elsewhere. It is a server-push; whether the
  gateway emits it is gated on account data (`55-terminal.md` derives
  positions client-side until then).
- **`Latency`** drives the terminal's speed strip: `cid` pairs the sample
  to the submitted order; `net_ns` is optional (the gateway leaves it
  unset, the client fills the net leg from its measured RTT); `internal_ns`
  / `engine_ns` are the server-stamped legs.

**Not on this oneof — additive kinds the gateway endpoint owns:**

- **General error** (parse failure, protocol error) — the canonical `E`
  frame (`code`, `msg`). Today only per-order `Rejected(reason)` exists.
  Target: `message WireError { uint32 code = 1; string msg = 2; }` as a new
  oneof variant.
- **Heartbeat / liveness.** This channel has no heartbeat frame; liveness
  is the transport's job (WebSocket ping/pong, QUIC keepalive/PING). The
  public feed carries its own protobuf `Heartbeat` (below).
- **Liquidation event** (`Q` in the legacy frame; `13-liquidator.md`) — a
  server-push, fire-and-forget over casting/UDP, routed to the user. An
  additive `WireEvent` variant when wired to this channel.

### Public market-data feed

The canonical feed schema is `rsx-marketdata/marketdata.proto` (package
`rsx.marketdata`), server → subscriber, one `MdFrame` per frame, tags
STABLE. Same `[u32 BE len][protobuf body]` framing.

```proto
message Level     { int64 px = 1; int64 qty = 2; uint32 count = 3; }
message Bbo       { uint32 symbol_id = 1; int64 bid_px = 2; int64 bid_qty = 3;
                    uint32 bid_count = 4; int64 ask_px = 5; int64 ask_qty = 6;
                    uint32 ask_count = 7; uint64 timestamp_ns = 8; uint64 seq = 9; }
message Snapshot  { uint32 symbol_id = 1; repeated Level bids = 2;
                    repeated Level asks = 3; uint64 timestamp_ns = 4; uint64 seq = 5; }
message Delta     { uint32 symbol_id = 1; uint32 side = 2; int64 price = 3;
                    int64 qty = 4; uint32 count = 5; uint64 timestamp_ns = 6; uint64 seq = 7; }
message Trade     { uint32 symbol_id = 1; int64 price = 2; int64 qty = 3;
                    uint32 taker_side = 4; uint64 timestamp_ns = 5; uint64 seq = 6; }
message Heartbeat { uint64 timestamp_ms = 1; }

message MdFrame {
  oneof body {
    Bbo       bbo       = 1;
    Snapshot  snapshot  = 2;
    Delta     delta     = 3;
    Trade     trade     = 4;
    Heartbeat heartbeat = 5;
  }
}
```

- **`Snapshot`** carries full bids (descending px) and asks (ascending px);
  the server sends a snapshot on subscribe before any delta.
- **`Delta`** is one level: `side` 0=bid / 1=ask, `qty == 0` removes the
  level (proto3 zero-omission).
- **`seq`** is the matching-engine height (monotonic per symbol). Gap
  detection: if `seq` jumps by more than 1, re-subscribe for a fresh
  snapshot.
- **`Heartbeat`** is server → subscriber liveness. (The subscriber's own
  heartbeat is the inbound control frame — JSON text today, see
  [Client → server](#client--server).)

Field-name mapping to the WAL records (`16-marketdata.md`): `Bbo.*` ↔
`BboRecord.*`, `Delta.*` ↔ `L2Delta.*`, `seq` ↔ the record `seq` — the wire
field names now match the record field names directly (no single-letter
aliases).

### Post-MVP query messages

> **Post-MVP: not implemented in v1** (`11-gateway.md` §Post-MVP). Expressed
> here as protobuf request/response so they land as additive messages, not a
> second JSON dialect. Until they ship, clients restore state from the REST
> read endpoints (`GET /v1/orders`, `/v1/positions`, `/v1/account`,
> `26-rest.md`).

All values are tick / lot units (raw i64); the client applies tick/lot for
display. Request/response pairs (each response repeats a row message):

```proto
// Open orders
message OpenOrdersReq {}
message OpenOrder  { uint64 oid = 1; uint64 cid = 2; uint32 symbol = 3;
                     int32 side = 4; int64 price = 5; int64 qty = 6;
                     int64 filled = 7; int32 status = 8; int32 tif = 9;
                     bool reduce_only = 10; bool post_only = 11; uint64 ts_ns = 12; }
message OpenOrders { repeated OpenOrder orders = 1; }

// Positions (non-zero only)
message PositionsReq {}
message PositionRow  { uint32 symbol = 1; int32 side = 2; int64 qty = 3;
                       int64 entry_px = 4; int64 mark_px = 5;
                       int64 unrealized_pnl = 6; int64 liq_px = 7; }
message Positions    { repeated PositionRow positions = 1; }

// Account summary
message AccountReq {}
message Account    { int64 collateral = 1; int64 equity = 2; int64 unrealized_pnl = 3;
                     int64 initial_margin = 4; int64 maint_margin = 5; int64 available = 6; }

// Fill history (sym filter, 0 = all; before = cursor ts, 0 = latest)
message FillsReq  { uint32 symbol = 1; uint32 limit = 2; uint64 before = 3; }
message FillRow   { uint64 oid = 1; uint32 symbol = 2; int64 px = 3; int64 qty = 4;
                    int32 side = 5; int64 fee = 6; bool is_maker = 7; uint64 ts_ns = 8; }
message Fills     { repeated FillRow fills = 1; }   // descending by ts

// Funding history
message FundingReq { uint32 symbol = 1; uint32 limit = 2; uint64 before = 3; }
message FundingRow { uint32 symbol = 1; int64 amount = 2; int32 rate_bps = 3; uint64 ts_ns = 4; }
message Funding    { repeated FundingRow funding = 1; }   // descending by ts

// Symbol metadata (tick/lot as human strings for display)
message MetadataReq {}
message SymbolMeta  { uint32 symbol = 1; string tick = 2; string lot = 3; string name = 4; }
message Metadata    { repeated SymbolMeta symbols = 1; }
```

These are not on the order hot path — the gateway serves them from cached
state or Postgres.

## Transports

The messages above are identical on every transport. Each binding below
changes only how framed bytes move.

### WebSocket (live)

- **Binary frames.** Each WebSocket **binary** frame (opcode `0x2`; `0x82`
  with the FIN bit) carries one length-prefixed protobuf body. No
  permessage-deflate.
- **What is live today:** the public market-data feed is protobuf over
  WebSocket binary frames (`rsx-marketdata/src/wire.rs`, decoded by
  `rsx-playground/md_wire.py`). The inbound subscribe/heartbeat control is
  still JSON text (`records.rs`).
- **Auth.** Two accepted paths: (a) JWT in the WebSocket upgrade header
  (`Authorization: Bearer <JWT>`), rejected with HTTP 401 before the
  handshake completes; or (b) the in-band `WireHello` auth first-frame
  (the transport-neutral path the protobuf protocol standardizes on). JWT
  rules — HS256, `aud`/`iss`/`exp` claims — are in `11-gateway.md`.
- **Parse errors.** A malformed frame yields a server error message (target
  `WireError`), not a disconnect. The connection is closed only on fatal
  protocol violations (oversized frame > `MAX_FRAME`) or auth failure.

> **Note:** `11-gateway.md` §Limits still describes the pre-protobuf WS as
> "text frames only, no binary" with a 4 KB cap. That is stale against this
> binary protobuf binding — reconcile when the gateway order endpoint lands.

### QUIC / HTTP-3 (roadmap)

The **roadmap transport** for the order path (README §Roadmap step 4).
Full transport binding: **[58-http3.md](58-http3.md)**.

- A client-initiated **bidirectional QUIC stream** carries the same
  `[u32 BE len][protobuf body]` frame sequence — the first frame is
  `WireHello` (auth first-frame), then orders/events.
- **Unreliable datagrams (optional)** may carry droppable feed data (BBO /
  L2 delta / trade — best-effort per `GUARANTEES.md` §1.4, drops
  superseded); order/event frames stay on the reliable stream.
- **Browsers** cannot open raw QUIC streams; they reach this protocol via
  **WebTransport over HTTP/3**, carrying the same framed messages.
- **Current state:** `rsx-tui` speaks this client-side
  (`rsx-tui/src/wire.rs` + `quic.rs`, golden bytes in `wire_test.rs`,
  loopback server in `rsx-tui/tests/quic_test.rs`). No gateway terminates
  it yet — the gateway QUIC listener is the pending piece (58-http3.md).

## ACK / retry semantics

- There is **no order-accepted ACK.** The first response to a `WireOrder`
  is an `accepted` / `fill` / `rejected` event from the matching path.
- A client **retries with the same `cid`** if no order-update or fill
  arrives within its timeout. ME WAL dedup (`RECORD_ORDER_ACCEPTED`)
  collapses repeated attempts into **exactly-once ME acceptance**.
- The full retry / drop-safety / exactly-once contract per stream is
  **`GUARANTEES.md` §1.0 "Delivery Guarantees by Stream"** — the
  client→gateway order stream is drop-safe (client retries), fills are not
  (recovered via ME WAL). This spec does not restate that contract; it
  points there.

## Removed: JSON single-letter frames

The legacy `{N:[...]}` / `{U:[...]}` / `{BBO:[...]}` single-key JSON frame
format is **removed and superseded** by the protobuf messages above. The
only JSON that remains on any wire is the public feed's inbound
subscribe/heartbeat control (`records.rs`), pending its move to
`MdSubscribe` / `MdUnsubscribe`.

## Cross-references

| Concern | Spec |
|---------|------|
| Gateway: sessions, JWT, rate limits, backpressure | [11-gateway.md](11-gateway.md) |
| QUIC / HTTP-3 transport binding | [58-http3.md](58-http3.md) |
| Delivery guarantees (retry / exactly-once) | [../../GUARANTEES.md](../../GUARANTEES.md) §1.0 |
| Market-data feed (records, WAL mapping) | [16-marketdata.md](16-marketdata.md) |
| Terminal UX (the client that speaks this) | [55-terminal.md](55-terminal.md) |
| Terminal access / identity model | [54-tui-access.md](54-tui-access.md) |
| Error codes / reject reasons | [18-messages.md](18-messages.md) |
| REST read endpoints (state restore) | [26-rest.md](26-rest.md) |
| casting trust boundary (internal, unauthenticated) | [4-cast.md](4-cast.md) §10.4 |
