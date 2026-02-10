# Progress

## Timeline

```
Feb 7 22:13  first commit (networking spec)
Feb 8 23:15  all 36 CRITIQUE.md items resolved
Feb 9 06:58  orderbook + matching logic shipped
Feb 9 07:47  refined (warnings cleared)
Feb 9 08:30  DXS + recorder shipped
Feb 9 09:00  TILES.md + blog post 4
Feb 9 12:00  rsx-risk shipped (margin, funding, persist, shard)
Feb 9 15:00  rsx-mark shipped (aggregator, config, main)
Feb 9 15:00  ME timestamp fix (real ns clock)
Feb 9 19:30  rsx-risk Phase 3: Postgres persistence
Feb 9 21:00  rsx-gateway shipped (protocol, rate limit, circuit
             breaker, pending orders, wire types, UUIDv7)
Feb 9 21:30  rsx-marketdata shipped (shadow book, BBO, L2,
             trades, subscriptions, WS protocol)
Feb 9 22:00  spec compliance audit: 14 tests added,
             clippy fixes across rsx-mark
Feb 9 23:30  CRITIQUE fixes: real order IDs, risk binary,
             DXS sidecar, production panic/retry patterns,
             rsx-types macros crate
```

33 hours from first spec to working orderbook + matching logic.
36 hours to WAL/streaming infrastructure.
42 hours to risk engine + mark price aggregator.
48 hours to all 9 crates shipping (pure logic complete).

## What Shipped

Nine crates: `rsx-types`, `rsx-book`, `rsx-matching`,
`rsx-dxs`, `rsx-recorder`, `rsx-risk`, `rsx-mark`,
`rsx-gateway`, `rsx-marketdata`.

**rsx-types** (~150 lines) -- Price/Qty newtypes (i64,
repr(transparent)), Side/TimeInForce/OrderStatus/FinalStatus/
FailureReason enums, SymbolConfig, validate_order.
Production macros: install_panic_handler, DeferCall/defer!,
on_error_continue!, on_none_continue!, on_error_return_ok!,
on_none_return_ok!. 15 tests.

**rsx-book** (1,342 lines) -- The core orderbook:
- Slab arena allocator (generic, O(1) alloc/free)
- CompressionMap (5-zone price indexing, ~617K slots)
- PriceLevel (24 bytes, compile-time assert)
- OrderSlot (128 bytes, align(64), compile-time assert)
- Matching algorithm (GTC/IOC/FOK, smooshed tick support)
- Incremental CoW recentering (frontier-based migration)
- User position tracking (reduce-only enforcement)
- Event buffer (fixed array, no heap)
- 75 tests across 7 test files

**rsx-matching** (~600 lines) -- ME binary with main loop,
WAL integration, wire format encoding, fanout to SPSC
rings, panic handler, busy-spin. Real nanosecond timestamps.
DXS sidecar (spawns DxsReplayService if RSX_ME_DXS_ADDR set).
Real order IDs (UUIDv7 hi/lo) wired through all events/WAL.
11 tests (5 fanout + 2 WAL integration + 4 wire).

**rsx-dxs** (1,488 lines) -- WAL + event streaming:
- WalWriter: append is memcpy (0ns I/O), flush+fsync every
  10ms, rotation at 64MB, GC past retention, backpressure
  stalls producer at 2x buffer limit
- WalReader: sequential read, CRC32 validation, invalid CRC
  truncates stream, unknown version fails fast, file
  transition across rotated + active files
- 8 record types: Fill, Bbo, OrderInserted, OrderCancelled,
  OrderDone, ConfigApplied, CaughtUp, OrderAccepted -- all
  `#[repr(C, align(64))]`
- 16-byte header: version(2)+type(2)+len(4)+stream_id(4)+crc32(4)
- DxsReplayService: tonic gRPC, historical replay + CaughtUp
  marker + live tail via tokio::sync::Notify
- DxsConsumer: gRPC client, tip persistence (atomic write
  every 10ms), reconnect backoff 1/2/4/8/30s
- Config: env only
- 68 tests, 8 Criterion benchmarks

**rsx-recorder** (138 lines) -- Daily archival consumer:
- Connects via DxsConsumer, writes same WAL format to
  `archive/{stream_id}/{stream_id}_{YYYY-MM-DD}.wal`
- UTC midnight rotation, buffered writes, flush every 1000
- Config from env vars

**rsx-risk** (~1,600 lines src + 2,250 lines tests + 72 SQL)
Risk engine per user shard with binary entry point:
- Account state (collateral, frozen margin, version tracking)
- Margin checking (initial/maintenance, portfolio offset)
- Fee calculation (floor division, rebates)
- Funding rate + settlement (8h intervals, zero-sum,
  clamp to bounds, idempotent)
- Exposure tracking (per-symbol user sets)
- Shard orchestration (main loop, WAL consumer, persist)
- Phase 3: Postgres persistence layer:
  - Write-behind worker: SPSC ring from hot path, 10ms
    flush, single tx per batch (positions/accounts/fills/
    tips/funding payments)
  - Cold start: load from Postgres, replay WAL from tips
  - Schema migration (idempotent DO $migration$ pattern)
  - Advisory lock (pg_advisory_lock per shard)
- Config from env vars
- Tests: 72 total (57 pass without Docker,
  15 Docker-gated via testcontainers)
  Phase 1 unit tests: complete (55/55 unit)
  Phase 2 shard tests: 2 non-Docker tests pass
  Phase 3 persist tests: 17 total (15 need Docker,
    2 pass without: backpressure_ring_full,
    replay_from_wal_rebuilds_positions)

**rsx-mark** (384 lines) -- Mark price aggregator:
- MarkPriceEvent (64-byte repr(C) WAL record)
- SymbolMarkState (8 sources, bitmask, staleness tracking)
- Median aggregation (lower median for even count > 2,
  average for 2 sources)
- Staleness sweep (10s threshold, 1s interval)
- PriceSource trait (exchange connector interface)
- Main loop skeleton (drain -> sweep -> WAL flush)
- Config from env vars (per-source settings)
- 40 tests (27 aggregator + 7 types + 6 config)

**rsx-gateway** (1,172 lines) -- Gateway tile pure logic:
- WS protocol parser/serializer (672 lines): N/C/U/F/E/H/Q
  frames per WEBPROTO.md, single-letter JSON keys, positional
  arrays, BBO/L2 snapshot/delta frames
- Fixed-point conversion: price_to_fixed, qty_to_fixed,
  validate_tick_alignment, validate_lot_alignment
- Token bucket rate limiter: 10/s per user, 100/s per IP,
  1000/s per instance, refill over elapsed time
- Circuit breaker: closed/open/half-open state machine,
  10 failures threshold, 30s cooldown, probe-on-half-open
- Pending orders: VecDeque LIFO pop, linear scan fallback,
  10k cap backpressure, stale order timeout removal
- Wire types: RiskNewOrder, RiskCancelOrder, RiskOrderUpdate,
  OrderFill, StreamError -- all #[repr(C, align(64))]
- UUIDv7 order ID generation (16 bytes, time-sortable,
  monotonic within millisecond)
- Config from env vars (RSX_GW_* prefix)
- Main loop skeleton (panic handler, config, spin_loop)
- 91 tests across 8 test files

**rsx-marketdata** (643 lines) -- Marketdata tile pure logic:
- ShadowBook wrapping rsx_book::Orderbook: apply_fill,
  apply_insert, apply_cancel, derive_bbo, derive_l2_snapshot,
  derive_l2_delta, make_trade
- Types: BboUpdate, L2Level, L2Snapshot, L2Delta, TradeEvent,
  MarketDataMessage enum
- WS protocol: serialize BBO/L2 snapshot/L2 delta/trade
  frames, parse S (subscribe) and X (unsubscribe) frames,
  sequence numbers per spec
- SubscriptionManager: per-client symbol+channel tracking,
  BBO/depth channel bitmask, depth parameter (10/25/50),
  subscribe/unsubscribe/unsubscribe_all
- Config from env vars (RSX_MD_* prefix)
- Main loop skeleton (panic handler, config, SPSC drain ->
  shadow book -> broadcast)
- 57 tests across 4 test files

## Infrastructure

- Release profile: `lto=true`, `codegen-units=1`,
  `strip=true`, `panic=abort`
- Linker: mold via `.cargo/config.toml` (fast dev builds)
- WAL wire format: raw #[repr(C)] fixed records, no protobuf
- Networking spec: `specs/v1/TILES.md` -- tile-based
  architecture, monoio/io_uring, pluggable I/O, SPSC rings

## Shocking Parts

**The sonnet.** The README opens with a love poem to the
exchange architecture. "Thy slab did catch me: firm,
pre-allocated / No malloc on thy hot path -- O! how pure."

**36 critique items before any code.** The spec went through
a full adversarial audit (9 critical, 12 high, 15 medium)
and every item was resolved before writing a single line
of Rust. Items like "IOC/FOK missing from matching",
"dedup window unsafe across restarts", "no clock sync for
funding settlement".

**8,883 lines of spec for 7,310 lines of code.** 1.2:1
spec-to-code ratio. The specs cover matching, risk, WAL,
liquidation, mark price, gateway, market data, networking,
and testing. Implementation now covers all nine crates.

**The infinite loop bug.** First test run hung on
`match_no_cross_taker_rests`. A buy at 50,100 with best
ask at 50,200 should rest without matching. But the
matching loop checked `remaining_qty > 0 && best_ask != NONE`
without verifying the price actually crosses. Fix: track
remaining_qty before/after, break if unchanged.

**The hook that keeps reverting Cargo.toml.** A pre-commit or
post-write hook keeps resetting the workspace members list.
Every edit to any file triggers it. Had to re-add crates to
the members list repeatedly during sessions.

**128 bytes of OrderSlot, designed by hand.** Hot fields
(price, qty, side, flags, next/prev) in cache line 1.
Cold fields (user_id, timestamp, original_qty) in cache
line 2. 40 bytes of explicit padding to hit exactly 128.

**CompressionMap: 617K slots instead of 20M.** Naive
price-to-index for BTC at $0.01 ticks covering $1K-$200K
needs 20M slots (477 MB). Five compression zones reduce
this to ~617K slots (~14.8 MB).

**~50ns WAL append.** The WAL write is a memcpy, not a disk
operation. Durability is on a 10ms timer. Bounded data loss
is the design -- deterministic replay + client retry + dedup
makes this correct.

**fsync is 1000x more expensive than serialization.** The wire
format debate (protobuf vs FlatBuffers vs raw structs) is
noise. fsync is 200us-2ms. Serialization is 50-200ns. The only
optimization that matters is batching fsync.

**Backpressure stalls the matching engine.** Binary, not
graceful. Buffer full = no more orders. The alternative
(drop events, degrade silently) is worse -- positions
diverge, fills vanish, P&L is fiction.

**No Kafka.** The WAL file IS the stream. Reader tails the
file. Replay service reads from the beginning. No broker, no
ZooKeeper, no consumer groups. The recorder IS the replica --
it's a consumer that writes to a different disk.

**tonic gRPC on the cold path.** Streaming of raw #[repr(C)] WAL
records over HTTP/2. Same bytes on disk, on wire, in memory. Hot
path optimizations are deferred. Gateway + market data use monoio
(io_uring). Reference impl in `../trader/monoio-client/` is
production-proven.

## Blog

4 posts in `blog/`:

1. **dont-yolo-structs-over-the-wire.md** -- 9 ways raw
   structs bite you (alignment, endianness, versioning,
   torn reads, transmute, invalid enums, floats, DoS,
   framing)
2. **flatbuffers-isnt-free.md** -- 22x write overhead,
   2.5x wire bloat, pointer chasing, immutable mutation
3. **picking-a-wire-format.md** -- raw structs vs protobuf vs
   FlatBuffers vs Cap'n Proto. Hybrid: FlatBuffers external,
   raw structs internal
4. **your-wal-is-lying-to-you.md** -- bounded data loss,
   fsync dominance, backpressure, no Kafka

## What's Next

All pure logic is shipped. Remaining work is networking
and system integration:

| Area | Description | Blocked By |
|------|-------------|------------|
| monoio WS | Gateway + marketdata I/O layer | monoio setup |
| QUIC transport | quinn inter-process communication | quinn setup |
| SPSC rings | rtrb intra-process IPC (real impl) | - |
| System integration | Wire all tiles together | above 3 |
| Liquidator | LIQUIDATOR.md implementation | risk + matching |
| rsx-risk Phase 4-5 | Replication, full system tests | integration |

Priority: SPSC rings (no external deps), then monoio WS,
then QUIC transport, then wire everything together.

## Numbers

```
Implementation:     7,310 lines across 9 crates
                    + 72 lines SQL migration
Tests:              7,941 lines, 414 tests passing
                    (15 more Docker-gated, 429 total)
Specifications:     8,883 lines across 28 files
Blog:               1,114 lines across 4 posts
Docs:               ~16 root-level markdown files
Ratio spec:code:    1.2:1
```

### Per-crate breakdown

```
Crate            Src    Tests  #Tests
rsx-types         88      148      15
rsx-book       1,342    1,271      75
rsx-matching     556      450      11
rsx-dxs        1,488    1,260      68
rsx-recorder     138        -       -
rsx-risk       1,499    2,250   57+15
rsx-mark         384      591      40
rsx-gateway    1,172    1,169      91
rsx-marketdata   643      802      57
─────────────────────────────────────
Total          7,310    7,941     429
```
