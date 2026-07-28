# RSX in one page

> **Work in progress.** RSX is a pre-v1 portfolio-margin derivatives
> exchange. `rsx-cast` and `rsx-book` are finished; the full exchange is not.

## Scope

RSX implements perpetual-futures order entry, portfolio risk, matching,
market-data fan-out, WAL-backed recovery, and a terminal client. Options are not
implemented.

The system is split into gateway, risk, matching, marketdata, mark, and recorder
processes. Risk shards by user. Matching shards by symbol.

## Finished components

- **`rsx-cast`** — domain-agnostic fixed-record transport. The WAL record, UDP
  frame, and TCP replay record share one representation.
- **`rsx-book`** — fixed-point order book and deterministic matching core. It
  uses preallocated order storage, direct cancel by handle, compressed price
  slots, and occupancy summaries.

`rsx-matching` is a release candidate. The status of every other component is
listed in [PROGRESS.md](PROGRESS.md).

## Data path

```text
client → Gateway → Risk[user] → Matching[symbol]
                                  │
                   client ← Gateway ← Risk
                                  │
                                  ├── Marketdata
                                  └── Recorder
```

The live path uses casting/UDP. Recovery and archival use WAL replication over
TCP. Prices and quantities are fixed-point integers.

## Evidence

- [reports/](reports/) contains dated benchmark runs with their workloads and
  limitations.
- [docs/benches.md](docs/benches.md) maps each benchmark to the production path
  it measures.
- [GUARANTEES.md](GUARANTEES.md) defines ordering, durability, and loss bounds.
- [BUGS.md](BUGS.md) and [PROGRESS.md](PROGRESS.md) list current gaps.

Published latency summaries require p50 and p99 from the same run. Targets are
kept separate from measured results.
