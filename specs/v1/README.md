# RSX Specifications (v1)

## Core Architecture

| File | Description |
|------|-------------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | System architecture overview and component diagram |
| [NETWORK.md](NETWORK.md) | System topology, process layout, component communication |
| [TILES.md](TILES.md) | Tile architecture: pinned threads, SPSC rings, core affinity |
| [DEPLOY.md](DEPLOY.md) | Deployment topology and operational procedures |
| [PROCESS.md](PROCESS.md) | Process lifecycle, startup, shutdown, crash recovery |
| [METADATA.md](METADATA.md) | Symbol metadata, configuration, and admin operations |

## Orderbook and Matching

| File | Description |
|------|-------------|
| [ORDERBOOK.md](ORDERBOOK.md) | Orderbook data structures, slab allocator, compression map |
| [MATCHING.md](MATCHING.md) | Matching engine tile logic (stub, see ORDERBOOK.md) |
| [CONSISTENCY.md](CONSISTENCY.md) | Event fan-out, ordering guarantees, exactly-once delivery |

## Risk and Liquidation

| File | Description |
|------|-------------|
| [RISK.md](RISK.md) | Risk engine: margin, positions, funding, shard architecture |
| [LIQUIDATOR.md](LIQUIDATOR.md) | Liquidation engine: backoff, slippage, order generation |
| [POSITION-EDGE-CASES.md](POSITION-EDGE-CASES.md) | Position tracking edge cases and boundary conditions |

## Messaging and Persistence

| File | Description |
|------|-------------|
| [MESSAGES.md](MESSAGES.md) | CMP/WAL wire format, message type definitions |
| [CMP.md](CMP.md) | C Message Protocol: UDP transport, NACK, flow control |
| [WAL.md](WAL.md) | WAL design: flush, backpressure, bounded loss window |
| [DXS.md](DXS.md) | WAL writer/reader, replay server, consumer, file format |
| [ARCHIVE.md](ARCHIVE.md) | WAL archival and cold storage |

## Gateway and Market Data

| File | Description |
|------|-------------|
| [GATEWAY.md](GATEWAY.md) | Gateway tile: WS ingress, CMP/UDP to risk (stub) |
| [MANAGEMENT-DASHBOARD.md](MANAGEMENT-DASHBOARD.md) | Parent spec for split management dashboards |
| [DASHBOARD.md](DASHBOARD.md) | Support dashboard: user balances, positions, trading state, controlled user actions |
| [RISK-DASHBOARD.md](RISK-DASHBOARD.md) | Risk/ops dashboard: exchange risk posture, symbol controls, risk parameter operations |
| [HEALTH-DASHBOARD.md](HEALTH-DASHBOARD.md) | Systems ops dashboard: load, CPU/memory/disk/network, service health and alerts |
| [PLAYGROUND-DASHBOARD.md](PLAYGROUND-DASHBOARD.md) | Dev/test dashboard: scenario launch, fault injection, observe/act/verify workflows |
| [WEBPROTO.md](WEBPROTO.md) | WebSocket protocol: frames, auth, subscriptions |
| [RPC.md](RPC.md) | RPC message definitions, request/response schemas |
| [MARKETDATA.md](MARKETDATA.md) | Market data tile: shadow book, L2/BBO/trades (stub) |
| [MARK.md](MARK.md) | Mark price aggregator: sources, EWMA, fallback chain |
| [DATABASE.md](DATABASE.md) | Postgres schema, write-behind, connection pooling |

## Validation

| File | Description |
|------|-------------|
| [VALIDATION-EDGE-CASES.md](VALIDATION-EDGE-CASES.md) | Edge cases across all validation layers |

## Testing

| File | Description |
|------|-------------|
| [TESTING.md](TESTING.md) | Testing strategy: unit, e2e, integration, perf |
| [TESTING-BOOK.md](TESTING-BOOK.md) | Orderbook test spec |
| [TESTING-CMP.md](TESTING-CMP.md) | CMP transport test spec |
| [TESTING-DXS.md](TESTING-DXS.md) | WAL/DXS test spec |
| [TESTING-GATEWAY.md](TESTING-GATEWAY.md) | Gateway test spec |
| [TESTING-LIQUIDATOR.md](TESTING-LIQUIDATOR.md) | Liquidator test spec |
| [TESTING-MARK.md](TESTING-MARK.md) | Mark price test spec |
| [TESTING-MARKETDATA.md](TESTING-MARKETDATA.md) | Market data test spec |
| [TESTING-MATCHING.md](TESTING-MATCHING.md) | Matching engine test spec |
| [TESTING-RISK.md](TESTING-RISK.md) | Risk engine test spec |
| [TESTING-SMRB.md](TESTING-SMRB.md) | SPSC ring buffer test spec |
