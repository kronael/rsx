# rsx-gateway Architecture

How the gateway process is built: where each piece lives and how an order and
its response move through it. Env names, defaults and the error-code table are
in [README.md](README.md).

The gateway accepts client WebSocket connections, validates orders, forwards
them to Risk over casting/UDP and pushes Risk's responses back to the owning
user's connections. The same port serves REST.

Specs: `specs/2/11-gateway.md`, `specs/2/49-webproto.md`.

## Runtime Model

A reactor thread and a busy-spin receiver thread, joined by one SPSC ring.

```
   clients (WS, JSON text frames)
        ^                   |
   U F E Q H                | N C H
        |                   v
 +------------------ monoio reactor (main thread) ------------------+
 | accept_loop -> handle_connection, one task per connection         |
 | housekeeping loop: pop ring -> route_* -> connection queues;      |
 |   sender tick, pending sweep, cid expire, heartbeat, idle reap    |
 +-----------+---------------------------------------^---------------+
             | OrderRequest, CancelRequest           | CastEvent via
             | casting/UDP                           | SPSC ring (8192)
             v                                       | and UDP doorbell
           Risk ---- fills, updates, casting/UDP --> gw-cast-recv thread
                                                     (CastReceiver spin)
        state: Rc<RefCell<GatewayState>>, owned by the reactor, no locks
```

- **Reactor.** One monoio (io_uring) thread owns all gateway state in
  `Rc<RefCell<GatewayState>>`: no locks, no cross-thread sharing. Each
  connection is a monoio task spawned from the accept loop.
- **`gw-cast-recv`.** A plain thread busy-spins `CastReceiver::try_recv_with`,
  decodes each record into a `CastEvent`, pushes it on the ring and sends one
  byte to a loopback UDP doorbell per drained batch. A full ring stalls this
  thread instead of dropping a response. A sticky FAULTED or RECONNECT state
  is repaired here by `handle_replay`: replay from Risk through
  `RSX_RISK_REPLICATION_ADDR`, or skip the gap and warn.
- **Housekeeping loop.** The reactor main task waits on the doorbell's
  readiness or a 1 ms timer, never polls. Each pass drains the ring into the
  `route_*` functions, ticks the casting sender, runs the sweeps below and
  publishes the gauges.
- **`gw-symbols`.** The symbol table (tick, lot, decimals, status, minimum
  notional) is read from risk's Postgres at boot, before the reactor exists,
  and again by this thread on a poll (`RSX_GW_SYMBOL_POLL_S`) and when the
  housekeeping loop sees a `CONFIG_APPLIED` newer than the table. A changed
  table comes back over an SPSC ring; the loop swaps it in. The reactor never
  waits on Postgres. The thread owns a current-thread tokio runtime and one
  `tokio_postgres` client, reopened after a failed read.
- Other threads: `rsx-log-drain` and the health server. `drain_replay` builds
  a short-lived tokio runtime on `gw-cast-recv` for one replay; tokio is
  otherwise only in `gw-symbols`.

Shutdown ticks the sender once and joins `gw-cast-recv`; it sends no close
frames. The gateway is not a tile: its loop multiplexes many sockets, where a
tile spins on one ring. See
[`../docs/concepts/02-tiles-and-pinning.md`](../docs/concepts/02-tiles-and-pinning.md)
and [notes/io-model.md](notes/io-model.md).

## Core Pinning

Each of the two threads takes its own core, an index into the process's allowed cpus
(`rsx_types::cpu::setup_hot_thread`):

| Thread | Env | Unset |
|--------|-----|-------|
| monoio reactor (main) | `RSX_GW_CORE_ID` | unpinned |
| `gw-cast-recv` | `RSX_GW_CAST_CORE_ID` | unpinned, warning at startup |

`gw-cast-recv` pins itself when it starts. The reactor pins after every
sidecar thread (log drainer, health server, `gw-cast-recv`, `gw-symbols`) is spawned,
because a thread inherits its creator's affinity. Naming one core for both
threads logs a warning. `tests/gateway_process_test.rs` reads
`/proc/<pid>/task/*/status` to check both.

## Module Layout

| File | Purpose |
|------|---------|
| `main.rs` | Binary: env, health server, casting sockets, reactor, housekeeping loop |
| `lib.rs` | Module list and the `drain_replay` re-export |
| `config.rs` | `GatewayConfig` from env, JWT secret check, core-id parsing |
| `symbols.rs` | Symbol table from risk's Postgres: boot load, `gw-symbols` sidecar thread (poll and reload request), hand-over ring |
| `state.rs` | `GatewayState`, `ConnectionState`, bounded IP limiter, heartbeat queueing, idle reap |
| `handler.rs` | Per-connection task: HTTP read, REST or WS branch, handshake, frame loop, `admit_order`, casting forward |
| `ws.rs` | HTTP request read, WS handshake, JWT extraction, frame read and write, 4 KB frame cap |
| `rest.rs` | `GET /health` and `GET /v1/symbols` on the same listener |
| `jwt.rs` | HS256 validation into `rsx_messages::Claims`, which `rsx_messages::mint_jwt` signs |
| `rate_limit.rs` | Microsecond-resolution token bucket |
| `circuit.rs` | `CircuitBreaker` (Closed, Open, HalfOpen) |
| `pending.rs` | `PendingOrders`: one queue per user, `Submitted` and `Resting` stages, stale sweep |
| `client_ids.rs` | `ClientOrderIds`: `(user_id, cid)` to order id for 1 hour, expiry queue |
| `order_id.rs` | UUIDv7 generation and the hex codec |
| `convert.rs` | Tick and lot alignment checks |
| `route.rs` | Risk record to client frame: Fill, OrderInserted, OrderDone, OrderFailed, Liquidation; the timeout sweep |
| `cast_recv.rs` | `gw-cast-recv` thread, `CastEvent`, ring and doorbell, replay handling |
| `replay.rs` | `drain_replay`: fetch a gap from Risk's replication server |
| `egress.rs` | `EgressWaker`: per-connection wake and close signal |
| `metrics.rs` | `GatewayMetrics`: single-writer counters and the `/metrics` snapshot |

The client JSON frames (`WsFrame`, `CancelKey`, `ParseError`, `parse`, `serialize`) are owned by `rsx-messages` (`src/frames.rs`); the handler and `route.rs` import them from there.

## REST and WS on One Port

`handle_connection` reads the initial HTTP request head, at most 8 KiB (`431`
beyond) and within `WS_UPGRADE_DEADLINE` (5 s, `rsx-types`). `ws::is_ws_upgrade`
decides: a request carrying `Sec-WebSocket-Key` takes the WS path, any other
request takes `rest::handle_rest`, which writes one response and closes. A GET
of `/health` answers `{"status","version","uptime_sec","ts"}`. A GET of
`/v1/symbols` answers `{"symbols":[{"id","tick_size","lot_size",
"price_decimals","qty_decimals","status","min_notional"}]}` from the symbol
table, in symbol id order. Another GET
path is 404 and any other method is 400, both with an `{"E":[code,"message"]}`
body. Neither path reads a JWT.

## JWT Authentication

`jwt::validate_jwt` enforces the HS256 signature, `exp`, `nbf` when present,
`aud == "rsx-gateway"` and `iss == "rsx-auth"`. The `user_id` claim is used,
else `sub` parsed as a number. Startup exits 2 when `RSX_GW_JWT_SECRET` is
empty or shorter than `JWT_SECRET_MIN_LEN = 32` bytes. A failed check answers
HTTP 401 before the 101.

A token is reusable until `exp`: the gateway keeps no per-token state, so one
token opens any number of connections up to the per-user limit, and a `jti`
claim is ignored. `tests/handler_session_test.rs::same_token_opens_two_connections`
covers it.

## Order Admission

`handler::admit_order` runs every `N` frame through these checks in order. The
first failure answers and ends the frame.

| # | Check | Answer |
|---|-------|--------|
| 1 | per-IP bucket, `peer.ip()` from `accept()` | `E[1006]` |
| 2 | per-user bucket | `E[1006]` |
| 3 | per-instance bucket | `E[5]` |
| 4 | `(user_id, cid)` already bound | duplicate `U`, nothing forwarded |
| 5 | symbol id configured | `E[1007]` |
| 6 | price tick aligned | `E[1008]` |
| 7 | qty lot aligned | `E[1009]` |
| 8 | in-flight set not full and `circuit.allow()` | `E[5]` |

Check 8 is last so a half-open circuit spends its one probe on an order that
is sent. A repeat at check 4 takes no pending slot and no probe. A forward
failure after check 8 answers `E[8]` and records a circuit failure.

The per-IP map holds at most `IP_LIMITER_MAX = 10_000` addresses with a
parallel `VecDeque<IpAddr>` for insertion order; at the cap the oldest address
is evicted. This bounds memory against a rotating-address sender
(`tests/state_test.rs::ip_limiter_map_is_bounded`). `RateLimiter` counts in
capacity times 1,000,000 internal units for sub-second refill.

## Circuit Breaker

`circuit.rs` is fail-closed. `Closed` becomes `Open` after `threshold`
consecutive failures. After `cooldown` the next `allow()` moves it to
`HalfOpen`, which admits one probe. A success closes it; a failure reopens it.
The only recorded failure is a failed casting send of an `N`: the breaker does
not see a slow or silent Risk. Defaults: threshold 10, cooldown 30 s.

## Connection Lifecycle

```
accept -> read HTTP -> REST | WS upgrade -> JWT check -> 101
       -> add_connection (5 per user) -> greeting H -> frame loop
frame loop: drain outbound queue -> wait (socket readable | egress wake)
            -> read frame -> parse -> act
```

- **Limit.** At most 5 connections per `user_id`. The check runs after the
  101, so the 6th connection completes the upgrade and is then dropped with
  no error frame.
- **Frames.** `ws_read_frame_buf` rejects a payload over 4096 bytes, a
  fragmented frame and an unmasked frame with an error that ends the session.
  Binary and other non-text data opcodes end it too. Ping is answered with
  pong. A parse failure answers `E[1001]` or `E[1002]` and the session
  continues.
- **Heartbeat and reap.** The housekeeping loop owns both. It queues
  `{"H":[ts_ms]}` on every connection each `RSX_GW_HEARTBEAT_INTERVAL_S` and
  calls `reap_idle_connections` each pass; a connection with no inbound frame
  for `RSX_GW_IDLE_TIMEOUT_S` is removed. The handler queues only the greeting
  and echoes a client `H`.
- **Outbound queue.** Each connection holds a `VecDeque<Arc<str>>` capped at
  `RSX_GW_MAX_OUTBOUND`. A producer that finds it full removes the connection.
  Updates are never dropped or coalesced; the client reconnects.
- **Close.** `remove_connection` closes the connection's `EgressWaker`. The
  handler races its session against `egress.closed()`, so a reap or overflow
  ends the session wherever it is parked and the socket is dropped.

## Message Flow

```
Client (WS JSON)
   |
   v
handler.rs serve_session
   1. wait readable | egress wake
   2. ws_read_frame_buf (4 KB cap), UTF-8, rsx_messages::parse
   3. N: admit_order -> UUIDv7 oid -> OrderRequest
      -> CastSender::send_raw(ORDER_REQUEST)   (stack struct, no heap)
      -> pending.push, client_ids.insert, circuit.record_success
   3. C: pending lookup by oid or cid for this user
      -> CancelRequest via send_raw
   |
   v  casting/UDP to Risk, Risk -> Matching -> Risk, casting/UDP back
   |
gw-cast-recv: decode -> SPSC ring -> doorbell
   |
   v
main.rs loop: pop CastEvent -> route.rs
   route_fill / _order_inserted / _order_done /
   _order_failed / _liquidation: serialize WsFrame once (Arc<str>),
   push_to_user -> queue on each of the user's connections, wake handlers
   |
   v
handler.rs: drain_outbound -> ws_write_text
```

The forward path writes a stack struct and allocates nothing. The response
path allocates one `Arc<str>` per frame, shared by every connection of the
user. The gateway has no order-accepted ACK: the first response to an `N` is a
`U`, an `F` or an `E`.

## Pending Orders

`PendingOrders` holds one queue per user. `RSX_GW_MAX_PENDING` bounds the
`Submitted` entries across all users, so a user's resting orders never take
another user's slot; the `Resting` entries stay tracked for cancel and are
bounded only by what Risk lets an account rest (the minimum order notional).
A lookup names the user, so one user's cancel never sees
another user's order. An entry is `Submitted` until Risk answers and `Resting`
after ORDER_INSERTED, which frees its slot under the cap. A terminal record (`OrderDone`, `OrderFailed`) removes the entry and the
update goes to the entry's user; every order ends in exactly one `U`. The
`U` reason slot of a cancelled `OrderDone` is its `cancel_reason`.

A cancel needs a pending entry: `CancelRequest` carries the symbol id, which
only the entry holds. So an order that is not pending cannot be cancelled.

The housekeeping loop sweeps every 100 ms. A `Submitted` entry older than
`RSX_GW_ORDER_TIMEOUT_MS` is removed, counted as a `TIMEOUT` reject and
answered with `U` status 3, reason 10. The gateway re-sends nothing. A
`Resting` entry is never swept.

## Client Order Ids

`ClientOrderIds` binds `(user_id, cid)` to the order id the gateway minted,
once the order has been forwarded. A frame with a bound cid inside `WINDOW_NS`
(1 hour, the ME's dedup window) is never forwarded: the sender gets `U` status
3, reason 3 (`DUPLICATE_ORDER_ID`), naming the first order's id. A failed
forward binds nothing. A binding is not released when its order completes.

The loop calls `expire(now)` every 100 ms. The map is a `FxHashMap` plus a
`VecDeque` of `(inserted_ns, key)` in insertion order, so a sweep pops only
what expired. Memory is 72 bytes per live binding (40 map slot, 32 queue
entry) before hash-table slack, and at most one binding per order admitted in
the last hour. The instance limit (default 1000/s) bounds that at 3.6 million
bindings, about 260 MB before slack. A restart empties the map; a retry
across a restart is a new order, because the ME dedups on `(user_id,
order_id)` and the retry mints a new order id.

## Metrics

`RSX_GW_HEALTH_ADDR` serves `/health`, `/ready` and `/metrics`
(`metrics.rs`). Counters are relaxed `u64` gauges with one writer, the
reactor thread; the health thread only reads them.

| Counter | Counts |
|---------|--------|
| `orders_processed` | orders forwarded to Risk |
| `rejects_<reason>` | orders refused or failed, one counter per `FailureReason`: gateway rejects, Risk and ME `ORDER_FAILED`, the timeout, a repeated cid; `rejects_other` for a code outside the enum |
| `rejects_total` | sum of the `rejects_*` counters |
| `circuit_state` | 0 closed, 1 open, 2 half-open |
| `connections` | open WS connections |
| `resting_orders` | resting entries tracked for cancel; not part of the `pending` queue |

The `pending` gauge carries the in-flight (`Submitted`) count and its capacity;
`/ready` is 503 while it is 90% full. `circuit_state` is published by the housekeeping loop, so after a
cooldown it reads open until the next order moves the breaker to half-open.

## Statelessness and Scaling

The gateway holds no durable state. A crash drops the sessions, the pending
set and the cid map. Risk and its WAL hold authoritative state, and fills are
persisted by Risk and the Recorder.

Risk sends all responses to one `RSX_GW_CAST_ADDR`, so one gateway serves one
Risk. A client set larger than one gateway is not yet spread over several.

## Why monoio

The gateway sits on the GW to ME to GW critical path and multiplexes many
client sockets. io_uring batches submissions and completions in shared rings,
so it pays a fraction of epoll's per-event syscall cost, and a single reactor
keeps state in `Rc<RefCell<_>>` with no locks. The decision and its limits are
in [notes/io-model.md](notes/io-model.md).
