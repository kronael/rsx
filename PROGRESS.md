# Progress

## Timeline

```
Feb 7 22:13  first commit (networking spec)
Feb 8 23:15  all 36 CRITIQUE.md items resolved
Feb 9 06:58  orderbook + matching logic shipped
Feb 9 07:47  refined (warnings cleared)
Feb 9 08:30  DXS + recorder shipped
Feb 9 09:00  TILES.md + blog post 4
```

33 hours from first spec to working orderbook + matching logic.
36 hours to WAL/streaming infrastructure.

## What Shipped

Five crates: `rsx-types`, `rsx-book`, `rsx-matching`,
`rsx-dxs`, `rsx-recorder`.

**rsx-types** (55 lines) -- Price/Qty newtypes (i64,
repr(transparent)), Side/TimeInForce enums, SymbolConfig,
validate_order. 12 tests.

**rsx-book** (1,263 lines) -- The core orderbook:
- Slab arena allocator (generic, O(1) alloc/free)
- CompressionMap (5-zone price indexing, ~617K slots)
- PriceLevel (24 bytes, compile-time assert)
- OrderSlot (128 bytes, align(64), compile-time assert)
- Matching algorithm (GTC/IOC/FOK, smooshed tick support)
- Incremental CoW recentering (frontier-based migration)
- User position tracking (reduce-only enforcement)
- Event buffer (fixed array, no heap)
- 50 tests across 7 test files

**rsx-matching** (40 lines) -- Binary stub with main loop
skeleton, panic handler, busy-spin. SPSC ring wiring TODO.

**rsx-dxs** (~1,558 lines) -- WAL + event streaming:
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
- 39 tests, 8 Criterion benchmarks

**rsx-recorder** (150 lines) -- Daily archival consumer:
- Connects via DxsConsumer, writes same WAL format to
  `archive/{stream_id}/{stream_id}_{YYYY-MM-DD}.wal`
- UTC midnight rotation, buffered writes, flush every 1000
- Config from env vars

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

**8,856 lines of spec for 3,086 lines of code.** 2.9:1
spec-to-code ratio. The specs cover matching, risk, WAL,
liquidation, mark price, gateway, market data, networking,
and testing -- most of which isn't implemented yet.

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

| Crate | Status | Blocked By |
|-------|--------|------------|
| rsx-matching wiring | stubbed | SPSC rings (rtrb) |
| rsx-risk | not started | rsx-dxs (consumer) |
| rsx-mark | not started | rsx-dxs (consumer) |
| rsx-gateway | not started | monoio WS server |
| rsx-marketdata | not started | rsx-dxs, rsx-book |

Priority: wire rsx-matching (SPSC + WAL), then rsx-risk
(positions + margin), then rsx-gateway (monoio WS).

## Numbers

```
Implementation:     3,086 lines across 5 crates
Tests:              1,115 lines, 114 tests, all passing
Specifications:     8,856 lines across 28 files
Blog:               1,114 lines across 4 posts
Docs:               ~16 root-level markdown files
Ratio spec:code:    2.9:1
```
