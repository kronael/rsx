# PROCESSES: Process + Tile Composition

## Scope

This document defines **processes** (monolithic binaries),
**tiles** (pinned thread loops within a process), and
**links** (inter-process CMP/UDP or WAL/TCP connections).
It provides a generic template and then instantiates it for
current RSX components.

## Terms

- **Process**: one OS process / binary (Gateway, Risk, etc.).
- **Tile**: pinned thread loop inside a process.
- **Link**: inter-process transport (CMP/UDP hot path, WAL/TCP cold path).

## Generic Process Template

Every process is composed from a set of tiles. Not every
process uses every tile, but the roles are standardized.

**Core tiles (hot path):**
- **Main/Core Tile**: CPU-pinned busy-spin loop. All critical
  decision logic lives here. No blocking syscalls.
- **Network Tile**: monoio event loop for hot-path I/O.

**Auxiliary tiles (cold path):**
- **WAL Writer Tile**: append + fsync every 10ms, rotate.
- **Replay/Streamer Tile**: TCP/WAL streaming to consumers.
- **Persistence Tile**: tokio DB write-behind, batched.
- **Telemetry Tile**: collects logs/metrics/heartbeats and
  appends to local telemetry log for external polling.

**Runtime selection:**
- Hot path tiles: **monoio** where I/O is needed; busy-spin
  for pure compute.
- Auxiliary tiles: **tokio** acceptable for integration
  breadth.
- Some auxiliary processes may be **tokio-only** end-to-end
  (no monoio), while still reusing common CMP/WAL/telemetry
  components.

## Inter-Process Links

- **CMP/UDP (hot path)**: one WAL record per datagram.
- **WAL/TCP (cold path)**: replay/streaming of WAL records.

All CMP **data** payloads begin with the CMP prefix (see
CMP.md). Control messages do not carry the prefix.

## Telemetry

- Each process has a **Telemetry Tile** that accepts structured
  events and heartbeats via SPSC.
- Telemetry tile appends to an on-disk telemetry log.
- A separate telemetry service may poll/ship those logs.

## Process Instantiations

### Gateway Process

Tiles:
- Network Tile (monoio): WS ingress/egress, auth, rate limit.
- Core Tile (busy-spin): order validation, routing.
- Telemetry Tile (tokio): logs + heartbeats.

Links:
- CMP/UDP to Risk (orders, responses).

### Risk Process

Tiles:
- Core Tile (busy-spin): margin checks, position updates.
- Network Tile (monoio or raw UDP): CMP ingress/egress.
- Persistence Tile (tokio): Postgres write-behind.
- Telemetry Tile (tokio).

Links:
- CMP/UDP from Gateway (orders).
- CMP/UDP to Matching (validated orders).
- CMP/UDP from Matching (fills/dones).

### Matching Process

Tiles:
- Core Tile (busy-spin): orderbook + matching.
- WAL Writer Tile (busy-spin): append + 10ms fsync.
- Replay/Streamer Tile (tokio): WAL/TCP DXS streaming.
- Telemetry Tile (tokio).

Links:
- CMP/UDP from Risk (orders).
- CMP/UDP to Risk (fills/dones).
- WAL/TCP to Recorder/Marketdata/Mark (replay).

### Marketdata Process

Tiles:
- Network Tile (monoio): WS pub/sub.
- Core Tile (busy-spin): shadow book + fanout.
- Telemetry Tile (tokio).

Links:
- CMP/UDP or WAL/TCP from Matching (events).

### Mark Process

Tiles:
- Network Tile (monoio): external price feeds.
- Core Tile (busy-spin): mark aggregation.
- Telemetry Tile (tokio).

Links:
- CMP/UDP to Risk or Matching (mark updates).

### Recorder Process

Tiles:
- Replay/Consumer Tile (tokio): WAL/TCP replay.
- Telemetry Tile (tokio).

Links:
- WAL/TCP from Matching (archive stream).

### Archive Process

Tiles:
- Replay Tile (tokio): WAL/TCP server for archived WAL.
- Telemetry Tile (tokio).

Links:
- WAL/TCP to consumers (replay).

## Busy-Spin Guidance

- Core tiles are busy-spin and pinned to dedicated cores.
- Network tiles yield in their runtime; no busy-spin.
- Auxiliary tiles are async (tokio) and can block on I/O.

## Heartbeats

- Each tile emits a heartbeat into Telemetry Tile at a
  fixed interval (default 1s).
- Heartbeat includes tile name, process name, and last
  progress timestamp.
