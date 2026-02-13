# RSX System Architecture

Spec-first perpetuals exchange. ~34k LOC across 9 Rust crates,
960 tests (all passing, zero flakiness). All specifications
in `specs/v1/`.

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
                  [CMP/UDP]
                       |
                  +----------+
                  |   Risk   |  margin, positions, liquidation
                  |  (shard) |  per user_id hash
                  +----+-----+
                       |
                  [CMP/UDP]
                       |
          +------------+------------+
          |            |            |
     +----+----+  +----+----+  +---+-----+
     | ME: BTC |  | ME: ETH |  | ME: ... |
     +---------+  +---------+  +---------+
          |            |            |
     [CMP/UDP]    [CMP/UDP]    [CMP/UDP]
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

```
rsx-types/      Price, Qty, Side, SymbolConfig, newtypes
rsx-book/       Orderbook, Slab, CompressionMap, PriceLevel
rsx-matching/   ME main loop, fanout, dedup, WAL integration
rsx-dxs/        WalWriter, WalReader, CmpSender, CmpReceiver,
                DxsReplayService, DxsConsumer
rsx-risk/       RiskShard, margin, positions, liquidation,
                funding, persistence, replication
rsx-gateway/    WS handler, JWT, rate limit, circuit breaker
rsx-marketdata/ ShadowBook, L2/BBO/trades, subscriptions
rsx-mark/       Aggregator, BinanceSource, CoinbaseSource
rsx-recorder/   Daily WAL archival
rsx-cli/        WAL dump tool
```

## Communication Patterns

Two transport modes carry identical WAL records:

**Hot path -- CMP/UDP:** Live order/fill flow between Gateway,
Risk, and ME. One WAL record per UDP datagram. Aeron-inspired
NACK + flow control. Sub-10us same-machine latency.

**Cold path -- WAL/TCP:** Replay, replication, archival. Plain
TCP byte stream, optional TLS. Used by DxsConsumer/DxsReplay.

```
Gateway --[CMP/UDP]--> Risk --[CMP/UDP]--> ME
Gateway <--[CMP/UDP]-- Risk <--[CMP/UDP]-- ME
                       Risk --[SPSC]-----> PG write-behind
                                    ME --[SPSC]--> WAL Writer
                          WAL Writer --[notify]--> DxsReplay
                                    ME --[CMP/UDP]--> Marketdata
Mark    --[DXS/TCP]--> Risk (mark prices)
```

Wire format: `WAL bytes = disk bytes = wire bytes = memory bytes`.
16-byte WalHeader + `#[repr(C, align(64))]` payload. Zero
serialization. See `rsx-dxs/src/header.rs`, `rsx-dxs/src/records.rs`.

## Order Lifecycle

```
1. User sends order via WebSocket JSON
2. Gateway: authenticate (JWT), rate limit, validate tick/lot
3. Gateway: assign UUIDv7 order_id, track in pending map
4. Gateway -> Risk: CMP/UDP NewOrder
5. Risk: portfolio margin check, freeze margin
6. Risk -> ME: CMP/UDP NewOrder
7. ME: dedup check, match against book (FIFO)
8. ME: emit Fill(s) + OrderDone/OrderFailed to WAL + CMP
9. ME -> Risk: CMP/UDP Fill(s), OrderDone
10. Risk: apply fills to positions, release frozen margin
11. Risk -> Gateway: CMP/UDP Fill(s), OrderDone
12. Gateway: pop from pending, send to user via WebSocket
```

Fills follow ME -> Risk -> Gateway. Orders follow
Gateway -> Risk -> ME. Same WAL record types everywhere.

## Tile Architecture

Each process runs pinned threads (tiles) for its concerns,
connected by SPSC rings (rtrb, 50-170ns per hop):

```
Matching Engine process:
+===============================================+
|  +-------+  SPSC  +---------+  SPSC  +------+ |
|  |  CMP  |------->| Matching|------->| WAL  | |
|  |Receiver|<------| tile    |------->|Writer| |
|  +-------+  fills |         | events +--+---+ |
|                    +---------+     +----v----+ |
|                        |          |DxsReplay | |
|                        |          |  tile    | |
|                        |          +---------+  |
+===============================================+
```

Within process: SPSC rings (zero syscall, 50-170ns).
Between processes: CMP/UDP (hot) or WAL/TCP (cold).
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
- Fixed-size event buffer (`[Event; 10_000]`, reset by
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
| End-to-end GW->ME->GW | <50us (same machine) |
| CMP encode/decode | <50ns (memcpy) |
| WAL append (in-memory) | <200ns |
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
snapshot + WAL. Risk from Postgres + DXS replay. Gateway
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
