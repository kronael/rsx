# Cross-match plan: rsx-book vs public orderbook/matching benchmarks

Goal: replace "60ns, so what?" with "60ns vs \<named competitor\>, N× faster" —
without a claim that falls apart under scrutiny. READ-ONLY research; no code
changed. rsx-book's own numbers for reference: match ~60-65ns (depth-invariant,
per rsx-book/target/criterion + the rsx-book/demo bench), insert/cancel/BBO-scan
also in the tens-to-hundreds-of-ns range (rsx-book/benches/book_bench.rs,
deep_book_bench.rs).

## Competitor table

| Competitor | Public number | Workload / HW | Fair-comparison feasibility | What it'd prove |
|---|---|---|---|---|
| **charles-cooper/itch-order-book** | **61 ns/tick (16M msgs/s)**, 2012 i7-3820, single core | Real Nasdaq TotalView-ITCH file replay (add/execute/cancel/replace ticks), warmed page cache, C++, flat vectors/arrays (no hashmap/tree), single symbol | **Easy-Medium.** Same op class (order-book maintenance per real exchange tick), same unit (ns/op), same measurement discipline (warm cache, single core). HW differs (2012 i7 vs whatever rsx runs on) — must footnote or re-run on comparable/same box. It's parse+book-maintenance, not a matching *engine* (no fill generation) — rsx-book match ns includes fill generation, so line up rsx-book's *insert/cancel* bench against this, not `match_*`. | Most credible number to cite. Same data source (public), same order-of-magnitude op, transparent methodology. |
| **hftbacktest** (nkaz001, Rust) | No published per-op ns; only full-backtest-run wall time in READMEs (workload-dependent, not isolatable) | `MarketDepth` trait has 4 impls: `BTreeMarketDepth`, `HashMapMarketDepth`, `ROIVectorMarketDepth`, `FusedHashMapMarketDepth` (hftbacktest/src/depth/{btreemarketdepth,hashmapmarketdepth,roivectormarketdepth,fuse}.rs). L2 depth reconstruction only: `update_bid_depth`/`update_ask_depth`/`clear_depth` take `f64` price/qty, return a 6-tuple; no order-id-level FIFO queue, no fill/match generation. | **Hard, but buildable — and the highest-credibility cross-match because it's code-level, not quoted.** No number to "cite"; instead *we* write a Criterion bench with `hftbacktest` as a dev-dependency, drive `BTreeMarketDepth`/`HashMapMarketDepth::update_bid_depth` with the same synthetic or real L2-diff stream fed into `rsx-book`, and publish both numbers ourselves. Traps: (a) hftbacktest depth is L2-aggregated (price+qty only) — must compare against rsx-book's level-touch path (insert/modify/cancel at a level), not `match_*` (order-level, FIFO-aware); (b) hftbacktest uses `f64`, rsx-book uses fixed-point `i64` — either exclude the tick-conversion cost from both or include it in both, pick one and say which; (c) hftbacktest's crate isn't built for this — depth structs are internal-ish but are `pub use`d, so a thin bench crate can call them without modifying hftbacktest. | The only comparison against another *real, current, maintained* Rust engine's actual code, not a self-reported number. Directly rebuts "you only benched yourself." |
| **liquibook** (enewhuis/liquibook, C++) | "2.0-2.5M inserts/sec" (≈ 400-500 ns/insert derived) | No HW/workload details in README beyond "varies by HW/OS"; header-only C++ matching engine (full order lifecycle: accept/reject/fill/cancel/replace + depth book) | **Unfair as stated.** Aggregate throughput number, no percentile, no HW spec, no isolation of insert-only vs full accept-with-matching. Converting "2.5M/s" to "~400ns" is our own derivation, not their claim — must label it as such if used at all. | Weak evidence value; only useful as "the other well-known open-source matching engine reports a number in this ballpark" with heavy caveats. Do not present the derived ns figure as their number. |
| **exchange-core** (Java, LMAX-Disruptor-based) | Detailed latency percentiles: 1M ops/s → p50 0.5µs/p99 4µs/p99.99 31µs; 5M ops/s → p50 1.5µs/p99 42µs. Separately: "~150ns per matching for large market orders" | Intel Xeon X5690 (2010-era, 3.47GHz), 1 socket isolated+tickless, spectre/meltdown mitigations off, workload mix 9% GTC/3% IOC/6% cancel/82% move/~6% trigger, 1000 accounts × ~1000 orders across ~750 price slots, 3M messages | **Hard.** Percentile-based (their harness, not Criterion), JVM (GC/JIT warmup differ fundamentally from a native Rust bench), old HW, workload mix not replicable exactly without their harness. The "150ns per matching" line is closest in spirit to rsx-book's `match_*` benches but is an isolated micro-claim without full methodology shown. | Useful as an order-of-magnitude sanity check ("a serious production-grade engine's own micro-claim is in the same 100-200ns neighborhood") — NOT as a head-to-head number. |
| **LMAX Disruptor micro-benchmarks** (the ring buffer, not an order book) | 52ns mean latency per hop vs 32,757ns for ArrayBlockingQueue; "50ns tail latency," "25M msgs/s" in production | JVM, Disruptor's own 3-stage pipeline microbench | **Unfair / out of scope.** This benchmarks an inter-thread queue, not an order book or matching algorithm. Do not cite as an orderbook comparison — it measures a different layer (SPSC/MPSC messaging), which rsx already has its own analogous rtrb numbers for (50-170ns, per CLAUDE.md). If used, cite it against rsx's *ring* numbers, not `rsx-book`. | Only relevant if RSX separately publishes an SPSC-ring comparison; not an orderbook comparison. |
| **OrderBook-rs** (joaquinbejar, Rust) | 168K ops/s aggregate HFT-sim throughput (93K adds, 38K matches, 36K cancels /s); up to 31.6M ops/s in a "hot spot" contention microbench | Apple M4 Max, 30 concurrent threads (10 maker/10 taker/10 canceller), 5s run, `DashMap` + `crossbeam_skiplist::SkipMap`, mixed read/write 0-95% read ratio | **Unfair as-is.** This is a *concurrent* multi-writer benchmark (lock/contention-bound); rsx-book is single-threaded per symbol by design (one ME owns one book, no lock contention). Comparing single-thread rsx-book ns to a 30-thread contended-throughput number is apples-to-oranges in the wrong direction (their number is throughput-under-contention, ours is latency-uncontended). | Not directly usable. Could footnote as "designs that allow concurrent writers pay a contention tax rsx-book's single-writer-per-symbol design avoids" — a design-tradeoff note, not a speed claim. |
| **Naive `BTreeMap<Price, VecDeque<Order>>`** (build ourselves) | None yet — this is the baseline to build | Same box, same Criterion harness, same op set as rsx-book's existing `book_bench.rs`/`deep_book_bench.rs` | **Trivial / already fair by construction** — same measurement tool, same machine, same op definitions, zero external dependency risk. | The baseline every reviewer already expects ("did you compare against the obvious thing"). Answers the CEO-eval "so what" with a same-repo, same-harness number first, before reaching for external claims. |

## Recommended cross-match: the honest floor first, then hftbacktest

1. **Naive BTreeMap book** (build first, cheapest, in `rsx-book/benches/` or a new
   `compare/` dir per the crate's existing `notes/`/`compare/`-style naming —
   e.g. `rsx-book/compare/naive_btree.rs` + a Criterion bench target). Same ops
   as the existing suite: `insert_resting_order`, `cancel_order`,
   `modify_order_qty_down`, `match_single_fill`, `match_sweep_10_levels`, run
   through `BTreeMap<i64, VecDeque<Order>>` with linear scan for best-bid/ask.
   Same machine, same Criterion, same iteration count. This is the
   uncontroversial "obvious thing you compare against first" the CEO audit
   wants, and it's ours to fully control.

2. **hftbacktest depth cross-match** (highest external credibility). Concrete
   integration seam:
   - Add `hftbacktest = "..."` (crates.io, MIT/Apache — check current version)
     as a **dev-dependency only** in a new bench crate (not `rsx-book` itself,
     to keep the frozen/production crates untouched) — e.g.
     `rsx-book/compare/hftbacktest_cross/` with its own `Cargo.toml`, or a
     workspace-external throwaway crate under `.ship/34-.../bench-cross/` if
     the team doesn't want a permanent dependency in the tree.
   - Data source: hftbacktest publishes sample tick-level crypto data at
     `https://reach.stratosphere.capital/data/usdm/` (referenced from its own
     "Data" tutorial) — L2 depth-update events (price, qty, side, ts) in its
     own `.npz`/binary tick format via `hftbacktest::data`. Convert once to a
     flat `(side, price_ticks, qty, ts_ns)` vector shared by both benches, OR
     — simpler and more controllable — generate a **synthetic** update stream
     (N=100k updates across M=20/100/1000 price levels, realistic power-law
     level-touch distribution) so both books see byte-identical event order;
     this sidesteps hftbacktest's f64-tick/price-scaling code entirely and
     keeps the workload parameterizable to match rsx-book's own depth sweep
     (`deep_book_bench.rs` already parameterizes depth — reuse that shape).
   - Op mapping: `hftbacktest::depth::BTreeMarketDepth::update_bid_depth` /
     `update_ask_depth` (aggregated L2 level update+return-best) vs rsx-book's
     `insert_resting_order`/`modify_order_qty_down`/`cancel_order` (the
     level-touch path, NOT `match_*` — hftbacktest's depth has no matching).
     Also compare best-bid/ask read: hftbacktest `best_bid_tick()`/
     `best_ask_tick()` vs rsx-book's own best-bid scan bench
     (`best_bid_scan_after_cancel`).
   - Report BOTH `BTreeMarketDepth` (their tree-based impl, most comparable to
     our own naive baseline) and `HashMapMarketDepth`/`ROIVectorMarketDepth`
     (their optimized impls, most comparable to rsx-book's slab+compression
     design) — publishing only the slow one would be cherry-picking.

3. **itch-order-book (charles-cooper) as a cited number**, not a cross-match —
   we don't need to build anything for this one, just cite it with full
   caveats: 61ns/tick, 2012 HW, C++, parse+book-maintain (not match), single
   symbol, warm page cache. If we want it to be a true head-to-head rather
   than "here's a number someone else measured," the same public ITCH sample
   file (`ftp://emi.nasdaq.com/ITCH/` or `https://emi.nasdaq.com/ITCH/Nasdaq
   ITCH/MMDDYYYY.NASDAQ_ITCH50.gz`) can be replayed through rsx-book's own
   order-insert/cancel path (map ITCH Add/Cancel/Execute/Replace messages onto
   rsx-book ops) on the SAME box the itch-order-book number was NOT measured
   on — so this still requires a footnote that HW differs unless we also
   build+run itch-order-book locally for a same-box re-measurement. That
   re-measurement (clone itch-order-book, build with their bench flags, run on
   our box, then run our own ITCH-replay-adapter through rsx-book on the same
   box) is the single most bulletproof comparison available, because it
   removes the HW variable — flag as a **stretch goal**, not the first cut.

4. **liquibook / exchange-core**: cite only as order-of-magnitude
   context ("independent production engines self-report numbers in the
   100ns-1µs range too"), never as a head-to-head ns comparison — their
   published numbers don't meet the fairness bar (see guardrails below).

## Fairness guardrails

A comparison is FAIR only if all of these hold; if any is violated, either fix
it or downgrade the claim to "context" rather than "N× faster":

- **Same op.** Level-touch (insert/modify/cancel at a price level) is not the
  same op as match (walks levels, generates fills, updates two sides). Never
  put a competitor's insert number next to rsx-book's `match_*` number.
- **Same book depth/shape.** A 10-level book and a 10,000-level book have
  different cache-locality behavior; rsx-book's own benches already vary
  depth (`deep_book_bench.rs`) — match the competitor's depth parameter or
  don't compare.
- **Same hardware.** Every number above was measured on different silicon
  (2012 i7, Apple M4 Max, 2010 Xeon X5690, unspecified). Cross-machine ns
  comparisons are directionally suggestive at best — always disclose HW
  per number, and prefer re-running the competitor's own bench on our box
  when the code is available (hftbacktest, itch-order-book, liquibook: yes;
  exchange-core: yes but JVM warmup makes it noisier).
- **Same measurement method.** Criterion (statistical, warm, many iterations,
  outlier-aware) vs a competitor's ad-hoc `println!(elapsed)` timer are not
  the same instrument. hftbacktest cross-match: use Criterion for both sides
  (we control the harness). itch-order-book / liquibook / exchange-core: their
  published numbers come from their own harnesses — say so explicitly.
- **Full-engine vs in-process algorithm.** Several "public" numbers
  (exchange-core's throughput-at-N-ops/s, OrderBook-rs's HFT-sim, liquibook's
  inserts/sec) may include queueing, other threads, or contention that rsx's
  in-process `match_*` number does not. rsx-book's 60-65ns is a pure
  in-process algorithm number (see `.ship/`'s existing landscape doc: the
  matching *algorithm* is 340ns fully loaded incl. WAL framing, vs the
  bare match step which is faster) — always state which layer is being
  quoted on both sides.
- **Self-reported ≠ independently reproduced.** OrderBook-rs, matchcore, and
  similar small crates publish their own numbers with no third-party
  verification and (for OrderBook-rs) workloads that don't match rsx-book's
  usage pattern (concurrent multi-writer vs single-writer-per-symbol). Treat
  these as "someone's claim," not evidence, unless we rerun their harness
  ourselves.

## Build-first ranking

1. **Naive `BTreeMap<Price, VecDeque>` bench in rsx-book** — cheapest, zero
   external risk, answers the most predictable "so what" question, reuses the
   existing Criterion bench harness and op definitions verbatim.
2. **hftbacktest depth cross-match** — highest credibility because it's a
   real, current, independently-maintained Rust project's actual code, run by
   us on our own box with our own harness (no cross-machine variable, no
   trust-their-number issue). Requires: adding hftbacktest as a scoped
   dev-dependency in an isolated bench crate, and a synthetic or converted L2
   update stream shared by both sides.
3. **itch-order-book same-box re-measurement** (stretch) — most bulletproof
   if pursued to completion (removes the HW variable via local rebuild), but
   costs more (build their C++ project, get real ITCH sample data, write an
   ITCH-to-rsx-book adapter for Add/Cancel/Execute/Replace). Do this after #2
   if the numbers from #1/#2 look good and a stronger claim is wanted.
4. **liquibook / exchange-core context citations** — no build required, just
   careful wording in whatever doc uses these; lowest effort, lowest evidence
   value, use only as "others report similar order-of-magnitude" framing.
5. **LMAX Disruptor / OrderBook-rs** — do not use as orderbook comparisons at
   all; wrong layer (Disruptor = messaging) or wrong axis (OrderBook-rs =
   contended-throughput, not uncontended-latency). Skip unless the goal shifts
   to a concurrency-design tradeoff writeup instead of a speed claim.

## Sources

- [charles-cooper/itch-order-book](https://github.com/charles-cooper/itch-order-book) — 61ns/tick, 16M msgs/s, 2012 i7-3820, C++, flat arrays, real Nasdaq ITCH replay
- [Nasdaq EMI ITCH sample data (public FTP)](https://emi.nasdaq.com/ITCH/Nasdaq%20ITCH/) — e.g. `01302019.NASDAQ_ITCH50.gz`
- [Nasdaq TotalView-ITCH 5.0 spec (PDF)](https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/NQTVITCHSpecification.pdf)
- [nkaz001/hftbacktest](https://github.com/nkaz001/hftbacktest) — Rust HFT backtesting framework
- [hftbacktest depth module (mod.rs, GitHub raw)](https://raw.githubusercontent.com/nkaz001/hftbacktest/master/hftbacktest/src/depth/mod.rs) — `MarketDepth`/`L2MarketDepth` traits, `BTreeMarketDepth`/`HashMapMarketDepth`/`ROIVectorMarketDepth`/`FusedHashMapMarketDepth`
- [hftbacktest docs (readthedocs)](https://hftbacktest.readthedocs.io/) — Data/Data Preparation tutorials, sample data pointer
- [hftbacktest on crates.io](https://crates.io/crates/hftbacktest)
- [enewhuis/liquibook](https://github.com/enewhuis/liquibook) — "2.0-2.5M inserts/sec," header-only C++ matching engine
- [exchange-core/exchange-core](https://github.com/exchange-core/exchange-core) — LMAX-Disruptor-based Java matching engine, detailed latency percentiles, "~150ns per matching for large market orders," Intel Xeon X5690 workload spec
- [LMAX Disruptor performance results](https://github.com/LMAX-Exchange/disruptor/wiki/Performance-Results) — 52ns mean latency/hop, ring-buffer messaging (not an order book)
- [The LMAX Architecture (Martin Fowler)](https://martinfowler.com/articles/lmax.html)
- [joaquinbejar/OrderBook-rs](https://github.com/joaquinbejar/OrderBook-rs) — `DashMap` + `crossbeam_skiplist::SkipMap`, Apple M4 Max, concurrent contention benchmarks (168K ops/s HFT-sim, up to 31.6M ops/s hot-spot microbench)
- [matchcore on crates.io](https://crates.io/crates/matchcore) — single-threaded deterministic Rust matcher, Criterion benchmarks on Apple M4 (page content not independently confirmed beyond crates.io listing — re-check before citing numbers)
- rsx-book existing benches (this repo): `rsx-book/benches/book_bench.rs`, `rsx-book/benches/deep_book_bench.rs`, `rsx-book/target/criterion/*` — the baseline our own numbers already come from
