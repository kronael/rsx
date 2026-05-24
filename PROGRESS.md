# PROGRESS

updated: May 21 2026

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
| Rust tests passing (`cargo test --workspace`) | 878 | `make test` |
| Python tests (rsx-playground) | ~930 | `pytest -q` |
| Playwright (gate-4) | 421 passing / 424 (3 conditional skips) | `make e2e` |
| LOC (Rust) | ~21k | `tokei` |

The attribute count (912) and runner-passing count (878) differ
because some tests are gated by feature flags or marked
`#[ignore]` for integration suites. Always quote whichever
matches the question being asked.

Refinement history: ~28 commits tagged `[refine]` since v0.1.0
plus ~12 commits from the `.ship/13-A16Z-FIXES/` security and
correctness batch.

## Crate Status

| Crate | Status | Delivers | Open |
|-------|--------|----------|------|
| rsx-types | shipped | newtypes, config, validation, invariant-named asserts | — |
| rsx-book | shipped | snapshot, matching, compression | proptest harness |
| rsx-matching | shipped | dedup, BBO, CONFIG_APPLIED, O(1) `(user,oid)` cancel index | — |
| rsx-cast | shipped | WAL, casting/UDP, replication/TCP, V0/V1 wire-format version byte, preallocated send_ring | — |
| rsx-messages | shipped | Fill, BBO, Order*, Mark, Liquidation, ConfigApplied, CancelRequest (extracted from rsx-cast) | — |
| rsx-gateway | shipped | JWT (32B min, exp/nbf, JtiTracker dormant), per-IP rate limit (FIFO eviction), circuit breaker, REST, monoio WS | tile parity (pinning, ring) |
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
| `WalWriter::append` (Vec extend, pre-fsync) | 31 ns |
| WAL flush + fsync 64 KB | 24 µs |
| protocol-record encode (StatusMessage / Nak / Heartbeat) | 43 ns |
| `FillRecord` encode | 23 ns |
| protocol-record decode | 9 ns |

## Remaining Polish

- Trade UI: nginx WS proxy, positions, reconnect
