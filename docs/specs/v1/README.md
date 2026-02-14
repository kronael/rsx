# Specifications (v1)

Complete technical specifications for all RSX components.

## Entry Point

Start with [ARCHITECTURE.md](ARCHITECTURE.md) for an overview of the entire system.

## Core Architecture

- **[ARCHITECTURE.md](ARCHITECTURE.md)** - System architecture overview
- **[NETWORK.md](NETWORK.md)** - Tiles, networking, SPSC rings
- **[CONSISTENCY.md](CONSISTENCY.md)** - Event ordering and guarantees

## Components

### Trading Engine

- **[ORDERBOOK.md](ORDERBOOK.md)** - Orderbook structures, compression, matching
- **[MATCHING.md](MATCHING.md)** - Matching engine behavior
- **[RISK.md](RISK.md)** - Risk engine, margin, positions, funding
- **[LIQUIDATOR.md](LIQUIDATOR.md)** - Liquidation rounds, insurance fund

### Gateways

- **[GATEWAY.md](GATEWAY.md)** - WebSocket gateway, auth, rate limiting
- **[MARKETDATA.md](MARKETDATA.md)** - Market data shadow book, L2/BBO/trades
- **[MARK.md](MARK.md)** - Mark price aggregator, external feeds

### Transport & Storage

- **[DXS.md](DXS.md)** - Write-ahead log, replay server, consumers
- **[CMP.md](CMP.md)** - C Message Protocol, flow control, NACK
- **[DATABASE.md](DATABASE.md)** - PostgreSQL schema, write-behind

## Testing Specifications

Each component has a corresponding testing spec in this directory.

See individual TESTING-*.md files for component-specific test cases.
