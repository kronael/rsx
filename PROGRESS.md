# PROGRESS

updated: May 01 2026

## Status: core complete, surfacing & spec rigor in progress

The 11 Rust crates build, the matching pipeline runs end-to-end
from a clean boot, and Playwright gate-4 is green. What's not
done: surfacing the genuine novelty (latency dashboard, E2E
harness), tightening the spec corpus so it doesn't retract
its own claims, and a few migrations off `tokio` on services
that should be on `monoio`. See `.ship/12-SHOWCASE-HONEST/`.

| Metric | Value |
|--------|-------|
| Crates | 11 |
| Rust tests (unit + integration) | ~1,200 |
| Python tests (rsx-playground) | ~930 |
| Playwright (gate-4) | 421 passing / 424 total (3 conditional skips) |
| LOC (Rust) | ~21k |

## Crate Status

| Crate | Status | Notes |
|-------|--------|-------|
| rsx-types | 100% | newtypes, config, validation |
| rsx-book | 100% | snapshot, matching, compression |
| rsx-matching | 100% | dedup, BBO, CONFIG_APPLIED |
| rsx-dxs | 100% | WAL, CMP, DXS replay, CLI decode |
| rsx-gateway | 100% | JWT, rate limit, circuit breaker, REST |
| rsx-risk | 100% | replication, funding, liquidation, PG |
| rsx-marketdata | 100% | shadow book, seq gap, multi-ME |
| rsx-mark | 100% | Binance/Coinbase aggregation |
| rsx-recorder | 100% | daily rotation, buffered writes |
| rsx-cli | 100% | filters, stats, follow, display scale |
| rsx-maker | 100% | two-sided quoting, reconnect |

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
| WAL append (in-memory) | 31 ns |
| WAL flush+fsync 64KB | 24 µs |
| CMP encode | 43 ns |
| CMP decode | 9 ns |

## Remaining Polish

- Trade UI: nginx WS proxy, positions, reconnect
