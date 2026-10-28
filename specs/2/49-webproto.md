---
status: partial
---

# Client Wire Protocol

The client wire protocol is what a client exchanges with the exchange's two
edge processes: the authenticated order channel on the gateway (new orders,
cancels, order updates, fills, errors), and the public market-data feed on
marketdata. The order channel is single-key JSON text frames over WebSocket
and is live. The market-data feed is protobuf binary frames over WebSocket and
is live. A protobuf order channel and QUIC / HTTP-3 bindings are roadmap; this
spec keeps their target schema under [Target: protobuf order channel
(proposed)](#target-protobuf-order-channel-proposed).

The code is the source of truth, not this prose. When they disagree, the code
wins and this spec is the bug. File anchors are at `29672aeb`.

| Surface | Source of truth |
|---|---|
| Order channel, gateway side | `rsx-messages/src/frames.rs` (`parse` :131, `serialize` :324), `rsx-gateway/src/handler.rs` (`serve_session` :104, `admit_order` :315), `rsx-gateway/src/route.rs` |
| Order channel, client side | `rsx-core/wire/order.go` |
| Public market-data feed | `rsx-marketdata/marketdata.proto` (schema), `rsx-marketdata/src/wire.rs` (encoder and `decode`, which `rsx-cli md` uses), `rsx-core/wire/md.go` (Go decoder), `rsx-glass/live/live.go` (live connector) |

## Table of Contents

- [Order channel](#order-channel)
- [Client → gateway frames](#client--gateway-frames)
- [Gateway → client frames](#gateway--client-frames)
- [Enums](#enums)
- [Error frame codes](#error-frame-codes)
- [ACK and retry](#ack-and-retry)
- [Public market-data feed](#public-market-data-feed)
- [Transports](#transports)
- [Target: protobuf order channel (proposed)](#target-protobuf-order-channel-proposed)
- [Cross-references](#cross-references)

---

## Order channel

One WebSocket per connection, text frames only. Each frame is one JSON object
with exactly one key, a letter naming the frame kind, whose value is a
positional array: `{"N":[...]}`. More than one key is `E[1002]`
(`rsx-messages/src/frames.rs:131`).

| Rule | Behaviour | Anchor |
|---|---|---|
| Upgrade | Any path carrying `Sec-WebSocket-Key` upgrades; any other HTTP request is REST (`26-rest.md`) | `rsx-gateway/src/ws.rs:60`, `rsx-gateway/src/handler.rs:53` |
| Auth | `Authorization: Bearer <JWT>` on the upgrade request. A missing or invalid token gets HTTP 401 and no handshake. There is no in-band auth frame. Claims and secret rules: `11-gateway.md` | `rsx-gateway/src/ws.rs:113` |
| TLS | The gateway listens on plain WS. TLS terminates at a reverse proxy in front of it (`9-deploy.md`) | `rsx-gateway/src/ws.rs` |
| Token reuse | A JWT opens any number of connections until its `exp`; the gateway keeps no per-token state | `rsx-gateway/src/jwt.rs:33` |
| Connections | At most 5 per user. The 6th is closed after the 101 with no error frame | `rsx-gateway/src/state.rs:143` |
| Client frames | Text, FIN set, masked, payload at most 4096 bytes. A fragmented, unmasked, oversized, binary or other non-text frame closes the connection. Ping is answered with pong; close ends the session | `rsx-gateway/src/ws.rs:150`, `rsx-gateway/src/handler.rs:104` |
| Compression | No `permessage-deflate` is negotiated | `rsx-gateway/src/ws.rs` |
| Parse errors | An invalid frame gets an `E` frame and the connection stays open | `rsx-gateway/src/handler.rs:167` |
| Liveness | The gateway sends `{"H":[ts_ms]}` on connect and every `RSX_GW_HEARTBEAT_INTERVAL_S` (5 s). A connection with no inbound frame of any kind for `RSX_GW_IDLE_TIMEOUT_S` (10 s) is closed | `rsx-gateway/src/main.rs:280-287` |
| Fixed point | Prices and quantities are `i64` in tick and lot units, never a float. Conversion to human units happens at the client's display boundary using the symbol's tick and lot size | `rsx-messages/src/frames.rs:171` |
| Identifiers | `cid` is the client's id for an order, 1 to 20 bytes, client-chosen. `oid` is the gateway-minted UUIDv7 as 32 lowercase hex characters. The client never sends an `oid` on `N` | `rsx-gateway/src/order_id.rs` |

## Client → gateway frames

| Frame | Fields | Rejected when |
|---|---|---|
| `N` new order | `[sym, side, px, qty, "cid", tif, ro, po]`: `sym` u32; `side` 0 buy / 1 sell; `px`, `qty` i64; `tif` 0 GTC / 1 IOC / 2 FOK; `ro` reduce-only and `po` post-only are optional 0 or 1 (default 0) | fewer than 6 elements, a wrong type, `side` > 1, `tif` > 2 or an empty or longer-than-20 `cid` is `E[1002]` |
| `C` cancel | `["key"]`: a 32-character key is an `oid`, a 1 to 20 character key is a `cid`; either names one of the sender's own pending orders | other key lengths are `E[1002]`; a malformed `oid` is `E[1005,"invalid order id"]`; no pending order of that user matches is `E[1005,"order not found"]` |
| `H` heartbeat | `[ts_ms]` | none; answered with an `H` carrying the gateway's time |

`U`, `F`, `E` and `Q` sent by a client are `E[1004,"unsupported"]`.

A new order passes these checks in order; the first failure answers with its
`E` and the order is not forwarded (`rsx-gateway/src/handler.rs:315`):

| # | Check | Answer |
|---|---|---|
| 1 | per-IP rate (`RSX_GW_RL_IP`, 100/s) | `E[1006,"rate limited"]` |
| 2 | per-user rate (`RSX_GW_RL_USER`, 10/s) | `E[1006,"rate limited"]` |
| 3 | per-instance rate (`RSX_GW_RL_INSTANCE`, 1000/s across all users and addresses) | `E[5,"overloaded"]` |
| 4 | `cid` already bound for this user inside the 1 hour window | the duplicate `U` (see [ACK and retry](#ack-and-retry)); nothing forwarded |
| 5 | symbol configured | `E[1007,"unknown symbol"]` |
| 6 | `px` > 0 and a multiple of the tick | `E[1008,"price not tick aligned"]` |
| 7 | `qty` > 0 and a multiple of the lot | `E[1009,"qty not lot aligned"]` |
| 8 | pending set not full (`RSX_GW_MAX_PENDING`) and circuit breaker closed or probing | `E[5,"overloaded"]` |
| 9 | the casting send to risk succeeds | `E[8,"network error"]` |

Margin, position limits, reduce-only and post-only semantics, and every other
policy check belong to risk and matching; the gateway answers for them with
the `U` those processes cause.

A cancel is forwarded to risk as a `CancelRequest` and gets no direct answer;
the outcome arrives as a `U` with status 2. A failed send is `E[8,"network
error"]`. A cancel spends no rate-limit token and does not consult the
circuit breaker (BUGS `GW-CANCEL-NO-RATELIMIT`).

## Gateway → client frames

| Frame | Array | Meaning |
|---|---|---|
| `U` order update | `["oid", status, filled, remaining, reason, gw_in_ns, risk_in_ns, me_in_ns, match_done_ns, gw_out_ns]` | one state change of one of the user's orders |
| `F` fill | `["taker_oid", "maker_oid", px, qty, ts_ns, fee, gw_in_ns, risk_in_ns, me_in_ns, match_done_ns, gw_out_ns]` | one fill on the user's order; the user is the taker, the maker or both |
| `E` error | `[code, "message"]` | a refusal or protocol error; see [Error frame codes](#error-frame-codes) |
| `H` heartbeat | `[ts_ms]` | liveness |
| `Q` liquidation | `[sym, status, round, side, qty, price, slip_bps]` | a liquidation notice for this user; `status` 0 to 4 (`13-liquidator.md`) |

`U` is produced by `rsx-gateway/src/route.rs` and the gateway's own sweep:

| `U` status | Source | `filled`, `remaining` | `reason` slot | Stamps |
|---|---|---|---|---|
| 0 FILLED | ORDER_DONE (`rsx-gateway/src/route.rs`, `route_order_done`) | from the record | 0 | set |
| 1 RESTING | ORDER_INSERTED (`rsx-gateway/src/route.rs`, `route_order_inserted`) | 0, the resting qty | 0 | 0 |
| 2 CANCELLED | ORDER_DONE with final status cancelled (`rsx-gateway/src/route.rs`, `route_order_done`) | from the record | the record's `CANCEL_REASON_*` | set |
| 3 FAILED | ORDER_FAILED (`rsx-gateway/src/route.rs:186`) | 0, 0 | the record's `FailureReason` | 0 |
| 3 FAILED | no first response inside `RSX_GW_ORDER_TIMEOUT_MS` (10 s) (`rsx-gateway/src/route.rs:203`) | 0, 0 | 10 TIMEOUT | 0 |
| 3 FAILED | repeated `cid` (`rsx-gateway/src/handler.rs:400`) | 0, 0 | 3 DUPLICATE_ORDER_ID | 0 |

**The `reason` slot (index 4) is one byte with two vocabularies, chosen by the
status in index 1.** Status 3 carries a `FailureReason`. Status 2 carries a
`CANCEL_REASON_*` code from the ORDER_DONE record: 0 for a user cancel, 2 for
an IOC remainder, 4 for a post-only order that would cross. Every other status
carries 0. Both vocabularies are in `18-messages.md`. A reader interprets the
slot only under status 2 or 3.

Other behaviour a client relies on:

- `U` carries no `cid` and no symbol, and `F` carries no side and no symbol.
  A client pairs the first non-failed `U` of an order with its pending
  submission itself (`rsx-core/wire/order.go` `Folder`).
- The stamps are Unix nanoseconds from synchronized clocks; 0 means unset.
  `gw_in_ns` and `gw_out_ns` are the gateway's; the middle three are carried
  from risk and the matching engine (`59-latency-observability.md`).
- A fill is one `F` per fill. When taker and maker are one user, that user
  gets one frame.
- `fee` is the receiving user's own signed fee, `floor(qty * px * bps /
  10_000)` with the taker or maker rate of the fill; a negative fee is a
  rebate. The taker's frame carries the taker fee and the maker's the maker
  fee; the one frame of a self-trade carries their sum.
- Every order ends in exactly one status-2 or status-0 `U` (ORDER_DONE) or one
  status-3 `U` (ORDER_FAILED), a user cancel included.
- Fills precede the order's ORDER_DONE on the channel (`6-consistency.md`).
- `U`, `F` and `Q` go to every open connection of the user. A connection that
  cannot keep up (more than `RSX_GW_MAX_OUTBOUND` queued frames) is closed;
  the client reconnects. Updates are never dropped or coalesced.
- There is no order-accepted ACK. The first response to an `N` is a `U`, an
  `F` or an `E`.

## Enums

Side, time in force and the `U` status are the integers in the frame tables
above. The two reason vocabularies are owned by `18-messages.md`:

| Vocabulary | Values | Where it appears |
|---|---|---|
| `FailureReason` | 0 to 14, `ORDER_TOO_SMALL` = 14 | `U` status 3, and the reject counters on `/metrics` |
| `CANCEL_REASON_*` | 0 to 5 | `U` status 2 |

## Error frame codes

`E[code,"message"]` is a refusal or a protocol error. One code names one
condition class and is never reused for another; the table is the complete
set the gateway sends (`rsx-gateway/src/handler.rs:44-46, 167-296`).

| Code | Message | Condition |
|---|---|---|
| 5 | `overloaded` | the per-instance rate limit, a full pending set, or an open circuit breaker |
| 8 | `network error` | the casting send of an `N` or `C` to risk failed |
| 1001 | `invalid utf8` | the frame payload is not UTF-8 |
| 1002 | the parse error text (`invalid json`, `multiple keys`, `unknown type: K`, `missing field: ...`, `invalid value: ...`) | the frame does not parse, including a `cid` outside 1 to 20 bytes |
| 1004 | `unsupported` | a frame kind the gateway does not accept from clients |
| 1005 | `invalid order id`, `order not found` | a cancel key that does not resolve to a pending order of the sender |
| 1006 | `rate limited` | the per-user or per-IP rate limit; no other condition |
| 1007 | `unknown symbol` | the symbol is not configured |
| 1008 | `price not tick aligned` | `px` <= 0 or off the tick |
| 1009 | `qty not lot aligned` | `qty` <= 0 or off the lot |

Codes 5 and 8 share their numbers with the `OVERLOADED` and `NETWORK_ERROR`
values of `FailureReason`. A refused `N` is counted on `/metrics` under the
`FailureReason` of the same condition.

## ACK and retry

- There is no order-accepted ACK. A client that gets no `U`, `F` or `E` for an
  order within its timeout resends it with the same `cid`
  (`GUARANTEES.md` §1.0).
- The gateway binds `(user_id, cid)` to the order id it minted when it forwards
  an order, for 1 hour, in memory (`rsx-gateway/src/client_ids.rs:14`). A later
  `N` with that `cid` inside the window is not forwarded. The sender gets `U`
  with status 3 and reason 3 (`DUPLICATE_ORDER_ID`) carrying the first order's
  `oid`, the answer the matching engine gives a repeated order id. The first
  order's own `U` and `F` frames are unaffected.
- The binding is never released early: it outlives the order's completion or
  failure for the full hour, and a failed order's `cid` is spent. A gateway
  restart empties the map, so a resend after a restart is a new order.
- The matching engine's dedup key is `(user_id, order_id)` over the same
  1 hour window, rebuilt from its WAL on restart. It rejects a repeated order
  id with ORDER_FAILED(`DUPLICATE_ORDER_ID`); a resent `cid` never reaches it,
  because the gateway answers first.
- The gateway forwards a `cid` once. A resend returns the first order's id; it
  does not submit again an order that was lost before it reached risk.
- TIMEOUT (status 3, reason 10) means no first response arrived in time. The
  order's fate is unknown: it may still reach the book. A resting order is
  never swept.
- The delivery contract per stream is `GUARANTEES.md` §1.0. The target design
  that answers a resend with a frame that cannot read as a failure, keeps the
  binding across a restart and gives clients their cid rules is
  [61-cid-idempotency.md](61-cid-idempotency.md) (draft).

## Public market-data feed

The canonical feed schema is `rsx-marketdata/marketdata.proto` (package
`rsx.marketdata`), server → subscriber, one `MdFrame` per binary WebSocket
frame, tags stable. Each frame is `[u32 big-endian length][protobuf body]`
(`rsx-marketdata/src/wire.rs`); a body over `MAX_FRAME` (1 MiB) is rejected on
read and write, so a corrupt length never triggers a large allocation. The
schema is hand-derived `prost::Message` and `prost::Oneof` structs, with no
`protoc` on the build path; marketdata's `wire_test.rs` pins the encoded bytes.

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

- **proto3 zero-omission.** A zero scalar is omitted on the wire; a decoder
  treats an absent scalar as 0. The feed decoders rely on it (`Delta.side` 0
  is a bid, `Delta.qty` 0 removes a level, `Bbo.bid_px` 0 is an empty side).
- **Strict enum decode.** `side` is decoded through explicit match arms that
  error on an unknown value rather than defaulting, so a garbled frame fails
  the read instead of trading the wrong direction.
- **Tags are stable and append-only.** Appending a field is compatible;
  renumbering is breaking; a new frame kind is a new oneof variant.
- **`Snapshot`** carries full bids (descending px) and asks (ascending px); the
  server sends one on subscribe before any delta.
- **`Delta`** is one level: `side` 0 = bid, 1 = ask; `qty` 0 removes it.
- **`seq`** is the matching-engine height, monotonic per symbol. If it jumps
  by more than 1, the subscriber re-subscribes for a fresh snapshot.
- **`Heartbeat`** is server → subscriber liveness.
- **Inbound control is JSON text**, not protobuf: `{"S":[sym,ch]}` subscribe,
  `{"X":[sym,ch]}` unsubscribe, `{"H":[ts]}` heartbeat, parsed by
  `rsx-marketdata/src/records.rs` `parse_client_frame`. `ch` is a bitmask
  (1 bbo, 2 depth, 4 trades).
- Field names map to the WAL records (`16-marketdata.md`): `Bbo.*` ↔
  `BboRecord.*`, `Delta.*` ↔ `L2Delta.*`, `seq` ↔ the record `seq`.

## Transports

- **WebSocket (live).** The order channel uses text frames; the feed uses
  binary frames. Auth on the order channel is the JWT upgrade header above.
  The feed's auth and limits are `16-marketdata.md`.
- **QUIC / HTTP-3 (roadmap).** The roadmap transport for the order path;
  binding in [58-http3.md](58-http3.md). No client speaks it and no gateway
  terminates it.

## Target: protobuf order channel (proposed)

Nothing in this section is implemented. It records the schema the roadmap
transports would carry; the live order channel is the JSON above, and a client
built from this section does not talk to the running gateway.

**One framing on every transport:** `[u32 big-endian length][protobuf body]`,
the feed's framing. WebSocket and QUIC already delimit messages; the prefix
is kept so one parser serves any transport. Integer tags are stable and
append-only.

**Client → server.** `WireHello` carries the JWT in-band as the first frame
(the transport-neutral path QUIC needs, since a browser WebTransport stream has
no upgrade header).

```proto
message WireHello  { string jwt = 1; uint32 user = 2; }
message WireOrder  { uint64 cid = 1; uint32 symbol = 2; int32 side = 3;
                     int64 price = 4; int64 qty = 5; int32 tif = 6; }
message WireCancel { uint64 cid = 1; uint64 oid = 2; }   // whichever is set
message MdSubscribe   { uint32 symbol_id = 1; uint32 channels = 2; }
message MdUnsubscribe { uint32 symbol_id = 1; uint32 channels = 2; }
```

`WireOrder.cid` is `uint64` here while the live channel's `cid` is a string of
1 to 20 bytes, and the live `N` carries `ro` and `po`, which `WireOrder` lacks
(tags 7 and 8 are free). The wire types are decided when the gateway
terminates this channel.

**Server → client.** One `WireEvent` envelope wraps a oneof over the kinds
below; exactly one is set per frame.

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
    Book book = 1; Trade trade = 2; OrderId accepted = 3; Fill fill = 4;
    OrderId done = 5; Rejected rejected = 6; Position position = 7;
    Latency latency = 8;
  }
}
```

- The event kind encodes status (`accepted`, `done`, `rejected`); there is no
  combined status, filled and remaining frame as the live `U`.
- `Rejected.reason` is a string; a tag from the `FailureReason` table in
  `18-messages.md` would replace it.
- `Fill` carries no counterparty, timestamp or fee.
- A general error (`WireError { uint32 code = 1; string msg = 2; }`) and a
  liquidation event are additive oneof variants. Liveness is the transport's
  job (WebSocket ping, QUIC keepalive).
- `Book` and `Trade` here are a single-market projection for a terminal; a
  full client consumes the public feed above.

**Post-MVP queries.** The queries that restore state after a reconnect (open
orders, positions, account, fills, funding, symbol metadata) are not
implemented (`11-gateway.md` §Post-MVP). They are specified as protobuf
request and response pairs so they land as additive messages, not a second
JSON dialect. Until they exist, state comes from REST reads (`26-rest.md`).
All values are raw tick and lot units.

```proto
message OpenOrdersReq {}
message OpenOrder  { uint64 oid = 1; uint64 cid = 2; uint32 symbol = 3;
                     int32 side = 4; int64 price = 5; int64 qty = 6;
                     int64 filled = 7; int32 status = 8; int32 tif = 9;
                     bool reduce_only = 10; bool post_only = 11; uint64 ts_ns = 12; }
message OpenOrders { repeated OpenOrder orders = 1; }

message PositionsReq {}
message PositionRow  { uint32 symbol = 1; int32 side = 2; int64 qty = 3;
                       int64 entry_px = 4; int64 mark_px = 5;
                       int64 unrealized_pnl = 6; int64 liq_px = 7; }
message Positions    { repeated PositionRow positions = 1; }

message AccountReq {}
message Account    { int64 collateral = 1; int64 equity = 2; int64 unrealized_pnl = 3;
                     int64 initial_margin = 4; int64 maint_margin = 5; int64 available = 6; }

message FillsReq  { uint32 symbol = 1; uint32 limit = 2; uint64 before = 3; }
message FillRow   { uint64 oid = 1; uint32 symbol = 2; int64 px = 3; int64 qty = 4;
                    int32 side = 5; int64 fee = 6; bool is_maker = 7; uint64 ts_ns = 8; }
message Fills     { repeated FillRow fills = 1; }   // descending by ts

message FundingReq { uint32 symbol = 1; uint32 limit = 2; uint64 before = 3; }
message FundingRow { uint32 symbol = 1; int64 amount = 2; int32 rate_bps = 3; uint64 ts_ns = 4; }
message Funding    { repeated FundingRow funding = 1; }   // descending by ts

message MetadataReq {}
message SymbolMeta  { uint32 symbol = 1; string tick = 2; string lot = 3; string name = 4; }
message Metadata    { repeated SymbolMeta symbols = 1; }
```

**QUIC.** A client-initiated bidirectional stream carries the same frame
sequence, `WireHello` first. Unreliable datagrams may carry droppable feed
data (BBO, L2 delta, trade; best-effort per `GUARANTEES.md` §1.4); order and
event frames stay on the reliable stream. Browsers reach it by WebTransport
over HTTP/3.

## Cross-references

| Concern | Spec |
|---------|------|
| Gateway: sessions, JWT, rate limits, backpressure | [11-gateway.md](11-gateway.md) |
| Reason codes (`FailureReason`, `CANCEL_REASON_*`), record layouts | [18-messages.md](18-messages.md) |
| Timeout, pending set, circuit breaker | [29-rpc.md](29-rpc.md) |
| cid idempotency target design (draft) | [61-cid-idempotency.md](61-cid-idempotency.md) |
| QUIC / HTTP-3 transport binding | [58-http3.md](58-http3.md) |
| Delivery guarantees | [../../GUARANTEES.md](../../GUARANTEES.md) §1.0 |
| Market-data feed (records, WAL mapping) | [16-marketdata.md](16-marketdata.md) |
| Terminal UX (the client that speaks this) | [55-terminal.md](55-terminal.md) |
| REST reads (state restore) | [26-rest.md](26-rest.md) |
| Per-hop latency stamps | [59-latency-observability.md](59-latency-observability.md) |
| casting trust boundary (internal, unauthenticated) | [4-cast.md](4-cast.md) §10.4 |
