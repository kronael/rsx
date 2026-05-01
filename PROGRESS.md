# PROGRESS

updated: May 01 2026

## Status: core complete, productionisation in progress

The 12 Rust crates build, the matching pipeline runs end-to-end
from a clean boot, and Playwright gate-4 is green. What's not
done: a measured GW→ME→GW p50/p99 under sustained load (the
probe is shipped — see commit `bded133`), schema versioning on
the wire, and tile-architecture parity for gateway and
marketdata (currently monoio reactors, not pinned tiles). See
`.ship/12-SHOWCASE-HONEST/` and `.ship/13-A16Z-FIXES/`.

The "% complete" framing was retired in this revision — every
crate has open work and stating otherwise is misleading. Status
verbs below describe what the crate currently delivers.

| Metric | Value | Source |
|--------|-------|--------|
| Crates | 12 | `Cargo.toml` workspace |
| Rust `#[test]` + `#[tokio::test]` attributes | 912 | `grep -rn '^#\[test\]\|^#\[tokio::test\]'` |
| Rust tests passing (`cargo test --workspace`) | 877 | `make test` |
| Python tests (rsx-playground) | ~930 | `pytest -q` |
| Playwright (gate-4) | 421 passing / 424 (3 conditional skips) | `make e2e` |
| LOC (Rust) | ~21k | `tokei` |

The attribute count (912) and runner-passing count (877) differ
because some tests are gated by feature flags or marked
`#[ignore]` for integration suites. Always quote whichever
matches the question being asked.

## Crate Status

| Crate | Status | Delivers | Open |
|-------|--------|----------|------|
| rsx-types | shipped | newtypes, config, validation | — |
| rsx-book | shipped | snapshot, matching, compression | proptest harness |
| rsx-matching | shipped | dedup, BBO, CONFIG_APPLIED | (T2.2) `(user,oid)` cancel index |
| rsx-dxs | shipped | WAL, CMP/UDP, DXS/TCP — domain-agnostic transport | (T3.1) wire schema versioning |
| rsx-messages | shipped | Fill, BBO, Order*, Mark, Liquidation, ConfigApplied, CancelRequest | — |
| rsx-gateway | shipped | JWT (HS256, exp/nbf, min-secret), rate limit, circuit breaker, REST, monoio WS | tile parity (pinning, ring) |
| rsx-risk | shipped | replication, funding, liquidation, PG write-behind, full tile (7 SPSC rings) | (T3.2) replica → main promotion via state machine |
| rsx-marketdata | shipped | shadow book, seq gap recovery, multi-ME | tile parity (pinning, ring) |
| rsx-mark | shipped | Binance/Coinbase aggregation, 1 SPSC ring | core pinning |
| rsx-recorder | shipped | daily rotation, buffered writes | — |
| rsx-cli | shipped | WAL dump (filters, stats, follow, display scale) | — |
| rsx-maker | shipped | two-sided quoting, reconnect | — |

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
