# TILES: Thread-per-Concern Architecture

## Overview

Every RSX process uses **tiles** -- pinned threads, one per
concern, communicating via SPSC rings (rtrb, 50-170ns)
within the process. Between processes: quinn QUIC with raw
WAL wire format. See NETWORK.md for process topology.

## Processes

Each is a separate monolithic process (see NETWORK.md):

- **Gateway** -- WS/QUIC ingress, auth, rate limit
- **Risk Engine** -- margin, positions, liquidation
- **Matching Engine** -- one per symbol, orderbook
- **Marketdata** -- shadow book, L2/BBO/trades fan-out
- **Recorder** -- daily WAL archival (DXS consumer)
- **Mark** -- external price aggregator

Between processes: quinn QUIC with raw WAL wire format (one
multiplexed stream per link). Latency: 10-100us.

## Tile Pattern (within each process)

Each process internally runs pinned threads (tiles) for
its own concerns, connected by SPSC rings (rtrb):

```
Example: Matching Engine process
+===============================================+
|  +-------+  SPSC  +---------+  SPSC  +------+ |
|  |  Net  |------->| Matching|------->| WAL  | |
|  | tile  |<-------| tile    |------->|Writer| |
|  |(monoio|  fills |         | events | tile | |
|  +-------+        +---------+        +--+---+ |
|                        |           +----v----+ |
|                        | SPSC     |DxsReplay | |
|                        v          |  tile    | |
|                   +---------+     +---------+  |
|                   |MARKETDATA|                  |
|                   | tile     |                  |
|                   +---------+                  |
+===============================================+
         QUIC ↕              gRPC ↕
     Risk Engine          Recorder, Mark
```

**Within process:** SPSC rings (rtrb). Same address space,
pinned threads, zero syscall overhead, 50-170ns per hop.

**Between processes:** quinn QUIC with raw WAL wire format.
DXS streams WAL records to external consumers.

## Networking Stack

### monoio with io_uring

Gateway and Market Data tiles use **monoio** (io_uring) for
all client-facing network I/O:

- WebSocket accept/read/write (gateway, market data)
- HTTP client (mark price aggregator)
- Zero-copy via kernel submission queues
- Lower latency than tokio (epoll)

### Why not tokio everywhere?

tokio is epoll-based. Each I/O operation is a syscall. For a
gateway handling 100K+ connections, that's too many syscalls.
io_uring batches submissions and completions in shared
kernel/userspace rings.

DxsReplay uses tonic gRPC -- it's a cold path (external
consumers, not hot-path matching). Raw fixed records minimize
serialization overhead.

### Reference Implementation

`/home/onvos/app/trader/monoio-client/`:
- `ws_monoio.rs`: WebSocket client/server on monoio
- `web_client.rs`: HTTP client with monoio
- Production-proven in funding-bot and trader

## Tiles Within rsx-engine

### Gateway Tile (monoio, thread pool)

Accepts client WebSocket and QUIC connections. Authenticates,
rate-limits, validates basic fields. Pushes commands to Risk
tile via SPSC ring. Receives fills/dones from Risk via SPSC
ring, sends back to client.

### Risk Tile (CPU-pinned, per user shard)

Pre-trade: checks portfolio margin across all symbols for the
user. Post-trade: applies fills, recalculates margin, triggers
liquidation if equity < maintenance margin. Sits **between**
gateway and ME -- all orders pass through risk first.

Communicates:
- Gateway → Risk: SPSC (orders in)
- Risk → ME: SPSC (validated orders)
- ME → Risk: SPSC (fills, dones)
- Risk → Gateway: SPSC (fills, dones back to user)
- Risk → Postgres: SPSC write-behind (positions, accounts)

### Matching Engine Tile (CPU-pinned, per symbol)

Pure computation. No network I/O. Reads validated orders from
SPSC ring (from Risk). Matches against book. Drains events to
WAL Writer tile via SPSC ring.

Single-threaded, bare busy-spin, dedicated core.

### WAL Writer Tile (CPU-pinned)

Reads events from SPSC ring (from ME). Appends to in-memory
buffer. Flushes to disk with fsync every 10ms. Rotates at
64MB. Notifies DxsReplay tile on flush via Arc<Notify>.

No network I/O.

### DxsReplay Tile (tonic/tokio)

QUIC server (quinn). Reads WAL files, streams raw fixed
records to external consumers (recorder, mark aggregator,
or risk during recovery replay). Multiple consumers get
independent streams.

gRPC is acceptable for DXS: this is a cold path (external
consumers, not hot-path matching). The interface is
"stream records from seq N" and raw fixed records minimize
serialization overhead.

This is the **only tile that talks to external processes**.

### Market Data Tile (monoio WebSocket server)

Maintains shadow orderbook from ME events (via SPSC from WAL
writer or directly from ME). Computes L2/BBO/trades.
Broadcasts to subscriber WebSocket connections.

### Postgres Write-Behind Tile

Receives position/account updates from Risk via SPSC. Batches
writes to Postgres every 10ms (COPY for fills, UPSERT for
positions). sync_commit=on.

## External Processes

### rsx-recorder (separate binary)

Connects to rsx-engine's DxsReplay gRPC endpoint (tonic).
Writes daily archive files. Same WAL format, infinite
retention. Runs on same or different host.

### rsx-mark (separate binary)

Fetches mark prices from external exchanges (Binance, etc.)
via monoio HTTP client. Publishes to rsx-engine via QUIC.
Runs on same or different host.

## SPSC Ring Topology (within rsx-engine)

```
Gateway --[orders]--> Risk --[validated]--> ME
Gateway <--[fills]--- Risk <--[fills]------ ME
                      Risk --[positions]--> PG write-behind
                                     ME --[events]--> WAL Writer
                           WAL Writer --[notify]--> DxsReplay
                                     ME --[events]--> Marketdata
```

Each arrow is a dedicated rtrb SPSC ring. Per-consumer rings:
slow market data broadcast doesn't stall risk. Ring full =
producer stalls (backpressure).

## Future: Userspace Networking

Replace monoio (kernel io_uring) with userspace networking:
- DPDK or AF_XDP: NIC → userspace ring buffer, no kernel
- Sub-microsecond latency for gateway
- Same tile architecture, swap the I/O layer
- No changes to ME (still reads from SPSC ring)

## Performance Targets

| Path | Latency | Notes |
|------|---------|-------|
| SPSC hop | 50-170ns | intra-process, same cache |
| ME match | 100-500ns | per order |
| Risk pre-trade | <5us | margin check |
| Risk post-trade | <1us | apply fill |
| End-to-end (GW→ME→GW) | <50us | same machine |
| DxsReplay stream | 10-100us | gRPC, external |
| Gateway WS | <50us | client-facing |
