# RSX

> **Work in progress.** RSX is a pre-v1 portfolio-margin derivatives
> exchange. `rsx-cast` and `rsx-book` are finished. The remaining components
> are at different stages of development and are not production-ready.

RSX runs the exchange as separate gateway, risk, matching, market-data, mark,
and recorder processes. Perpetual futures are implemented. Options are not.

## Component status

| Status | Components |
|---|---|
| **Finished** | `rsx-cast`, `rsx-book` |
| **Release candidate** | `rsx-matching` |
| **Working** | `rsx-types`, `rsx-messages`, `rsx-risk`, `rsx-mark`, `rsx-recorder`, `rsx-cli`, `rsx-log`, `rsx-health` |
| **In development** | `rsx-gateway`, `rsx-marketdata`, `rsx-glass`, production deployment |

[PROGRESS.md](PROGRESS.md) records the implemented surface, test counts, and
open work. A runnable component is not necessarily finished.

## System shape

An order from user `U` for symbol `S` follows:

```text
client
  │ WebSocket
  ▼
Gateway
  │ casting/UDP
  ▼
Risk[U] ── reserve margin and validate the portfolio
  │ casting/UDP
  ▼
ME[S] ───── match against one symbol's book
  │
  ├──► Risk[U] ── settle fills ──► Gateway ──► client
  ├──► Marketdata
  └──► Recorder
```

Risk shards by user. Matching shards by symbol. These axes scale
independently. The gateway is stateless with respect to positions and margin.

The live inter-process path uses fixed-layout records over casting/UDP. WAL
replication and recovery use TCP. Threads inside a process communicate through
SPSC rings. Prices and quantities are fixed-point `i64` values; conversion to
human units happens at the API boundary.

## Components

- **`rsx-cast`** — domain-agnostic WAL, casting/UDP, and TCP replay. The same
  fixed record layout is used in memory, on disk, and on the wire.
- **`rsx-book`** — fixed-point order book and deterministic matching core with
  preallocated order storage, direct cancel by handle, compressed price slots,
  and occupancy summaries.
- **`rsx-matching`** — one matching-engine process per symbol. It owns
  transport, deduplication, WAL sequencing, recovery, and event fan-out.
- **`rsx-risk`** — portfolio-margin state, positions, funding, liquidation,
  and persistence, sharded by user.
- **`rsx-gateway`** — WebSocket authentication, validation, routing, and the
  bridge to the internal order path.
- **`rsx-marketdata`** — shadow books and public BBO, depth, and trade fan-out.
- **`rsx-mark`** — external mark-price aggregation.
- **`rsx-recorder`** — archival replication consumer.
- **`rsx-glass`** — desktop GPU trading UI (specs in `rsx-glass/specs/`).
- **`rsx-playground`** — local dashboard for starting processes, submitting
  orders, inspecting WAL, and checking invariants.

The Rust workspace also contains shared types, wire messages, logging, health,
and inspection crates. See [ARCHITECTURE.md](ARCHITECTURE.md) for ownership and
data flow.

## Quick start

Requirements: Linux, the pinned Rust toolchain, Postgres, `uv`, and `bun`.

```bash
make prepare
./rsx-playground/playground start
./rsx-playground/playground start-all
```

Open `http://localhost:49171`.

```bash
./rsx-playground/playground stop-all
./rsx-playground/playground stop
```

Use the repository gates before treating a change as releasable:

```bash
make check
make lint
make test
make gate
```

[TESTING.md](TESTING.md) explains the full test taxonomy.

## Performance evidence

Benchmarks and dated results are separate:

- [docs/benches.md](docs/benches.md) indexes the benchmark programs and the
  production path each one isolates.
- [reports/](reports/) stores dated runs, workloads, p50 and p99 values, and
  limitations.
- [PROGRESS.md](PROGRESS.md) carries only current summary evidence backed by a
  same-run p50 and p99 report.

Latency targets are budgets, not measurements. See
[specs/2/22-perf-verification.md](specs/2/22-perf-verification.md).

## Current limits

- The full exchange has known correctness, integration, and production gaps.
  [BUGS.md](BUGS.md) and [PROGRESS.md](PROGRESS.md) are the current queues.
- Perpetual futures are the only implemented instrument class.
- Gateway and market-data I/O are still being finalized.
- The system assumes a trusted internal network. External authentication ends
  at the gateway; internal casting is intentionally unauthenticated.
- Hot-path components are single-owner and designed to run on dedicated cores.
  Functional runs do not require pinning, but performance results do.

## How to read this repository

- [ARCHITECTURE.md](ARCHITECTURE.md) — process ownership, data flow, recovery,
  and system invariants.
- [specs/2/](specs/2/) — design contracts.
- [GUARANTEES.md](GUARANTEES.md) — ordering, durability, and loss bounds.
- [CRASH-SCENARIOS.md](CRASH-SCENARIOS.md) — failure cases.
- [RECOVERY-RUNBOOK.md](RECOVERY-RUNBOOK.md) — operator recovery.
- [docs/concepts/](docs/concepts/) — design rationale in plain language.
- Crate-local READMEs and architecture files — component APIs and internals.

## License

Project-owned code, documentation, and assets are licensed under
GPL-3.0-only. See [LICENSE](LICENSE). Bundled third-party material keeps
its own terms, recorded in [NOTICE](NOTICE).
