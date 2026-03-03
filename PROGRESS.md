# PROGRESS

updated: Mar 03 2026

## Status: 100% core, polish remaining

| Metric | Value |
|--------|-------|
| Crates | 11 |
| Rust tests | ~895 |
| Python e2e | 87 |
| Playwright | 228 |
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
- 87 Python e2e tests, 228 Playwright tests
- bench-gate.sh regression gating

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
