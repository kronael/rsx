# Crate Documentation

RSX is organized into 9 crates, each with a specific responsibility.

## Core Libraries

### rsx-types
Shared newtypes, macros, and utilities used across all crates.

- `Price(i64)`, `Qty(i64)` - fixed-point newtypes
- `Side`, `TimeInForce`, `OrderType` - enums
- `SymbolConfig` - trading pair configuration
- Time utilities with nanosecond precision
- Panic handler

**Location:** `rsx-types/`

### rsx-book
Orderbook implementation with slab arena and compression map.

- `Slab` - arena allocator for orders
- `CompressionMap` - price level compression
- `PriceLevel` - FIFO queue at each price
- `OrderSlot` - order storage
- Snapshot save/load

**Location:** `rsx-book/`
**Documentation:** [Architecture](rsx-book.md)

### rsx-dxs
WAL writer/reader, CMP sender/receiver, DXS replay server.

- `WalWriter` - write-ahead log with rotation
- `WalReader` - sequential WAL replay
- `CmpSender` / `CmpReceiver` - UDP message transport
- `DxsReplay` - TCP stream server for consumers
- Flow control, NACK retransmission

**Location:** `rsx-dxs/`
**Specifications:** [DXS.md](../specs/v1/DXS.md), [CMP.md](../specs/v1/CMP.md)

## Process Binaries

### rsx-matching
Matching engine binary (1 per symbol, single-threaded).

- Order matching with price-time priority
- GTC, IOC, FOK time-in-force
- Post-only and reduce-only flags
- BBO updates, CONFIG_APPLIED messages
- Order deduplication

**Location:** `rsx-matching/`
**Documentation:** [Architecture](rsx-matching.md)
**Specifications:** [ORDERBOOK.md](../specs/v1/ORDERBOOK.md), [MATCHING.md](../specs/v1/MATCHING.md)

### rsx-risk
Risk engine binary (1 per user shard).

- Pre-trade margin checks
- Position tracking
- Funding payments
- Liquidation triggering
- Insurance fund management
- Replication/failover (future)

**Location:** `rsx-risk/`
**Specifications:** [RISK.md](../specs/v1/RISK.md), [LIQUIDATOR.md](../specs/v1/LIQUIDATOR.md)

### rsx-gateway
Gateway binary (WebSocket overlay + CMP bridge).

- WebSocket server (monoio + io_uring)
- JWT authentication
- Per-user rate limiting
- Circuit breaker
- CMP/UDP to risk engine
- Fill broadcast to users

**Location:** `rsx-gateway/`
**Specifications:** [GATEWAY.md](../specs/v1/GATEWAY.md), [NETWORK.md](../specs/v1/NETWORK.md)

### rsx-marketdata
Market data binary (shadow orderbook + public broadcast).

- Shadow orderbook from ME WAL
- L2 depth updates
- BBO (best bid/offer) updates
- Trade updates
- Sequence gap detection
- Public WebSocket broadcast

**Location:** `rsx-marketdata/`
**Specifications:** [MARKETDATA.md](../specs/v1/MARKETDATA.md)

### rsx-mark
Mark price aggregator binary.

- Binance and Coinbase feed integration
- Median price calculation
- Staleness detection
- CMP broadcast to risk engine
- Fallback handling

**Location:** `rsx-mark/`
**Specifications:** [MARK.md](../specs/v1/MARK.md)

### rsx-recorder
Archival DXS consumer binary.

- Connects to ME, Risk, Gateway DXS streams
- Daily WAL file rotation
- Compressed archival storage
- Replay for compliance/debugging

**Location:** `rsx-recorder/`
**Specifications:** [DXS.md](../specs/v1/DXS.md)

## CLI Tools

### rsx-cli
Command-line tools for WAL inspection and debugging.

- `wal dump` - display WAL records
- `wal replay` - replay WAL to stdout
- `wal stats` - WAL file statistics
- Future: compression, export, validation

**Location:** `rsx-cli/`
**Documentation:** [Architecture](rsx-cli.md)

## Dependency Graph

```
rsx-types (base)
  ├─ rsx-book
  │   ├─ rsx-matching
  │   └─ rsx-marketdata
  ├─ rsx-dxs
  │   ├─ rsx-matching
  │   ├─ rsx-risk
  │   ├─ rsx-gateway
  │   ├─ rsx-marketdata
  │   ├─ rsx-mark
  │   ├─ rsx-recorder
  │   └─ rsx-cli
  └─ rsx-risk
```

## Testing

Each crate has its own test suite:

- **Unit tests:** `tests/*_test.rs` files
- **Integration tests:** `tests/` directory
- **Benchmarks:** `benches/` directory (Criterion)

See [Testing Specifications](../specs/v1/TESTING-BOOK.md) for details.

## Build Commands

```bash
# Single crate
cargo check -p rsx-book
cargo test -p rsx-book
cargo build -p rsx-matching

# All crates
cargo check --workspace
cargo test --workspace
cargo build --workspace --release
```

## Per-Crate Documentation

- [rsx-matching](rsx-matching.md) - Matching engine architecture
- [rsx-cli](rsx-cli.md) - CLI tool architecture
