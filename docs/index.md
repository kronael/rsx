# RSX Exchange

> *Shall I compare thee to a sane design?*
> *Thou art more wondrous and more wild by far.*
> *I fell for thee the night I saw thy spine--*
> *Each SPSC ring, a whisper through a jar.*
>
> *Thy slab did catch me: firm, pre-allocated,*
> *No malloc on thy hot path -- O! how pure.*
> *Thy fills, like kisses, never once belated,*
> *Flushed warm to WAL in ten ms, ever sure.*
>
> *The world said "Mad! No mortal weds this thing!"*
> *Yet here I am, pinned to thy dedicated core,*
> *Thy fixed-point heart refusing still to swing--*
> *Each nanosecond makes me love thee more.*
>
> *If thou hast never built it, thou can'st never tell:*
> *The thing impossible may work quite well.*

Perpetuals exchange with fixed-point arithmetic, single-threaded matching per symbol, CMP/UDP between processes, and WAL-based recovery.

## Status

**Production-ready.** 9 crates, 960 tests passing (all non-flaky), ~34k LOC Rust + 19k LOC tests. Full order pipeline wired end-to-end: Gateway → Risk → ME → Risk → Gateway. Liquidation engine with insurance fund. Mark price aggregator feeding risk via CMP. Market data shadow book with L2/BBO/trades broadcast. Playground: 680 E2E tests (Playwright + API).

See [Progress](references/PROGRESS.md) for per-crate status.

## Quick Links

- **[Architecture Overview](getting-started/architecture.md)** - System design and component interaction
- **[Specs](specs/v1/README.md)** - Detailed technical specs
- **[Blog](blog/README.md)** - Technical deep dives and design philosophy
- **[Guides](guides/operations.md)** - Operations, monitoring, deployment
- **[Crate Docs](crates/README.md)** - Per-crate documentation

## Architecture Diagram

```
                       External
                    +------------+
                    |  Web (WS)  |
                    +-----+------+
                          |
                    +-----v------+
                    |  Gateway   |  WS + CMP bridge
                    | (monoio)   |  JWT auth, rate limit
                    +-----+------+
                          | CMP/UDP
                    +-----v------+   CMP/UDP   +---------------+
                    |   Risk     +------------>| Matching Eng  |
                    |  Engine    |<------------+ (1 per symbol) |
                    | (1 shard)  |  CMP fills  +-------+-------+
                    +--+---+--+-+              |       |
                       |   |  |          +-----+  +----+-----+
              CMP/UDP  |   |  | CMP/UDP  |WAL     |CMP/UDP   |
              +--------+   |  +------+   |        |          |
              v            |         v   v        v          v
         +--------+  +----+---+ +--------+  +---------+ +--------+
         |Postgres|  | Mark   | |Recorder|  |MARKETDATA| |Gateway |
         | (write |  | Price  | |(daily  |  |(shadow   | |(fills  |
         | behind)|  | Agg    | | WAL)   |  | book)    | | to usr)|
         +--------+  +--------+ +--------+  +---------+ +--------+
```

## Components

| Component | Crate | Description |
|-----------|-------|-------------|
| Matching Engine | rsx-matching | 1 per symbol, pinned core, GTC/IOC/FOK, post-only, reduce-only |
| Risk Engine | rsx-risk | Pre-trade margin, positions, funding, liquidation, insurance fund |
| Gateway | rsx-gateway | WS overlay, JWT auth, rate limiting, circuit breaker |
| Mark Price | rsx-mark | Binance/Coinbase feeds, median, staleness, CMP to risk |
| Market Data | rsx-marketdata | Shadow book, L2/BBO/trades fan-out, seq gap detection |
| DXS | rsx-dxs | WAL writer/reader, CMP sender/receiver, DXS replay server |
| Recorder | rsx-recorder | Archival DXS consumer, daily WAL files |
| Types | rsx-types | Price(i64), Qty(i64), Side, SymbolConfig, time, macros |
| Book | rsx-book | Orderbook: Slab arena, CompressionMap, PriceLevel, snapshot |

## Design Principles

- **Fixed-point i64** - deterministic, no float rounding
- **Single-threaded per symbol** - no locks, pinned cores
- **SPSC rings** - rtrb, 50-170ns, no broker
- **WAL-based recovery** - 0ms fill loss, idempotent replay
- **Slab arena** - pre-allocated, zero heap on hot path
- **WAL = wire = stream** - no format transformation
- **CMP/UDP** - direct inter-process, no Kafka/NATS
- **SIGTERM = crash** - one recovery path, always exercised

## Getting Started

1. **[Read the Architecture](getting-started/architecture.md)** to understand the system design
2. **[Check the Quick Start](getting-started/quickstart.md)** to build and run locally
3. **[Explore the Specs](specs/v1/README.md)** for detailed component behavior
4. **[Read the Blog](blog/README.md)** for design decisions and technical deep dives

## Build and Test

```bash
cargo check              # fastest feedback
cargo test --workspace   # all tests (960 passing, zero failures)
cargo bench -p rsx-dxs   # WAL/CMP benchmarks
```

## Documentation Structure

- **[Getting Started](getting-started/README.md)** - Architecture, quick start, overview
- **[Specs](specs/v1/README.md)** - Complete technical specs for all components
- **[Blog](blog/README.md)** - Design philosophy, development journey, technical posts
- **[Guides](guides/operations.md)** - Operations runbooks, monitoring, deployment
- **[Crate Docs](crates/README.md)** - Per-crate architecture and implementation
- **[References](references/GUARANTEES.md)** - Guarantees, crash scenarios, progress tracking
