# RSX Exchange

Spec-first perpetuals exchange. All specs in `specs/v1/`.
No implementation code yet -- this file guides future impl.

## Crate Layout

```
crates/
  rsx-book/       shared orderbook (PriceLevel, OrderSlot, Slab, CompressionMap)
  rsx-matching/   matching engine binary (one per symbol)
  rsx-risk/       risk engine binary (one per user shard)
  rsx-dxs/        WAL writer/reader, DxsConsumer, DxsReplay server
  rsx-mark/       mark price aggregator (standalone service)
  rsx-gateway/    WS overlay + gRPC passthrough
  rsx-marketdata/ market data fan-out (shadow book, L2/BBO/trades)
  rsx-recorder/   archival consumer (daily WAL files)
  rsx-types/      Price, Qty, Side, SymbolConfig, shared newtypes
```

## Rust Patterns

- Single import per line (`use tracing::info;` not `use tracing::{info, debug};`)
- `#[repr(C, align(64))]` on all hot-path structs (cache line alignment)
- Fixed-point i64 for all prices/quantities -- NEVER float
- `Price(pub i64)`, `Qty(pub i64)` newtypes (`#[repr(transparent)]`)
- Slab arena allocator for fixed-size objects (orders, levels)
- Zero heap allocation on hot path (pre-allocate everything)
- Explicit enum states, not implicit flags
- FxHashMap for integer-keyed maps (not std HashMap)
- SPSC rings via rtrb for intra-process IPC
- Pin hot threads to cores via core_affinity
- Panic handler: `std::panic::set_hook(Box::new(|_| std::process::exit(0)));`
- Document lock acquisition order where locks exist

## Naming

- `seq` not `seq_no`, `ts_ns` not `timestamp_nanoseconds`
- `px` for price, `qty` for quantity in wire/WAL contexts
- `bid_px`, `ask_px`, `bid_qty`, `ask_qty` for BBO fields
- `symbol_id: u32`, `user_id: u32` -- always u32, never string
- `_utils.rs` suffix for utility modules
- `_pad` prefix for padding fields in repr(C) structs

## Build & Dev

- `cargo check` first (fastest feedback, no codegen)
- Debug builds default (~3x faster compile than release)
- 80 char line width, max 120
- `make test`: unit tests <5s, every commit
- `make e2e`: component tests ~30s, every PR
- `make integration`: testcontainers, 1-5min
- `make wal`: WAL correctness <10s
- `make smoke`: deployed system <1min
- `make perf`: Criterion benchmarks, nightly
- TOML config as first CLI param, API keys as second
- Entrypoint always called `main`

## Testing

- Unit tests: `#[cfg(test)]` in same file, no I/O, <5s total
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

## SPSC Ring Patterns

- rtrb for same-process IPC (~50-170ns latency)
- `push_spin()`: bare busy-spin, no `spin_loop()`, dedicated core
- Per-consumer rings (slow mktdata doesn't stall risk)
- Ring full = producer stalls (matching engine waits)

## Component Spec Cross-References

| Component | Spec | Test Spec |
|-----------|------|-----------|
| Shared orderbook | ORDERBOOK.md | TESTING-BOOK.md |
| Matching engine | ORDERBOOK.md, CONSISTENCY.md | TESTING-MATCHING.md |
| DXS (WAL + replay) | DXS.md, WAL.md | TESTING-DXS.md |
| Risk engine | RISK.md | TESTING-RISK.md |
| Liquidator | LIQUIDATOR.md | TESTING-LIQUIDATOR.md |
| Mark price | MARK.md | TESTING-MARK.md |
| Gateway | NETWORK.md, WEBPROTO.md, RPC.md, GRPC.md | TESTING-GATEWAY.md |
| Market data | MARKETDATA.md | TESTING-MARKETDATA.md |
| SPSC rings | notes/SMRB.md | TESTING-SMRB.md |

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
