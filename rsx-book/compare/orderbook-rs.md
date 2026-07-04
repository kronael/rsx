# orderbook-rs (joaquinbejar/OrderBook-rs, crates.io `orderbook-rs` 0.9.0) — skipped

Named directly in the original cross-match research
(`.ship/34-COMPARE-RESEARCH/PLAN.md`) as `OrderBook-rs`: a `DashMap` +
`crossbeam_skiplist::SkipMap`-based concurrent order book, with a published
self-reported number of 168K ops/s aggregate HFT-sim throughput (93K
adds/38K matches/36K cancels per sec) on a 30-thread contended benchmark,
Apple M4 Max.

## Why it isn't in `compare_all_bench.rs`

Two independent reasons, either one alone would be enough:

1. **Scope grew since the plan's research snapshot.** Version 0.9.0 (the
   current crates.io release) is no longer the compact concurrent
   price-level crate the plan characterized — it now bundles a full wire
   protocol (`src/wire/{inbound,outbound}`), a risk engine, options pricing
   (Black-Scholes, implied-vol solver), a sequencer/journal
   (`FileJournal`, `BincodeEventSerializer`), NATS publishing, and a
   `BookManagerStd`/`BookManagerTokio` pair. Integrating it cleanly (correct
   `Id`/`Side`/`TimeInForce` construction from the `pricelevel` companion
   crate, picking the right manager variant to avoid pulling in tokio) is a
   meaningfully bigger task than the other four contenders combined, for a
   crate whose core numbers are already flagged as unfair below.
2. **The plan's own fairness guardrail already disqualifies its published
   number for a latency head-to-head.** `168K ops/s` is aggregate
   throughput under 30-thread contention (`DashMap`/`SkipMap` atomics,
   lock-free but not lock-free of contention cost). rsx-book is
   single-writer-per-symbol by design (one ME instance owns one book's
   mutation, per `specs/2/45-tiles.md`) — there is no contention to measure
   against. Even a clean same-box, single-threaded rebench of
   `orderbook-rs`'s uncontended path would only show "how fast is this
   crate when you don't use the concurrency it's built for," which doesn't
   answer the question the crate's own design optimizes for.

## Verdict

Cited as context only, per the plan's original guardrail: "designs that
allow concurrent writers pay a contention tax rsx-book's single-writer-
per-symbol design avoids" — a design-tradeoff note, not a speed claim.
Not benched. Not disqualified by a crash (unlike `orderbook` inv2004,
see `compare/orderbook-inv2004.md`) — disqualified by scope/fit.
