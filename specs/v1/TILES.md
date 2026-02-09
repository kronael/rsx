# TILES: Tile-based Architecture for High-Performance Networking

## Overview

RSX uses a tile-based architecture where each "tile" is a separate thread
or thread pool handling a specific concern. Critical components use fast
networking stacks with pluggable I/O backends.

## Networking Stack Requirements

### Primary Stack: monoio with io_uring

All network I/O uses **monoio** (io_uring-based async runtime) for
maximum performance on Linux:

- WebSocket connections (gateway, market data fan-out)
- HTTP REST APIs (if needed)
- gRPC can be implemented on top (custom impl, not tonic)
- QUIC for future userspace networking

### Why monoio?

- Zero-copy I/O via io_uring
- Lower latency than tokio (epoll-based)
- Direct kernel submission queue (no syscall per operation)
- Batched completions
- Proven in production (see ../trader, ../funding-bot)

### Implementation Reference

See `/home/onvos/app/trader/monoio-client/` for production patterns:
- `ws_monoio.rs`: WebSocket client/server
- `web_client.rs`: HTTP client with monoio
- `utils/monoio.rs`: Timeout and helper utilities

## Tile Breakdown

### Tile 1: Gateway WebSocket (monoio)

**Purpose:** Accept client WebSocket connections, authenticate, route
commands to matching engine via gRPC or shared memory.

**Stack:**
- monoio TcpListener
- WebSocket handshake + framing
- Thread-per-core or thread pool (8-16 threads)
- Communicates with matching engine via SPSC rings or gRPC

**Files:**
- `rsx-gateway/src/ws_server.rs` (monoio WebSocket server)
- `rsx-gateway/src/auth.rs` (session management)
- `rsx-gateway/src/router.rs` (command routing)

### Tile 2: Gateway gRPC Passthrough (monoio)

**Purpose:** Expose gRPC API for programmatic clients (alternative to WS).

**Stack:**
- Custom gRPC impl on monoio (NOT tonic/tokio)
- HTTP/2 framing over monoio TcpStream
- Same backend as WebSocket tile (SPSC rings to matching engine)

**Note:** May start with tonic for prototype, replace with monoio gRPC
when performance matters.

### Tile 3: Matching Engine (CPU-pinned, no network I/O)

**Purpose:** Order matching, book maintenance, deterministic core.

**Stack:**
- No network I/O (pure computation)
- Reads commands from SPSC ring (from gateway)
- Writes events to DXS WalWriter (SPSC ring)
- Single-threaded, CPU-pinned, zero allocation hot path

### Tile 4: DXS WAL Writer (CPU-pinned)

**Purpose:** Persist all events to WAL with fsync.

**Stack:**
- No network I/O
- Reads from SPSC ring (from matching engine, mark aggregator)
- Writes to disk with fsync
- Notifies DxsReplay server on flush (via Arc<Notify>)

### Tile 5: DXS Replay Server (monoio gRPC)

**Purpose:** Stream WAL records to consumers (risk engine, market data,
recorder).

**Stack:**
- Custom gRPC on monoio (or tonic for now)
- Reads WAL files (blocking I/O in separate thread pool)
- Streams via gRPC to consumers
- Multiple consumers get independent streams

### Tile 6: Risk Engine (per-user shard)

**Purpose:** Track positions, margin, liquidations.

**Stack:**
- DxsConsumer (gRPC client, monoio)
- CPU-bound logic (position accounting)
- Writes to own WAL via DXS (risk events)

### Tile 7: Market Data Fan-out (monoio WebSocket)

**Purpose:** Broadcast BBO/trades/L2 to subscribers.

**Stack:**
- Shadow orderbook (DxsConsumer ingests fills/inserts/cancels)
- monoio WebSocket server (broadcast to N clients)
- Thread-per-core, lock-free broadcast

### Tile 8: Mark Price Aggregator (monoio HTTP client)

**Purpose:** Fetch external mark prices, publish to DXS.

**Stack:**
- monoio HTTP client to external exchanges
- Publishes mark price updates to DXS
- Single-threaded, timed loop

## Thread Communication

All inter-tile communication via:
1. **SPSC rings** (rtrb crate) for same-process, low-latency
2. **gRPC over monoio** for cross-process or networked
3. **Shared WAL files** for persistence + replay

No locks on hot path. Producer stalls if consumer can't keep up
(backpressure).

## Future: Userspace Networking

Replace kernel network stack with userspace (DPDK, AF_XDP, or custom):
- NIC directly writes to userspace ring buffers
- Zero kernel involvement
- Sub-microsecond latency
- Requires kernel bypass, dedicated NIC queues

Architecture supports this via pluggable I/O:
- Gateway tile swaps monoio TcpListener for userspace socket
- Same WebSocket/gRPC framing logic
- No changes to matching engine (still reads from SPSC ring)

## Performance Targets

| Tile | Latency Target | Throughput Target |
|------|----------------|-------------------|
| Gateway WS | <50µs per message | >1M msgs/sec |
| Matching Engine | <1µs per order | >1M orders/sec |
| DXS WAL Writer | <1ms fsync (10ms batch) | >100K writes/sec |
| DXS Replay | <10µs per record | >500 MB/s |
| Risk Engine | <10µs per fill | >100K fills/sec |
| Market Data | <50µs broadcast | >1M msgs/sec |

## Implementation Order

1. **Phase 1 (prototype):** tokio for all network I/O (tonic gRPC)
2. **Phase 2 (production):** monoio for Gateway WS + Market Data
3. **Phase 3 (optimization):** Custom monoio gRPC (replace tonic)
4. **Phase 4 (future):** Userspace networking

Current DXS implementation uses tonic (Phase 1). Gateway and Market
Data will use monoio from the start (Phase 2).

## References

- Monoio client implementation: `/home/onvos/app/trader/monoio-client/`
- SPSC ring spec: `notes/SMRB.md`
- DXS spec: `DXS.md`, `WAL.md`
- Gateway spec: `NETWORK.md`, `WEBPROTO.md`
- Market data spec: `MARKETDATA.md`
