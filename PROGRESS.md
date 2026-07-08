# PROGRESS

Per-crate status and current counts. Each crate is **done** (no open work)
or **in progress** (open items in the last column) — no "% complete."

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
| rsx-types | done | newtypes, config, validation, invariant asserts | — |
| rsx-book | in progress | snapshot, matching, compression | book bugs (BUGS.md); proptest harness |
| rsx-matching | done | dedup, BBO, CONFIG_APPLIED, O(1) `(user,oid)` cancel | — |
| rsx-cast | done | WAL, casting/UDP, replication/TCP, V1 wire version byte, prealloc send_ring | — |
| rsx-messages | done | Fill, BBO, Order*, Mark, Liquidation, ConfigApplied, CancelRequest | — |
| rsx-gateway | in progress | JWT, per-IP rate limit, circuit breaker, REST, monoio WS, 500µs egress-drain | tile parity (pinning, ring) |
| rsx-risk | done | replication, funding, liquidation, PG write-behind, full tile, warm-standby promotion | MIGRATIONS-UNLOCKED (BUGS.md) |
| rsx-marketdata | in progress | shadow book, seq-gap recovery, multi-ME, Arc fan-out | tile parity (pinning, ring) |
| rsx-mark | done | Binance/Coinbase aggregation, off-path (sleeps, unpinned) | — |
| rsx-recorder | done | daily rotation, buffered writes | — |
| rsx-cli | done | WAL dump (filters, stats, follow, scale) | — |
| rsx-log | done | per-thread SPSC → drain → tracing; `latency_sample!` gate | — |
| rsx-health | done | `/health` `/ready` `/metrics`, port per daemon | — |

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
