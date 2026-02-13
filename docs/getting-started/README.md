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

Perpetuals exchange with fixed-point arithmetic,
single-threaded matching per symbol, CMP/UDP between
processes, and WAL-based recovery.

Built with [Claude](https://claude.ai) and
[Codex](https://openai.com/codex), orchestrated by
[ship skill](https://github.com/anthropics/claude-code) workflow.

## Status

Production-ready. 9 crates, 960 tests passing (all non-flaky),
~34k LOC Rust + 19k LOC tests. Full order pipeline wired
end-to-end: Gateway -> Risk -> ME -> Risk -> Gateway.
Liquidation engine with insurance fund. Mark price aggregator
feeding risk via CMP. Market data shadow book with L2/BBO/trades
broadcast. Playground: 680 E2E tests (Playwright + API).

See [PROGRESS.md](PROGRESS.md) for per-crate status.

## Architecture

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

**Transports:**
- Between processes: CMP/UDP (hot), WAL/TCP (cold)
- Within process: tile threads + SPSC rings (rtrb)
- DXS: WAL streaming to consumers over TCP

See [specs/v1/ARCHITECTURE.md](specs/v1/ARCHITECTURE.md)
for full architecture. Per-component docs in
[architecture/](architecture/).

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

## Crate Layout

```
rsx-types/      shared newtypes, macros, time
rsx-book/       orderbook (Slab, CompressionMap, PriceLevel)
rsx-matching/   ME binary (per-symbol, single-threaded)
rsx-risk/       risk binary (per-shard, margin + funding + liquidation)
rsx-dxs/        WAL, CMP, DXS replay (transport library)
rsx-gateway/    gateway binary (WS + CMP bridge)
rsx-marketdata/ marketdata binary (shadow book, public WS)
rsx-mark/       mark price binary (external feeds, CMP to risk)
rsx-recorder/   recorder binary (archival DXS consumer)
```

## Build and Test

```
cargo check              # fastest feedback
cargo test --workspace   # all tests (960 passing, zero failures)
cargo bench -p rsx-dxs   # WAL/CMP benchmarks
```

## Design Principles

- **Fixed-point i64** -- deterministic, no float rounding
- **Single-threaded per symbol** -- no locks, pinned cores
- **SPSC rings** -- rtrb, 50-170ns, no broker
- **WAL-based recovery** -- 0ms fill loss, idempotent replay
- **Slab arena** -- pre-allocated, zero heap on hot path
- **WAL = wire = stream** -- no format transformation
- **CMP/UDP** -- direct inter-process, no Kafka/NATS
- **SIGTERM = crash** -- one recovery path, always exercised

## Specs

All specifications in `specs/v1/`. Entry point:
[specs/v1/ARCHITECTURE.md](specs/v1/ARCHITECTURE.md).

| Spec | Covers |
|------|--------|
| ORDERBOOK.md | Book structures, matching, compression |
| RISK.md | Margin, positions, funding, liquidation |
| LIQUIDATOR.md | Liquidation rounds, insurance fund |
| DXS.md | WAL format, replay server, consumers |
| CMP.md | C Message Protocol, flow control |
| MARK.md | External feeds, median, staleness |
| WEBPROTO.md | WS compact JSON protocol |
| CONSISTENCY.md | Event fan-out, ordering guarantees |
| TILES.md | Tile architecture, SPSC rings |

## Documentation

| Document | Purpose |
|----------|---------|
| [PROGRESS.md](PROGRESS.md) | Per-crate implementation status |
| [GUARANTEES.md](GUARANTEES.md) | Consistency, durability, recovery |
| [CRITIQUE.md](CRITIQUE.md) | Current spec-vs-impl gaps |
| [CRASH-SCENARIOS.md](CRASH-SCENARIOS.md) | Failure scenarios |
| [RECOVERY-RUNBOOK.md](RECOVERY-RUNBOOK.md) | Ops recovery procedures |
| [architecture/](architecture/) | Per-component architecture |
