# rsx-marketdata Architecture

Market data process. Aggregates events from N matching engines,
maintains a shadow orderbook per symbol, and publishes L2 depth,
BBO, and trades to subscribed WebSocket clients.

Spec: `specs/2/16-marketdata.md`. WS client protocol shares the
envelope conventions of `specs/2/49-webproto.md`.

## Runtime Model

Single monoio (io_uring) reactor on one thread. All state lives
in `Rc<RefCell<MarketDataState>>`; no locks, no cross-thread
sharing. The main loop iterates over one CMP receiver per
matching engine, dispatches records to the shadow book,
broadcasts deltas/BBO/trades, and runs heartbeat + stale-book
eviction sweeps.

## Module Layout

| File | Purpose |
|------|---------|
| `main.rs` | Binary: monoio runtime, replay bootstrap, per-ME CMP loop, dispatch, heartbeat, book eviction |
| `lib.rs` | Re-exports |
| `config.rs` | `MarketDataConfig` from env; `me_cmp_addrs_from_env` parses multi-ME list |
| `state.rs` | `MarketDataState`, connection registry, subscription manager wrapper, per-symbol seq tracking, snapshot dispatch |
| `shadow.rs` | `ShadowBook` -- wraps `rsx_book::Orderbook` with order-id index for `apply_*_by_order_id` |
| `subscription.rs` | `SubscriptionManager` -- per-client channels (BBO/depth/trades) + depth pref |
| `protocol.rs` | JSON envelopes (`BBO`/`B`/`D`/`T`/`H`) + client frame parser (`S`/`X`/`H`) |
| `handler.rs` | Per-connection: handshake, subscribe/unsubscribe/heartbeat, outbound drain |
| `ws.rs` | WebSocket handshake (no auth) + frame I/O |
| `types.rs` | `BboUpdate`, `L2Snapshot`, `L2Delta`, `L2Level`, `TradeEvent` |
| `replay.rs` | DXS cold-path bootstrap (TCP) before going live |

## Derived-From-Events Approach

Marketdata does not own authoritative book state. It rebuilds
each book from the ME event stream:

- `OrderInserted` -> `apply_insert_by_id`
- `OrderCancelled` -> `apply_cancel_by_order_id`
- `Fill` -> `apply_fill_by_order_id` (+ trade event)
- `OrderDone` -> ignored (terminal status not needed for depth)

`ShadowBook` reuses `rsx_book::Orderbook` and keeps an order-id
-> slab-handle map so cancel/fill records (which carry only the
order id) can locate the resting level.

## Multi-ME Aggregation

The exchange runs one matching engine per symbol on its own
CMP port. Marketdata opens **one `CmpReceiver` per ME**:

- ME addresses come from `RSX_ME_CMP_ADDRS` (comma-separated)
  or `RSX_ME_CMP_ADDR` (single)
- For each ME at port P, marketdata binds its local recv side
  at port **P + 400** (e.g. ME 9100 -> MD 9500)
- The main loop iterates over the receiver vector each tick,
  draining all available records before sleeping

Per-symbol sequence tracking lets one ME's gaps be detected
independently of the others.

## Sequence Gap Detection & Recovery

For every record carrying a `seq`, `state.check_seq(symbol_id,
seq)` compares against the per-symbol `expected_seq`:

- First record initializes the expectation
- `seq == expected` -> advance
- `seq > expected` -> gap; bump `gap_count`, log warning,
  call `resend_snapshot` which broadcasts a fresh L2 snapshot
  to every depth subscriber on that symbol
- `seq < expected` -> duplicate, ignored

`resend_snapshot` also fires when a per-client outbound queue
overflows (`max_outbound`) -- drop deltas, send a snapshot,
let the client resync.

## Cold-Path Bootstrap (DXS/TCP)

When `RSX_MD_REPLAY_ADDR` is set, `replay::run_replay_bootstrap_blocking`
connects to the DXS endpoint, drains `OrderInserted` /
`OrderCancelled` / `Fill` records up to `CaughtUp`, and replays
them into the shadow books before the live CMP loop starts.
Persisted tip at `RSX_MD_TIP_FILE` makes restart idempotent.

The blocking wrapper uses a single-threaded tokio runtime
inside `run_replay_bootstrap_blocking`; the live path remains
pure monoio.

## Publishing

| Feed | Trigger | Wire envelope |
|------|---------|---------------|
| Snapshot | On subscribe (depth channel) + on seq gap | `{"B":[sid,bids,asks,ts,seq]}` |
| L2 delta | Any level change | `{"D":[sid,side,px,qty,count,ts,seq]}` |
| BBO | Best level change (deduped via `last_bbo`) | `{"BBO":[...]}` |
| Trades | Each fill | `{"T":[sid,px,qty,taker_side,ts,seq]}` |
| Heartbeat | Every `RSX_MD_HEARTBEAT_INTERVAL_S` | `{"H":[ts_ms]}` |

Per-client subscriptions are checked via `has_bbo` / `has_depth`
/ `has_trades` before each push.

## Client Protocol

| Op | Frame |
|----|-------|
| Subscribe | `{"S":[sym,channels]}` -- channels bitmask (1=BBO, 2=depth, 4=trades) |
| Unsubscribe | `{"X":[sym,channels]}` -- `sym==0` -> all |
| Heartbeat | `{"H":[timestamp_ms]}` -- echoed |

Subscribing to depth (`channels & 2`) ensures the book exists
and immediately sends an L2 snapshot at `RSX_MD_SNAPSHOT_DEPTH`
(default 10). WS connections are **public**; no auth.

## Heap Allocation on Hot Path

The marketdata fan-out path uses three documented `msg.clone()`
sites because `push_to_client` requires an owned `String` per
subscriber's `VecDeque<String>`:

- L2 delta clone per depth subscriber (`main.rs::broadcast_updates`)
- BBO clone per BBO subscriber (same function)
- Trade clone per trades subscriber (`main.rs::handle_fill`)

These are tagged with `HEAP:` comments and are accepted as part
of the JSON broadcast contract per spec. A binary fan-out
encoding would replace them.

A fourth heap site lives in `handler.rs::ws_read_frame` pong
construction on WS ping receipt; cold path (keepalive cadence).

## Backpressure

Per-client outbound queue is capped at `RSX_MD_MAX_OUTBOUND`
(default 1024). On overflow during fan-out, the client gets a
fresh L2 snapshot instead of a stale delta -- no silent
out-of-order state. Heartbeat timeouts drop the connection.

## Stale Book Eviction

Symbols with zero subscribers and `last_book_access > 60s`
have their `ShadowBook` dropped (`state::evict_stale_books`).
The next subscribe re-materializes the book from incoming events
(and a future replay if cold-started).

## Design Notes

- Single-threaded monoio reactor; no shared mutability across
  threads.
- Separate process from gateway. Public WS; no auth required.
- N CMP/UDP inputs (one per ME) feed into one shadow-book set.
- No durable state -- shadow books are ephemeral and rebuilt
  via DXS bootstrap + live CMP.
- Heartbeat interval/timeout configurable via env.

## Architectural Decisions

**Runtime: monoio (io_uring).** Marketdata fans out L2 depth,
BBO, and trade events to potentially thousands of WS
subscribers. The dominant cost is socket multiplexing — the
exact case io_uring's batched submission rings are designed
for. The reactor also drains one `CmpReceiver` per matching
engine on every tick, so I/O is the inner loop on both sides.

Single-threaded reactor, no `core_affinity` pinning:
marketdata is not on the GW→ME→GW critical path, so the
extra core is better spent on gateway/risk. The shadow book
is single-owner per-process state, but the surrounding loop
is async (not a strict tile) because WS fan-out dominates.
See [`../notes/tiles.md`](../notes/tiles.md).
