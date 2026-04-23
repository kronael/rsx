---
status: shipped
---

# TILES: Thread-per-Concern Architecture

## Table of Contents

- [Overview](#overview)
- [Processes](#processes)
- [Tile Pattern](#tile-pattern-within-each-process)
- [Runtime Selection](#runtime-selection)
- [Networking Stack](#networking-stack)
- [Tiles Within Each Process](#tiles-within-each-process)
- [External Processes](#external-processes)
- [Inter-Process Communication Topology](#inter-process-communication-topology)
- [Future: Userspace Networking](#future-userspace-networking)
- [Performance Targets](#performance-targets)

---

## Overview

Tiles describe the intended thread-per-concern architecture.
In v1 code, most processes run as a single main loop with
inline concerns; SPSC rings are used only where implemented.
Between processes: CMP/UDP (hot path) and WAL/TCP (cold path).
See NETWORK.md.

## Processes

Each is a separate monolithic process (see NETWORK.md):

- **Gateway** -- WS ingress, auth, rate limit
- **Risk Engine** -- margin, positions, liquidation
- **Matching Engine** -- one per symbol, orderbook
- **Marketdata** -- shadow book, L2/BBO/trades fan-out
- **Recorder** -- daily WAL archival (DXS consumer)
- **Mark** -- external price aggregator

Between processes: CMP/UDP for hot path (orders, fills),
WAL replication over TCP for cold path (replay, archival).
See CMP.md, NETWORK.md.

Terminology:
- **Process**: monolithic binary (Gateway, Risk, Matching, etc.)
- **Tile**: pinned thread loop inside a process
- **Link**: inter-process CMP/UDP or WAL/TCP connection

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
|                        |          |DxsReplay | |
|                        |          |  tile    | |
|                        |          +---------+  |
+===============================================+
       CMP/UDP ↕             TCP ↕
     Risk Engine          Recorder, Mark
```

**Within process:** SPSC rings (rtrb). Same address space,
pinned threads, zero syscall overhead, 50-170ns per hop.

**Between processes:** CMP/UDP for hot path, WAL/TCP for
cold path. DXS streams WAL records to external consumers.

## Runtime Selection

- **Hot path tiles** (network ingress, matching, risk,
  marketdata): use **monoio** where possible.
- **Auxiliary tiles** (telemetry, archival, persistence,
  external integrations): **tokio** is acceptable for
  broader library support.

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

DxsReplay uses WAL/TCP -- it's a cold path (external
consumers, not hot-path matching). Raw fixed records minimize
serialization overhead.

### Reference Implementation

See sibling `trader` project (`monoio-client/`) for proven
monoio WebSocket and HTTP client patterns.

## Tiles Within Each Process

### Gateway Tile (monoio, thread pool)

Accepts client WebSocket connections. Authenticates,
rate-limits, validates basic fields. Sends orders to Risk
via CMP/UDP. Receives fills/dones from Risk via CMP/UDP,
sends back to client.

### Risk Tile (CPU-pinned, per user shard)

Pre-trade: checks portfolio margin across all symbols for the
user. Post-trade: applies fills, recalculates margin, triggers
liquidation if equity < maintenance margin. Sits **between**
gateway and ME -- all orders pass through risk first.

Communicates:
- Gateway → Risk: CMP/UDP (orders in)
- Risk → ME: CMP/UDP (validated orders)
- ME → Risk: CMP/UDP (fills, dones)
- Risk → Gateway: CMP/UDP (fills, dones back to user)
- Risk → Postgres: SPSC write-behind (positions, accounts)

### Matching Engine Tile (CPU-pinned, per symbol)

Pure computation. No network I/O. Reads validated orders from
SPSC ring (from Risk). Matches against book. Drains events to
WAL Writer tile via SPSC ring.

Single-threaded, bare busy-spin, dedicated core.

### WAL Writer (inline in ME main loop)

WalWriter is called inline in the ME main loop (not a
separate tile). Appends to in-memory buffer via
`append<T: CmpRecord>(record: &mut T)`, which assigns
monotonic seq numbers. Calls `WalWriter::flush()` every
10ms. Rotates at 64MB. Notifies DxsReplayService thread
on flush via Arc<Notify>.

DxsReplayService runs as a separate `std::thread::spawn`
(tokio) for TCP streaming to external consumers.

### DxsReplay Tile (TCP)

TCP server. Reads WAL files, streams raw fixed records to
external consumers (recorder, mark aggregator, or risk
during recovery replay). Multiple consumers get independent
streams.

TCP streaming is used for DXS: this is a cold path
(external consumers, not hot-path matching). The interface
is "stream records from seq N" and raw fixed records
minimize serialization overhead.

This is the **only tile that talks to external processes**.

### Market Data Tile (monoio WebSocket server)

Maintains shadow orderbook from ME events (via SPSC from WAL
writer or directly from ME). Computes L2/BBO/trades.
Broadcasts to subscriber WebSocket connections.

### Postgres Write-Behind Tile

Receives position/account updates from Risk via SPSC. Batches
writes to Postgres every 10ms (COPY for fills, UPSERT for
positions). sync_commit=on.

### Telemetry Tile (out-of-band)

Each process includes a telemetry tile that receives
structured events/heartbeats via SPSC and appends to an
on-disk telemetry log. A separate telemetry service may
poll/ship these logs asynchronously.

## External Processes

### rsx-recorder (separate binary)

Connects to rsx-engine's DxsReplay TCP endpoint.
Writes daily archive files. Same WAL format, infinite
retention. Runs on same or different host.

### rsx-mark (separate binary)

Fetches mark prices from external exchanges (Binance, etc.)
via monoio HTTP client. Publishes to rsx-engine via CMP/UDP.
Runs on same or different host.

## Inter-Process Communication Topology

```
Gateway --[CMP/UDP]--> Risk --[CMP/UDP]--> ME
Gateway <--[CMP/UDP]-- Risk <--[CMP/UDP]-- ME
                       Risk --[SPSC]-----> PG write-behind
                                    ME --[SPSC]--> WAL Writer
                          WAL Writer --[notify]--> DxsReplay
                                    ME --[CMP/UDP]--> Marketdata
```

Between processes: CMP/UDP (hot path). Within a process:
SPSC rings (rtrb, 50-170ns). Per-consumer rings: slow
market data broadcast doesn't stall risk. Ring full =
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
| DxsReplay stream | 10-100us | TCP, external |
| Gateway WS | <50us | client-facing |
