# PROGRESS

updated: May 30 2026

## Status: core complete, productionisation in progress

The 13 Rust crates build, the matching pipeline runs end-to-end
from a clean boot, and Playwright gate-4 is green. **`rsx-cast`
is done and tested** — transport: WAL + casting/UDP +
replication/TCP, V1 wire-format version byte. **Matching, risk,
and gateway are done.** WS single-stream round-trip is now
measured at p50 **2.25 ms** (down from 11.5 ms before the gateway
egress-drain fix — see `reports/20260530_e2e-ws-latency.md`).
What's not done: a measured GW→ME→GW p50/p99 under *sustained
parallel* load (the parallel harness currently hits
`ME-FAULTED-NO-REPLAY-ADDR` — see bugs.md), schema versioning on
the wire, and tile-architecture parity for gateway and
marketdata (currently monoio reactors, not pinned tiles).
Sprint history baked into `CHANGELOG.md` + `.diary/`; the
ad-hoc `.ship/12-*` and `.ship/13-*` audit dirs were pruned
on close-out.

The "% complete" framing was retired in this revision — every
crate has open work and stating otherwise is misleading. Status
below is **done** (no open work) or **in progress** (open items
named in the last column).

| Metric | Value | Source |
|--------|-------|--------|
| Crates | 13 | `Cargo.toml` workspace |
| Rust `#[test]` + `#[tokio::test]` attributes | 912 | `grep -rn '^#\[test\]\|^#\[tokio::test\]'` |
| Rust tests passing (`cargo test --workspace`) | 878 | `make test` |
| Python tests (rsx-playground) | ~930 | `pytest -q` |
| Playwright (gate-4) | 421 passing / 424 (3 conditional skips) | `make e2e` |
| LOC (Rust) | ~21k | `tokei` |

The attribute count (912) and runner-passing count (878) differ
because some tests are gated by feature flags or marked
`#[ignore]` for integration suites. Always quote whichever
matches the question being asked.

Refinement history: ~28 commits tagged `[refine]` since v0.1.0
plus ~12 commits from the a16z-fixes security and correctness
batch (sprint dir pruned; commits in `git log`).

## Crate Status

| Crate | Status | Delivers | Open |
|-------|--------|----------|------|
| rsx-types | done | newtypes, config, validation, invariant-named asserts | — |
| rsx-book | in progress | snapshot, matching, compression | book-session bugs (bugs.md); proptest harness |
| rsx-matching | done | dedup, BBO, CONFIG_APPLIED, O(1) `(user,oid)` cancel index | — |
| rsx-cast | **done (tested)** | WAL, casting/UDP, replication/TCP, V1 wire-format version byte (at offset 0), preallocated send_ring | — |
| rsx-messages | done | Fill, BBO, Order*, Mark, Liquidation, ConfigApplied, CancelRequest (extracted from rsx-cast) | — |
| rsx-gateway | in progress | JWT (32B min, exp/nbf, JtiTracker), per-IP rate limit (FIFO eviction), circuit breaker, REST, monoio WS, egress-drain 500µs | tile parity (pinning, ring) |
| rsx-risk | done | replication, funding, liquidation, PG write-behind, full tile (SPSC rings), eager warm-standby replica → main promotion (state machine) | MIGRATIONS-UNLOCKED (low, triage — bugs.md) |
| rsx-marketdata | in progress | shadow book, seq gap recovery, multi-ME, Arc fan-out | tile parity (pinning, ring) |
| rsx-mark | done | Binance/Coinbase aggregation, 1 SPSC ring, off-path (sleeps, unpinned) | — |
| rsx-recorder | done | daily rotation, buffered writes | — |
| rsx-cli | done | WAL dump (filters, stats, follow, display scale) | — |
| rsx-log | done | per-thread SPSC ring → drain thread → tracing events; compile-time `latency_sample!` gate | — |
| rsx-health | done | unified `/health` · `/ready` · `/metrics` (queue/saturation gauges) on a port per daemon | — |

## Playground

- 14 tabs, 60+ API endpoints
- Scenarios: minimal/duo/full/stress implemented
- stress.py subprocess management
- ~930 Python tests, 421 Playwright (canonical)
- bench-gate.sh regression gating (10% threshold)

## Benchmarks (rsx-book, release)

| Operation | Latency |
|-----------|---------|
| match single fill | 54 ns |
| insert resting order | 857 ns |
| `WalWriter::prepare` + `append_framed` (Vec extend, pre-fsync) | 31 ns |
| WAL flush + fsync 64 KB | 24 µs |
| protocol-record encode (Nak / CastHeartbeat) | 43 ns |
| `FillRecord` encode | 23 ns |
| protocol-record decode | 9 ns |

## Remaining Polish

- Trade UI: nginx WS proxy, positions, reconnect
