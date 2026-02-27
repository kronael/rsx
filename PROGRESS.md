# PROGRESS

updated: Feb 27 2026

## Status: ~99% complete

| Metric | Value |
|--------|-------|
| Crates | 9 |
| Rust tests | ~570 |
| Python e2e | 85 |
| Playwright | 228 |
| LOC (Rust) | ~21k |

## Crate Status

| Crate | Status | Notes |
|-------|--------|-------|
| rsx-types | 100% | newtypes, config, validation |
| rsx-book | 100% | snapshot save/load, 9 tests |
| rsx-matching | 100% | dedup, BBO, CONFIG_APPLIED |
| rsx-dxs | 99% | WAL dump works, missing payload decode |
| rsx-gateway | 97% | JWT, rate limit, circuit breaker |
| rsx-risk | 99% | replication done, PG tests #[ignore] |
| rsx-marketdata | 98% | seq gap detection done |
| rsx-mark | 100% | Binance/Coinbase aggregation |
| rsx-recorder | 100% | daily rotation, buffered writes |
| rsx-cli | 95% | WAL dump, missing LIQUIDATION decode |
| rsx-maker | 100% | two-sided quoting, reconnect |
| rsx-sim | deleted | crate removed |

## Playground

- 14 tabs, 60+ API endpoints
- All screens verified with live processes (Feb 27)
- 10 bugs found and fixed during verification
- See TESTING.md for details

## Remaining Work

- Latency pipeline (real gateway orders needed)
- Scenarios: duo/full/stress not implemented
- stress.py subprocess management (stress generator)
- CLI payload decoding (LIQUIDATION field decode)
