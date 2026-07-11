# PROGRESS

Per-component status and current counts. Status means:

- **signed off** — frozen for v1; bug fixes only unless the founder approves a
  change;
- **release candidate** — working and through its current gates, but not signed
  off;
- **working** — runs and passes the tracked checks, but has not entered final
  sign-off;
- **in development** — runs, but known work still changes its release shape.

None of the crates or binaries is published to an external registry. Git tags
through `v0.7.1` are pre-v1 source releases, not component sign-off.

| Metric | Value | Source |
|---|---|---|
| Crates | 13 | `Cargo.toml` |
| Rust `#[test]` / `#[tokio::test]` | 912 | grep |
| Rust passing (`cargo test --workspace`) | 878 | `make test` |
| Python (rsx-playground) | ~930 | `pytest -q` |
| Playwright (gate-4) | 421 / 424 (3 conditional skips) | `make e2e` |
| Rust LOC | ~21k | `tokei` |

912 attributes vs 878 passing: the rest are `#[ignore]` integration or
feature-gated. Refinement history: ~28 `[refine]` commits since v0.1.0 plus
~12 from the a16z-fixes batch (in `git log`).

## Crates

| Crate | Status | Delivers | Open |
|---|---|---|---|
| rsx-types | working | newtypes, config, validation, invariant asserts | — |
| rsx-book | in development | snapshot, matching, compression | book bugs (BUGS.md); proptest harness |
| rsx-matching | release candidate | dedup, BBO, CONFIG_APPLIED, O(1) `(user,oid)` cancel | final sign-off |
| rsx-cast | signed off | WAL, casting/UDP, replication/TCP, V1 wire version byte, prealloc send_ring | — |
| rsx-messages | working | Fill, BBO, Order*, Mark, Liquidation, ConfigApplied, CancelRequest | final sign-off |
| rsx-gateway | in development | JWT, per-IP rate limit, circuit breaker, REST, monoio WS, 500µs egress-drain | tile parity (pinning, ring) |
| rsx-risk | working | replication, funding, liquidation, PG write-behind, full tile, warm-standby promotion | MIGRATIONS-UNLOCKED (BUGS.md); final sign-off |
| rsx-marketdata | in development | shadow book, seq-gap recovery, multi-ME, Arc fan-out | tile parity (pinning, ring) |
| rsx-mark | working | Binance/Coinbase aggregation, off-path (sleeps, unpinned) | final sign-off |
| rsx-recorder | working | daily rotation, buffered writes | final sign-off |
| rsx-cli | working | WAL dump (filters, stats, follow, scale) | final sign-off |
| rsx-log | working | per-thread SPSC → drain → tracing; `latency_sample!` gate | final sign-off |
| rsx-health | working | `/health` `/ready` `/metrics`, port per daemon | final sign-off |

## Other required v1 surfaces

| Surface | Status | Open |
|---|---|---|
| rsx-term | in development | final RSX integration and sign-off |
| production deploy | in development | founder-run host deployment and sign-off |

RSX reaches v1 only when every crate and surface in these two tables passes its
release gates and is explicitly signed off. A green test suite, a working demo,
or a tagged pre-v1 release is not sign-off.

Not yet: GW→ME→GW p50/p99 under sustained parallel load (blocked on
`ME-FAULTED-NO-REPLAY-ADDR`), wire schema versioning, tile parity for
gateway/marketdata.

## Benchmarks (rsx-book, release)

| Op | Latency |
|---|---|
| match single fill | 54 ns |
| insert resting order | 857 ns |
| `WalWriter::prepare` + `append_framed` | 31 ns |
| WAL flush + fsync 64 KB | 24 µs |
| record encode (Nak / CastHeartbeat) | 43 ns |
| FillRecord encode | 23 ns |
| record decode | 9 ns |

## Playground

14 tabs, 60+ API endpoints; minimal/duo/full/stress scenarios; ~930 Python
tests, 421 Playwright; bench-gate 10% regression threshold.

## Remaining

Trade UI: nginx WS proxy, positions, reconnect.
