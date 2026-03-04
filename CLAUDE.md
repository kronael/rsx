# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# RSX Exchange

Spec-first perpetuals exchange. All specs in `specs/v1/`.

## Architecture (see specs/v1/NETWORK.md, TILES.md)

- Separate processes: Gateway, Risk, ME (per symbol),
  Marketdata, Recorder, Mark
- Between processes: CMP (C structs over UDP) + WAL
  replication (TCP)
  - Live path: CMP/UDP (order flow, fills)
  - Cold path: WAL replication over TCP (replay, replication)
- Within each process: tile architecture (pinned threads
  + SPSC rings for intra-process IPC, see TILES.md)
- Hot path I/O: monoio (io_uring), not tokio (epoll)
- Later: DPDK/AF_XDP swaps I/O layer, same interfaces
- Target: <50us GW→ME→GW, <500ns ME match
- Zero heap on hot path, i64 fixed-point, no floats

## Crate Layout

```
rsx-types/      Price, Qty, Side, SymbolConfig, shared newtypes
rsx-book/       shared orderbook (PriceLevel, OrderSlot, Slab, CompressionMap)
rsx-matching/   ME tile logic (one instance per symbol)
rsx-risk/       Risk tile logic (one per user shard)
rsx-dxs/        WAL writer/reader, DxsConsumer, DxsReplay server
rsx-gateway/    Gateway tile, WS ingress + CMP/UDP to risk
rsx-marketdata/ Marketdata tile, shadow book, L2/BBO/trades
rsx-mark/       Mark price aggregator (separate process)
rsx-recorder/   Archival DXS consumer (separate process)
rsx-cli/        WAL dump/inspect tool (clap CLI)
rsx-maker/      Market maker bot (separate process)
```

Each process is a separate binary. Crates are libraries
linked into their respective process binaries.

## Implementation Philosophy

- Minimal implementation -- do the simplest thing that works
- Simple names, not abbreviated: `position` not `pos`, but not
  `user_position_state_container` either
- Do things simply, not intertwinedly -- each module does one thing
- Use traits where applicable for testability (mock boundaries)
- Flat file hierarchies: one level of modules, avoid deep nesting
- Copy standard macros from `../trader` where applicable
- Tracing + structured logs, NOT Prometheus -- dump metrics as
  structured log lines, a separate reader ships them elsewhere

## Patterns from funding-bot/trader

- Crate-per-concern, flat modules (no nested mod dirs)
- Re-export key types from lib.rs
- Tests in `tests/` dir with `_test.rs` suffix, not inline
- `_utils.rs` for stateless helpers only

## Rust Patterns

- Single import per line (`use tracing::info;` not
  `use tracing::{info, debug};`) -- cleaner git diffs
- `#[repr(C, align(64))]` on all hot-path structs (cache line)
- Fixed-point i64 for all prices/quantities -- NEVER float
- `Price(pub i64)`, `Qty(pub i64)` newtypes (`#[repr(transparent)]`)
- Slab arena allocator for fixed-size objects (orders, levels)
- Zero heap allocation on hot path (pre-allocate everything)
- Explicit enum states, not implicit flags
- FxHashMap for integer-keyed maps (not std HashMap)
- SPSC rings via rtrb for intra-process IPC
- Pin hot threads to cores via core_affinity
- Panic handler: `install_panic_handler()` from rsx_types
- Document lock acquisition order where locks exist

## Documentation
- NEVER use "rollout" as a heading or section name

## Naming

- `seq` not `seq_no`, `ts_ns` not `timestamp_nanoseconds`
- `px` for price, `qty` for quantity in wire/WAL contexts
- `bid_px`, `ask_px`, `bid_qty`, `ask_qty` for BBO fields
- `symbol_id: u32`, `user_id: u32` -- always u32, never string
- `_utils.rs` suffix for utility modules
- `_pad` prefix for padding fields in repr(C) structs

## Build & Dev

- `cargo check` first, always (fastest feedback, no codegen)
- Single test: `cargo test -p rsx-book -- test_name`
- Single test file: `cargo test -p rsx-dxs --test wal_test`
- Debug builds default (~3x faster compile than release)
- 80 char line width, max 120
- `make test`: unit tests <5s, every commit
- `make e2e`: component tests ~30s, every PR
- `make integration`: testcontainers, 1-5min
- `make wal`: WAL correctness <10s
- `make smoke`: deployed system <1min
- `make perf`: Criterion benchmarks, nightly
- Config via env vars only (no TOML args)
- Entrypoint always called `main`

## Testing

- Tests in dedicated files, separate from code:
  `src/margin.rs` -> `tests/margin_test.rs` (not inline #[cfg(test)])
- E2E tests: real component + mocked deps, `tests/` dir
- Integration: testcontainers-rs (Postgres), `tests/` dir
- `--test-threads=1` if global state via DashMap/RwLock
- Centralize test setup in `tests/common/mod.rs`
- Testcontainers: dynamic port via `.get_host_port_ipv4()`
- Criterion for benchmarks with regression detection (>10% = fail)
- Property tests: proptest for order sequence invariants (future)

## Fixed-Point Arithmetic

All values are i64 in smallest units. Conversion at API boundary only.

```
price_raw = (human_price / tick_size) as i64
qty_raw   = (human_qty / lot_size) as i64
```

Overflow: check at order entry, not on hot path. Use checked_mul
for notional = price * qty at risk boundary.

## WAL / DXS

- Fixed-record format: 16B header + `#[repr(C, align(64))]` payload
- WAL disk format = wire format = DXS stream format (no transformation)
- WalWriter flush every 10ms, rotate at 64MB, retain 10min
- Backpressure: buffer full or flush lag > 10ms -> stall producer
- Tip persistence: every 10ms, idempotent replay from tip+1

## Networking Stack

- **Gateway + Market Data:** monoio with io_uring. These are
  on the critical path (<50us end-to-end GW→ME→GW). Every
  epoll syscall adds latency. io_uring batches submissions
  in shared kernel/userspace rings -- fewer syscalls, lower
  tail latency. Each tile is a dedicated pinned thread with
  one SPSC downqueue (orders in) and one SPSC upqueue
  (fills out). The I/O multiplexing inside the tile is the
  only part that touches the network stack.
- **Later:** userspace networking (DPDK, AF_XDP) swaps the
  I/O layer inside the same tile. No changes to SPSC rings
  or ME.
- **DXS:** CMP/UDP for hot path, WAL replication over TCP
  for cold path. Same wire format as disk. See CMP.md.
- Reference impl: `/home/onvos/app/trader/monoio-client/`
  - `ws_monoio.rs`: WebSocket client/server on monoio
  - `web_client.rs`: HTTP client with monoio
  - Proven in production (funding-bot, trader)

## SPSC Ring Patterns

- rtrb for same-process IPC (~50-170ns latency)
- `push_spin()`: bare busy-spin, no `spin_loop()`, dedicated core
- Per-consumer rings (slow mktdata doesn't stall risk)
- Ring full = producer stalls (matching engine waits)

## Component Spec Cross-References

| Component | Spec | Test Spec |
|-----------|------|-----------|
| Architecture | TILES.md (networking, tiles) | - |
| Shared orderbook | ORDERBOOK.md | TESTING-BOOK.md |
| Matching engine | ORDERBOOK.md, CONSISTENCY.md | TESTING-MATCHING.md |
| DXS (WAL + replay) | DXS.md, WAL.md, CMP.md | TESTING-DXS.md |
| Risk engine | RISK.md | TESTING-RISK.md |
| Liquidator | LIQUIDATOR.md | TESTING-LIQUIDATOR.md |
| Mark price | MARK.md | TESTING-MARK.md |
| Gateway | NETWORK.md, WEBPROTO.md, RPC.md, MESSAGES.md | TESTING-GATEWAY.md |
| Market data | MARKETDATA.md | TESTING-MARKETDATA.md |
| SPSC rings | notes/SMRB.md | TESTING-SMRB.md |
| Validation edge cases | VALIDATION-EDGE-CASES.md | (cross-references all) |

## Correctness Invariants (system-wide)

1. Fills precede ORDER_DONE (per order)
2. Exactly-one completion per order (ORDER_DONE xor ORDER_FAILED)
3. FIFO within price level (time priority)
4. Position = sum of fills (risk engine)
5. Tips monotonic, never decrease
6. Best bid < best ask (no crossed book)
7. SPSC preserves event FIFO order
8. Slab no-leak: allocated = free + active
9. Funding zero-sum across all users per symbol per interval
10. Advisory lock exclusive: at most one main per shard
