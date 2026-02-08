# Market Data Dissemination

## 1. Context

Market data flows from matching engine to users. ME emits only
fills and BBO (cheap, necessary for risk). The MARKETDATA component
maintains a shadow orderbook per symbol (same data structure,
shared `rsx-book` crate) and derives L2 depth, trades, and BBO
from raw order events.

[CONSISTENCY.md](CONSISTENCY.md) already routes Fill,
OrderInserted, OrderCancelled to the MktData SPSC. No changes to
ME event emission needed.

References: [ORDERBOOK.md](ORDERBOOK.md) (book data structure),
[CONSISTENCY.md](CONSISTENCY.md) (event fan-out),
[DXS.md](DXS.md) (WAL/replay), [WEBPROTO.md](WEBPROTO.md)
(wire format), [RISK.md](RISK.md) (user data),
[LIQUIDATOR.md](LIQUIDATOR.md) (liquidation events).

---

## 2. Shared Orderbook Abstraction (rsx-book)

Extract from [ORDERBOOK.md](ORDERBOOK.md) into shared crate:
- PriceLevel (24 bytes: head, tail, total_qty, order_count)
- OrderSlot (128 bytes, cache-aligned, hot/cold split)
- Slab\<T\> arena allocator (Vec + free list)
- CompressionMap (distance-based zones, bisection lookup)
- BookState (Normal / Migrating)
- Book\<O: BookObserver\> struct with all shared operations

Zero-cost: `BookObserver` trait is generic parameter.
Monomorphization produces specialized code for each consumer.
`Book<NoopObserver>` in ME = zero overhead from observer calls.
`Book<MarketDataObserver>` in MARKETDATA = L2 delta emission
inlined at each state change point.

```rust
// rsx-book: zero-cost observer, compiles away when unused
trait BookObserver {
    fn on_level_changed(&mut self, side: u8, price: i64,
        level: &PriceLevel);
    fn on_bbo_changed(&mut self, bid: Option<(i64, &PriceLevel)>,
        ask: Option<(i64, &PriceLevel)>);
}

struct Book<O: BookObserver> {
    active_levels: Vec<PriceLevel>,
    staging_levels: Vec<PriceLevel>,
    orders: Slab<OrderSlot>,
    compression: CompressionMap,
    best_bid_tick: u32,
    best_ask_tick: u32,
    state: BookState,
    observer: O,
}
```

**Shared operations** (same code in ME and MARKETDATA):
- insert_order: allocate slot, append to level, update aggregates
- remove_order: unlink from level, free slot, update aggregates
- reduce_qty: decrease order qty, update level total_qty
- snapshot: iterate populated levels (reuses migration traversal)
- BBO tracking: best_bid_tick/best_ask_tick with linear scan

**ME observer** -- no-op (ME emits through its own event_buf):

```rust
struct NoopObserver;
impl BookObserver for NoopObserver {
    #[inline(always)]
    fn on_level_changed(&mut self, ..) {}
    #[inline(always)]
    fn on_bbo_changed(&mut self, ..) {}
}
// Book<NoopObserver> -- observer calls compile to nothing
```

**MARKETDATA observer** -- emits L2 deltas and BBO to broadcast:

```rust
struct MarketDataObserver {
    l2_deltas: Vec<L2Delta>,
    bbo_changed: bool,
}
impl BookObserver for MarketDataObserver {
    #[inline(always)]
    fn on_level_changed(&mut self, side, price, level) {
        self.l2_deltas.push(L2Delta {
            side, price,
            total_qty: level.total_qty,
            order_count: level.order_count,
        });
    }
    #[inline(always)]
    fn on_bbo_changed(&mut self, ..) {
        self.bbo_changed = true;
    }
}
```

**Not shared** (component-specific):
- Matching loop (ME only -- aggresses against opposite side)
- L2/trade derivation (MARKETDATA only)
- Event routing / drain_events (ME only)

**Same snapshot codec**: `Book::snapshot()` traversal works
identically for ME snapshots (WAL/recovery) and MARKETDATA
snapshots (L2 depth serving). Same serialization code.

---

## 3. Component Architecture

```
ME ──[SPSC: Fill, OrderInserted, OrderCancelled]──> MARKETDATA
                                                        |
                                                  Book<MarketDataObserver>
                                                  per symbol (shadow book)
                                                        |
                                             ┌──────────┴──────────┐
                                             |                     |
                                      WS subscribers         DXS WalWriter
                                      (live updates)         (replay/archive)
```

Single-threaded, dedicated core, busy-spin. Non-blocking epoll
for WS I/O (no Tokio).

Consumes from one SPSC ring per ME. Maintains per-symbol
`Book<MarketDataObserver>`. Observer captures level changes as
L2 deltas and BBO changes. After processing each event batch,
broadcasts accumulated deltas to subscribed clients.

---

## 4. Event Processing

MARKETDATA receives raw ME events and applies them to its
shadow book. The BookObserver captures all state changes:

```
fn process_event(book: &mut Book<MarketDataObserver>, event):
    match event:
        OrderInserted { handle, price, qty, side, user_id } =>
            // Insert into shadow book -- observer fires
            // on_level_changed for the affected level
            book.insert_order(price, qty, side, user_id)

        Fill { maker_handle, price, qty, taker_side, .. } =>
            // Reduce maker's order -- observer fires
            // on_level_changed for the affected level
            book.reduce_qty(maker_handle, qty)
            if maker_order.remaining_qty == 0:
                book.remove_order(maker_handle)
            // Emit trade (public: no user IDs)
            emit_trade(price, qty, taker_side, timestamp_ns)

        OrderCancelled { handle, .. } =>
            // Remove from book -- observer fires
            // on_level_changed
            book.remove_order(handle)

    // After processing batch:
    let deltas = book.observer.drain_deltas()
    broadcast_l2_deltas(deltas)
    if book.observer.bbo_changed:
        broadcast_bbo(book.best_bid(), book.best_ask())
        book.observer.bbo_changed = false
```

L2 deltas are absolute state (total_qty, order_count at price
level), not incremental. Client applies by setting level state.
`total_qty == 0` means level removed.

---

## 5. Main Loop

```
loop {
    // 1. Drain ME events (highest priority)
    for ring in me_rings:
        while let Ok(event) = ring.try_pop():
            process_event(&mut books[event.symbol_id], event)
            wal.append(event)
        // Flush accumulated deltas after each ring drain
        flush_broadcasts(&mut books[event.symbol_id])

    // 2. Accept new WS connections
    ws_server.accept_pending()

    // 3. Process subscription messages
    ws_server.process_client_messages()

    // 4. Flush WAL (every 10ms)
    wal.maybe_flush()
}
```

---

## 6. Subscription Protocol (Wire Format)

Same compact JSON as [WEBPROTO.md](WEBPROTO.md). Separate public
WS endpoint (no auth required for market data).

**Client -> Server:**

```
{S:[sym, channels]}     // subscribe
{X:[sym, channels]}     // unsubscribe
{X:[0, 0]}              // unsubscribe all
```

Channels bitmask: 1=bbo, 2=depth, 4=trades.

**Server -> Client:**

```
// BBO update
{B:[sym, bp, bq, bc, ap, aq, ac]}

// L2 snapshot (on subscribe, then incrementals)
// Uses Book::snapshot() -- same traversal as ME snapshots
{L:[sym, seq, [[p,q,c], ...bids], [[p,q,c], ...asks]]}

// L2 delta (incremental)
{D:[sym, seq, side, p, q, c]}
// q=0 means level removed

// Trade
{T:[sym, p, q, s, ts]}
```

**Snapshot-then-incremental protocol:**
1. On depth subscribe: assign from_seq = current update_seq
2. Send L2 snapshot via Book::snapshot() with seq
3. Stream deltas for seq > from_seq
4. Client applies deltas on top of snapshot
5. On gap: client re-subscribes for fresh snapshot

No server-side buffering. Client re-subscribes if behind.
Keeps server stateless per-client (just a subscription bitmask).

**Subscription state:**

```rust
struct SymbolSubs {
    bbo_clients: Vec<u64>,
    depth_clients: Vec<u64>,
    trade_clients: Vec<u64>,
}
```

Broadcast fan-out: iterate per-symbol subscriber list. O(C)
per event.

---

## 7. User Data Stream (Private, via Gateway)

Liquidation events flow risk -> gateway -> user on the existing
private WS ([WEBPROTO.md](WEBPROTO.md)). New message type:

```
// Liquidation event
{Q:[sym, status, round, side, qty, price, slip_bps]}
// status: 0=started, 1=round_placed, 2=filled,
//         3=cancelled, 4=completed
```

Risk engine pushes to gateway SPSC ring (same path as
fills/order updates). Gateway routes to user's WS by user_id.
Fire-and-forget delivery. Already persisted in Postgres
([LIQUIDATOR.md](LIQUIDATOR.md) section 8).

---

## 8. Recovery

MARKETDATA is stateless -- no Postgres. Recovery via event
replay from ME WAL.

1. Start with empty Book\<MarketDataObserver\> per symbol
2. Connect as DXS consumer to each ME's DxsReplay server
3. Replay from tip + 1 (OrderInserted/Fill/OrderCancelled)
4. Apply to shadow book -- rebuilds full state
5. On CaughtUp: transition to live SPSC processing
6. Start accepting WS connections

Same DXS consumer pattern as risk engine
([DXS.md](DXS.md) section 6). Tip persisted every 10ms to
tip_file. On crash, replay from last tip. Events are idempotent
when replayed in order.

---

## 9. Config

```toml
[marketdata]
listen_addr = "0.0.0.0:9300"
wal_dir = "./wal/marketdata"
depth_levels = 20               # top-N per side for snapshots
trade_history = 1000            # recent trades ring buffer
max_clients = 10000

[[marketdata.symbols]]
symbol_id = 1
me_addr = "10.0.0.1:9100"      # DXS replay for recovery
```

---

## 10. Performance Targets

| Path | Target |
|------|--------|
| Event processing (insert) | <200ns |
| Event processing (fill) | <200ns |
| L2 delta derivation per level | <50ns |
| Trade derivation per fill | <30ns |
| BBO derivation | <50ns |
| L2 snapshot (20 levels) | <10us |
| WS broadcast per client | <1us |
| WAL replay 100K records | <1s |
| End-to-end ME -> client | <100us |
| Book memory per symbol | ~10GB (same as ME) |

---

## 11. File Organization

```
crates/rsx-book/src/
    lib.rs           -- pub API: Book, BookObserver, PriceLevel
    level.rs         -- PriceLevel, level operations
    order.rs         -- OrderSlot, hot/cold split
    slab.rs          -- Slab<T> arena allocator
    compression.rs   -- CompressionMap, zone lookup
    migration.rs     -- BookState, recentering
    snapshot.rs      -- Book::snapshot() traversal + codec

crates/rsx-marketdata/src/
    main.rs          -- entrypoint, config, main loop
    observer.rs      -- MarketDataObserver, L2Delta, Trade
    consumer.rs      -- SPSC consumer, event dispatch
    ws.rs            -- WebSocket server, subscriptions
    broadcast.rs     -- fan-out to subscribed clients
    recovery.rs      -- DXS consumer, WAL replay, tip tracking
    config.rs        -- TOML config structs
    types.rs         -- PublicTrade, channel bitmask
```

---

## 12. Tests

### Unit Tests (rsx-book)

```
// level operations
insert_order_updates_level_qty
insert_order_increments_count
remove_order_decrements_level_qty
remove_order_decrements_count
reduce_qty_updates_level_total
level_empty_after_all_orders_removed
insert_multiple_orders_same_level
remove_from_middle_of_level

// observer callbacks
observer_called_on_insert
observer_called_on_remove
observer_called_on_reduce_qty
observer_called_on_bbo_change
noop_observer_compiles_away
observer_receives_correct_level_state
observer_not_called_when_level_unchanged

// slab
slab_alloc_returns_sequential
slab_free_reuses_slot
slab_alloc_after_free_returns_freed
slab_mixed_alloc_free_no_leak

// compression
price_to_index_zone_0_1_to_1
price_to_index_zone_1_compressed
price_to_index_zone_4_catchall
price_roundtrip_zone_0

// snapshot
snapshot_iterates_populated_levels_only
snapshot_bids_descending_asks_ascending
snapshot_empty_book_returns_empty
snapshot_same_result_me_and_marketdata
```

### Unit Tests (rsx-marketdata)

```
// observer / L2 derivation
order_insert_produces_l2_delta
fill_produces_l2_delta_and_trade
cancel_produces_l2_delta
level_removed_produces_delta_zero_qty
multiple_changes_same_level_single_delta
bbo_change_detected
bbo_unchanged_no_signal
trade_strips_user_ids

// WS / subscriptions
subscribe_bbo_channel
subscribe_depth_channel
subscribe_trades_channel
subscribe_multiple_channels_bitmask
unsubscribe_single_channel
unsubscribe_all
subscribe_sends_snapshot_then_deltas
broadcast_bbo_to_subscribed_only
broadcast_trade_to_subscribed_only
broadcast_delta_to_subscribed_only
unsubscribed_client_receives_nothing
client_disconnect_cleanup
```

### E2E Tests

```
subscribe_get_snapshot_then_deltas
subscribe_bbo_receives_updates
subscribe_trades_receives_fills
new_client_gets_consistent_snapshot
delta_applied_to_snapshot_matches_full_book
recovery_replay_rebuilds_correct_state
multiple_symbols_independent
heavy_update_burst_no_data_loss
client_reconnect_fresh_snapshot
1000_clients_subscribed_broadcast
liquidation_event_reaches_user_ws
shadow_book_matches_me_book_after_replay
```

### Benchmarks

```
bench_book_insert_order             // target <100ns
bench_book_remove_order             // target <100ns
bench_book_reduce_qty               // target <100ns
bench_observer_l2_delta             // target <50ns
bench_snapshot_20_levels            // target <10us
bench_broadcast_100_clients_bbo     // target <100us
bench_broadcast_1000_clients_bbo    // target <1ms
bench_replay_100k_events            // target <1s
bench_noop_observer_no_overhead     // verify zero-cost
```

---

## 13. Implementation Phases

### Phase 1: rsx-book extraction (shared crate)
- Extract PriceLevel, OrderSlot, Slab, CompressionMap, Book
  from ORDERBOOK.md design into rsx-book
- Add BookObserver trait (generic, zero-cost)
- NoopObserver for ME, verify no overhead
- Snapshot traversal + codec

### Phase 2: MARKETDATA core (shadow book + observer)
- MarketDataObserver: L2Delta, Trade derivation
- SPSC consumer applying events to Book\<MarketDataObserver\>
- Verify shadow book matches ME state

### Phase 3: WebSocket + subscription
- WS server with subscribe/unsubscribe
- Snapshot-then-incremental protocol
- L2/BBO/trade broadcast

### Phase 4: Recovery (DXS replay)
- DXS consumer for WAL replay on startup
- Tip persistence, CaughtUp transition

### Phase 5: User data stream (gateway changes)
- Add Q message to WEBPROTO
- Risk pushes liquidation events to gateway SPSC
- Gateway routes to user WS
