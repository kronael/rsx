# RSX System Architecture

Spec-first perpetuals exchange. ~21k LOC Rust across 12 crates.
All specifications in `specs/2/`.

## System Diagram

```
                     Internet
                       |
                   [WebSocket]
                       |
                  +----------+
                  | Gateway  |  auth, rate limit, validation
                  +----+-----+
                       |
                  [casting/UDP]
                       |
                  +----------+
                  |   Risk   |  margin, positions, liquidation
                  |  (shard) |  per user_id hash
                  +----+-----+
                       |
                  [casting/UDP]
                       |
          +------------+------------+
          |            |            |
     +----+----+  +----+----+  +---+-----+
     | ME: BTC |  | ME: ETH |  | ME: ... |
     +---------+  +---------+  +---------+
          |            |            |
     [casting/UDP]    [casting/UDP]    [casting/UDP]
          |            |            |
     +----+----+  +----+----+  +---+-----+
     |Marketdata|  |Recorder |  |  Mark   |
     +---------+  +---------+  +---------+
          |                    (Binance WS)
     [WebSocket]               (Coinbase WS)
          |
       Clients
```

## Process Model

Six process types, each a separate binary:

| Process | Binary | Scope | Count |
|---------|--------|-------|-------|
| Gateway | rsx-gateway | user sessions | N (sharded by user_id) |
| Risk | rsx-risk | positions, margin | N (sharded by user_id) |
| Matching Engine | rsx-matching | orderbook | 1 per symbol |
| Marketdata | rsx-marketdata | shadow book, L2/BBO | 1 per symbol group |
| Mark | rsx-mark | external prices | 1 global |
| Recorder | rsx-recorder | WAL archival | 1 per stream |

Each process is monolithic and single-concern. No distributed
consensus, no Raft, no cross-process coordination within a tier.

## Crate Layout

12 cargo crates. `rsx-messages` was extracted from `rsx-cast`
so transport is now domain-agnostic (no rsx-types prod dep).

```
rsx-types/      Price, Qty, Side, SymbolConfig, newtypes
rsx-cast/        Domain-agnostic transport: WalWriter, WalReader,
                CastSender, CastReceiver, ReplicationService,
                ReplicationConsumer. Versioned wire header
                (version: u8 at byte 0, V1=current).
                No rsx-types prod dep.
rsx-messages/   Exchange wire records: FillRecord, BboRecord,
                Order*, MarkPriceRecord, LiquidationRecord.
                22 size+align compile-time asserts.
rsx-book/       Orderbook, Slab, CompressionMap, PriceLevel
rsx-matching/   ME main loop, fanout, dedup, WAL integration.
                O(1) cancel via FxHashMap<OrderKey, slab_handle>.
rsx-risk/       RiskShard, margin, positions, liquidation,
                funding, persistence, replication
rsx-gateway/    WS handler, hardened JWT (min-32B secret + nbf
                + JtiTracker), bounded per-IP rate limit with
                FIFO eviction (cap 10 000), circuit breaker
rsx-marketdata/ ShadowBook, L2/BBO/trades, subscriptions
rsx-mark/       Aggregator, BinanceSource, CoinbaseSource
rsx-recorder/   Daily WAL archival
rsx-cli/        WAL dump tool
rsx-log/        Off-hot-path logging primitive (SPSC ring →
                drain thread → tracing events)
```

## Communication Patterns

Two transport modes carry identical WAL records:

**Hot path -- casting/UDP:** Live order/fill flow between Gateway,
Risk, and ME. One WAL record per UDP datagram. Aeron-inspired
NAK gap recovery, idle-only heartbeats, no flow control.
Sub-10us same-machine latency on loopback.

**Cold path -- WAL/TCP:** Replay, replication, archival. Plain
TCP byte stream, optional TLS. Used by ReplicationConsumer /
ReplicationService.

```
Gateway --[casting/UDP]--> Risk --[casting/UDP]--> ME
Gateway <--[casting/UDP]-- Risk <--[casting/UDP]-- ME
                       Risk --[SPSC]-----> PG write-behind
                                    ME --[SPSC]--> WAL Writer
                          WAL Writer --[notify]--> ReplicationService
                                    ME --[casting/UDP]--> Marketdata
Mark    --[replication/TCP]--> Risk (mark prices)
```

Wire format: `WAL bytes = disk bytes = wire bytes = memory bytes`.
16-byte WalHeader (with `version: u8` at byte 0) +
`#[repr(C, align(64))]` payload. Zero serialization. CRC32C
(Castagnoli) over payload only. See
`rsx-cast/src/header.rs` (transport + version),
`rsx-cast/src/records.rs` (CastRecord trait + control messages),
`rsx-messages/src/lib.rs` (domain wire records). Trust
boundaries: casting is intentionally unauthenticated (auth lives
at the gateway via JWT and at L3); see CLAUDE.md and
specs/2/4-cast.md §10.4.

## Order Lifecycle

```
1. User sends order via WebSocket JSON
2. Gateway: authenticate (JWT), rate limit, validate tick/lot
3. Gateway: assign UUIDv7 order_id, track in pending map
4. Gateway -> Risk: casting/UDP NewOrder
5. Risk: portfolio margin check, freeze margin
6. Risk -> ME: casting/UDP NewOrder
7. ME: dedup check, match against book (FIFO)
8. ME: emit Fill(s) + OrderDone/OrderFailed to WAL + casting
9. ME -> Risk: casting/UDP Fill(s), OrderDone
10. Risk: apply fills to positions, release frozen margin
11. Risk -> Gateway: casting/UDP Fill(s), OrderDone
12. Gateway: pop from pending, send to user via WebSocket
```

Fills follow ME -> Risk -> Gateway. Orders follow
Gateway -> Risk -> ME. Same WAL record types everywhere.

## Tile Architecture

Each process runs pinned threads (tiles) for its concerns,
connected by SPSC rings (rtrb, 50-170ns per hop):

```
Matching Engine process:
+=========================================================+
|  +-------+  SPSC  +---------+  SPSC  +-----------+      |
|  |Cast   |------->| Matching|------->| WalWriter |      |
|  |Receiver|<------| tile    |------->|           |      |
|  +-------+  fills |         | events +--+--------+      |
|                    +---------+         |                |
|                        |          +----v---------+      |
|                        |          | Replication- |      |
|                        |          | Service tile |      |
|                        |          +--------------+      |
+=========================================================+
```

Within process: SPSC rings (zero syscall, 50-170ns).
Between processes: casting/UDP (hot) or WAL/TCP (cold).
Per-consumer rings: slow marketdata does not stall risk.
Ring full = producer stalls (backpressure, never drop).

## Fixed-Point Arithmetic

All values are i64 in smallest units. No floats anywhere.

```
Price(pub i64)  -- #[repr(transparent)]
Qty(pub i64)    -- #[repr(transparent)]
```

Conversion at API boundary only:
```
price_raw = (human_price / tick_size) as i64
qty_raw   = (human_qty / lot_size) as i64
```

Overflow checked at order entry (Risk pre-trade), not on
hot path. `checked_mul` for notional = price * qty.
See `rsx-types/src/lib.rs`.

## Zero-Heap Hot Path

- Slab arena allocator for orders (pre-allocated Vec)
- Fixed-size event buffer (`[Event; MAX_EVENTS]` with
  `MAX_EVENTS = 65_536`, heap-boxed, reset by
  setting `event_len = 0`)
- No String, no Vec growth, no Box during matching
- `#[repr(C, align(64))]` on all wire structs (cache line)
- Hot/cold field split in OrderSlot (128B, 2 cache lines)

## Performance Targets

| Path | Target |
|------|--------|
| SPSC hop | 50-170ns |
| ME match (per order) | 100-500ns |
| Risk pre-trade check | <5us |
| Risk post-trade (apply fill) | <1us |
| End-to-end GW->ME->GW | <50us (same machine, budget) |
| casting record encode/decode | <50ns (memcpy + CRC) |
| `WalWriter::prepare` + `append_framed` (Vec extend, no disk I/O) | <200ns |
| WAL flush (fsync) | <1ms per 64KB batch |

## Correctness Invariants

1. Fills precede ORDER_DONE (per order)
2. Exactly-one completion per order (DONE xor FAILED)
3. FIFO within price level (time priority)
4. Position = sum of fills (risk engine)
5. Tips monotonic, never decrease
6. Best bid < best ask (no crossed book)
7. SPSC preserves event FIFO order
8. Slab no-leak: allocated = free + active
9. Funding zero-sum across all users per symbol
10. Advisory lock exclusive: one main per shard

See GUARANTEES.md for formal durability and recovery
specifications. See CRITIQUE.md for known gaps.

## Durability Model

- Fills: 0ms loss guarantee (WAL flushed within 10ms)
- Orders: at-most-once (ephemeral, can be lost)
- Positions: 0-100ms loss (reconstructed from fills)
- Backpressure enforced: never drop data silently

Recovery: each component replays from its tip. ME from
snapshot + WAL. Risk from Postgres + replication. Gateway
is stateless. Marketdata rebuilds shadow book from ME WAL.

## Configuration

All config via environment variables. No TOML, no config
files. Each component reads its own address and dependency
addresses:

```
RSX_ME_BTC_ADDR=127.0.0.1:9001
RSX_RISK_ADDR=127.0.0.1:9010
RSX_GATEWAY_ADDR=0.0.0.0:8080
RSX_POSTGRES_URL=postgres://localhost:5432/rsx
```

Components start in any order with exponential backoff
(1s/2s/4s/8s, max 30s). System converges as components
come online.

## Health and Load Endpoints

Each daemon optionally binds a `rsx-health` HTTP server on a
dedicated `std::thread` (off the hot path entirely). Activate
by setting the env var; skip by leaving it unset.

| Daemon | Env var | Default port |
|--------|---------|--------------|
| Gateway | `RSX_GW_HEALTH_ADDR` | 9200 |
| Risk | `RSX_RISK_HEALTH_ADDR` | 9201 |
| Matching Engine | `RSX_ME_HEALTH_ADDR` | 9202 |
| Marketdata | `RSX_MD_HEALTH_ADDR` | 9203 |
| Mark | `RSX_MARK_HEALTH_ADDR` | 9204 |
| Recorder | `RSX_RECORDER_HEALTH_ADDR` | 9205 |

Three HTTP endpoints per daemon:

- `GET /health` — liveness probe. 200 = process alive.
  503 = fatal state. k8s restarts the pod on 503.
- `GET /ready` — readiness probe. 200 = ready for traffic.
  503 = overloaded or warming up. k8s removes the pod from
  the Service load-balancer → sheds load. Risk returns 503
  during `WarmCatchup` (before it wins the advisory lock).
  Gateway returns 503 when pending queue ≥ 90% capacity.
- `GET /metrics` — JSON load snapshot for HPA and ops.
  Returns `HealthSnapshot` JSON with ring occupancy, event
  counters, and saturation (0.0–1.0, highest ring fraction).

### Hot-path cost

Zero. The hot loop does only `AtomicU64::store(n, Relaxed)` or
`fetch_add(1, Relaxed)` on an `Arc<LoadGauges>` shared with the
health thread. No mutex, no allocation, no syscall per message.
The health server reads those atomics with `Relaxed` loads only
when a HTTP request arrives (off the hot path, separate thread).

### k8s usage pattern

```yaml
livenessProbe:
  httpGet: { path: /health, port: 9202 }
  failureThreshold: 3
readinessProbe:
  httpGet: { path: /ready, port: 9202 }
  failureThreshold: 1   # shed immediately on overload
```

HPA can scrape `/metrics` via a custom adapter and scale on
`saturation` (ring fullness) rather than raw CPU.
