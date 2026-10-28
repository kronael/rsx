# rsx-marketdata Architecture

Market data process. Aggregates events from N matching engines,
maintains a shadow orderbook per symbol, and publishes L2 depth,
BBO, and trades to subscribed WebSocket clients.

Spec: `specs/2/16-marketdata.md`. WS client protocol shares the
envelope conventions of `specs/2/49-webproto.md`.

## Runtime Model

Single monoio (io_uring) reactor on one thread. All state lives
in `Rc<RefCell<MarketDataState>>`; no locks, no cross-thread
sharing. The main loop iterates over one casting receiver per
matching engine, dispatches records to the shadow book,
broadcasts deltas/BBO/trades, and runs heartbeat + stale-book
eviction sweeps.

## Module Layout

| File | Purpose |
|------|---------|
| `main.rs` | Binary: monoio runtime, replay bootstrap, per-ME casting loop, dispatch, heartbeat, book eviction |
| `lib.rs` | Re-exports |
| `apply.rs` | `apply_record`: decode one ME record, fold it into the shadow book, push L2 / BBO / trade frames; shared by the live loop, bootstrap and gap replay |
| `config.rs` | `MarketDataConfig` from env; `me_cast_addrs_from_env` parses multi-ME list |
| `state.rs` | `MarketDataState`, connection registry, subscription manager wrapper, per-symbol seq tracking, snapshot dispatch |
| `shadow.rs` | `ShadowBook` -- wraps `rsx_book::Orderbook` with order-id index for `apply_*_by_order_id` |
| `subscription.rs` | `SubscriptionManager` -- per-client channels (BBO/depth/trades) + depth pref |
| `records.rs` | JSON envelope serializers (`BBO`/`B`/`D`/`T`/`H`) + client frame parser (`S`/`X`/`H`) |
| `handler.rs` | Per-connection: handshake, subscribe/unsubscribe/heartbeat, outbound drain |
| `ws.rs` | WebSocket handshake (no auth, `WS_UPGRADE_DEADLINE` from `rsx-types`) + frame I/O: client frames must be masked, FIN=1, at most 4 KiB |
| `types.rs` | `BboUpdate`, `L2Snapshot`, `L2Delta`, `L2Level`, `TradeEvent` |
| `replay.rs` | replication (TCP): `drain_replay` for a FAULTED / RECONNECT gap, bootstrap before going live |

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
casting port. Marketdata opens **one `CastReceiver` per ME**:

- ME addresses come from `RSX_ME_CAST_ADDRS` (comma-separated)
  or `RSX_ME_CAST_ADDR` (single)
- For each ME at port P, marketdata binds its local recv side
  at port **P + 400** (e.g. ME 9100 -> MD 9500)
- The main loop iterates over the receiver vector each pass,
  draining all available records, then yields to the reactor with a
  zero-duration sleep

Per-stream sequence tracking (one expectation per CastReceiver)
detects a genuine loss on a stream independently of the others.

## Sequence Gap Detection & Recovery

Seq continuity is tracked per stream in the main loop
(`stream_expected[ri]`), not per symbol. ME's SEQ-1 fan-out puts
every record type on one monotonic per-stream seq -- including
the `ORDER_DONE` / `ORDER_FAILED` / `BBO` / `CONFIG_APPLIED`
records marketdata receives but ignores -- so the counter
advances on EVERY record, not only the handled ones. For each
record's `seq` vs the stream's `expected`:

- First record initializes the expectation
- `seq == expected` -> advance
- `seq > expected` -> gap; the `CastReceiver` delivers
  contiguously or FAULTs, so a hole here is a genuine
  unrecoverable loss. Bump `gap_count` (`note_gap`), log a
  warning, and `resend_all_snapshots` -- broadcast a fresh L2
  snapshot on every symbol so clients rebuild.
- `seq < expected` -> duplicate, ignored

`resend_snapshot` also fires when a per-client outbound queue
overflows (`max_outbound`) -- drop deltas, send a snapshot,
let the client resync.

## Cold-Path Bootstrap (replication/TCP)

When `RSX_MD_REPLAY_ADDR` is set, `replay::run_replay_bootstrap_blocking`
connects to the replication endpoint, drains `OrderInserted` /
`OrderCancelled` / `Fill` records up to `CaughtUp`, and replays
them into the shadow books before the live casting loop starts.
Persisted tip at `RSX_MD_TIP_FILE` makes restart idempotent. A round that
goes quiet for one second is dropped and the rest requested from the tip, so
a reply left in the server's TLS send buffer cannot hold the boot.

A cast receiver that FAULTs or needs a RECONNECT drains the same stream
from its last delivered seq with `replay::drain_replay`. Every record
goes through `apply::apply_record`, the function the live loop and the
bootstrap call, then `reset_after_replay` resumes at the new tip. The
live-stream seq-gap detector resends L2 snapshots to subscribers on the
first packet after the resume.

The blocking wrapper uses a single-threaded tokio runtime
inside `run_replay_bootstrap_blocking`; the live path remains
pure monoio.

## Publishing

| Feed | Trigger | `MdFrame` body |
|------|---------|---------------|
| Snapshot | On subscribe (depth channel), on seq gap, on outbound overflow | `Snapshot` |
| L2 delta | Any level change | `Delta` |
| BBO | Best level change (deduped via `last_bbo`) | `Bbo` |
| Trades | Each fill | `Trade` |
| Heartbeat | Every `RSX_MD_HEARTBEAT_INTERVAL_S` | `Heartbeat` |

Every feed frame is one WebSocket binary message: a 4-byte big-endian
length prefix, then a protobuf `MdFrame`. The schema is
`marketdata.proto`.

`wire.rs` owns the feed both ways: the `encode_*` functions and
`wire::decode`, its exact inverse, share one set of wire structs. `rsx-cli md
snapshot|follow` decodes with it.

Per-client subscriptions are checked via `has_bbo` / `has_depth`
/ `has_trades` before each push.

## Client Protocol

Client frames are JSON text; the feed back is binary protobuf.

| Op | Frame |
|----|-------|
| Subscribe | `{"S":[sym,channels]}` -- channels bitmask (1=BBO, 2=depth, 4=trades) |
| Unsubscribe | `{"X":[sym,channels]}` -- `sym==0` -> all |
| Heartbeat | `{"H":[timestamp_ms]}` -- answered with a `Heartbeat` frame |

Subscribing to depth (`channels & 2`) immediately sends an L2
snapshot at `RSX_MD_SNAPSHOT_DEPTH` (default 10). A subscribe does not
create the book, so the snapshot is empty until the first
`OrderInserted` for the symbol creates it. WS connections are
**public**; no auth.

## Heap Allocation on Hot Path

The marketdata fan-out path uses three documented `msg.clone()`
sites because `push_to_client` takes an owned `Arc<[u8]>` per
subscriber's `VecDeque`:

- L2 delta clone per depth subscriber (`apply.rs::broadcast_updates`)
- BBO clone per BBO subscriber (same function)
- Trade clone per trades subscriber (`apply.rs::apply_fill`)

These are tagged with `HEAP:` comments. Each clone is one `Arc`
reference-count bump per subscriber per frame.

A fourth heap site lives in `handler.rs::ws_read_frame` pong
construction on WS ping receipt; cold path (keepalive cadence).

## Backpressure

Per-client outbound queue is capped at `RSX_MD_MAX_OUTBOUND`
(default 1024). On overflow of a delta or trade push, the queue is
cleared and the client gets a fresh L2 snapshot instead -- no silent
out-of-order state. An overflowing BBO frame is dropped, the next BBO
supersedes it. Heartbeat timeouts drop the connection.

## Stale Book Eviction

Symbols with zero subscribers and `last_book_access > 60s`
have their `ShadowBook` dropped (`state::evict_stale_books`).
The next subscribe re-materializes the book from incoming events
(and a replay bootstrap if cold-started).

## Design Notes

- Single-threaded monoio reactor; no shared mutability across
  threads.
- Separate process from gateway. Public WS; no auth required.
- N casting/UDP inputs (one per ME) feed into one shadow-book set.
- No durable state -- shadow books are ephemeral and rebuilt
  via replication bootstrap + live casting.
- Heartbeat interval and idle timeout come from env
  (`RSX_MD_HEARTBEAT_INTERVAL_S`, `RSX_MD_IDLE_TIMEOUT_S`).

## Architectural Decisions

**Runtime: monoio (io_uring).** Marketdata fans out L2 depth,
BBO, and trade events to potentially thousands of WS
subscribers. The dominant cost is socket multiplexing — the
exact case io_uring's batched submission rings are designed
for. The reactor also drains one `CastReceiver` per matching
engine on every pass, so I/O is the inner loop on both sides.

Single-threaded reactor. When `RSX_MD_CORE_ID` is set, the main thread
pins to that core and locks its memory (`setup_hot_thread`) before the
loop starts. Without it the thread is unpinned.

Marketdata is not on the GW→ME→GW critical path, but it drains one
casting receiver per matching engine, and a receive buffer overflow
forces a replay. The pinned core keeps the reactor ahead of the casting
stream. The shadow book is single-owner per-process state, and the
surrounding loop is a monoio reactor, not a strict tile, because WS
fan-out dominates. The loop yields to the reactor with a zero-duration
sleep on each pass.

See [`../docs/concepts/02-tiles-and-pinning.md`](../docs/concepts/02-tiles-and-pinning.md).
