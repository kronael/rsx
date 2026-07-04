# Matching-engine / order-book alternatives — the wider field

Companion to `rsx-book/compare/` (which already benches five Rust
contenders head-to-head) and to `PLAN.md` (which scoped the fair-bench
seam). This doc is the `niche.md`-style census: the *whole* landscape of
real order-book + matching-engine implementations, their **core data
structure**, how they find best/next-best, their **published or claimed
latency**, the source URL, and whether a fair same-box Rust bench is
feasible or we must cite with caveats.

**Read the fairness rule first** (from `PLAN.md`, restated): a number is
only a head-to-head if same-op + same-depth + same-HW + same-instrument.
Everything cross-language / cross-machine below is *directional context*,
never "N× faster". rsx-book's own reference numbers (single core, AMD
Ryzen 9 5950X, Criterion, `reports/20260704_book-bench.md`): **match
~60–65 ns depth-invariant to 10M orders, level insert/cancel 15–33 ns,
best-read 1.47 ns.**

## Data-structure taxonomy

Every order book is one of a handful of shapes. This is the axis that
actually predicts performance:

| Shape | Best/next-best | Who uses it | Trade-off |
|---|---|---|---|
| **Sorted map** (BTree / red-black / skiplist) of price→level | O(log M) descend; cached best pointer → O(1) | naive BTreeMap, most crypto exchanges, `rust-orderbook`, hftbacktest BTree | Simple, correct, no bounds; next-best is a tree step (log M), cache-unfriendly at depth |
| **BST-of-levels + intrusive list + order-id hashmap** (WK Selph) | O(1) best via cached ptr; O(log M) only for *first* order at a new limit | WK Selph design, HFT-Orderbook (C/Py), liquibook (variant) | The canonical textbook design; add O(1) amortized, cancel O(1) |
| **Flat / price-indexed array** ("ladder") — array slot per tick | O(1) index by price; next-best = linear/bitmap scan | QuantCup/voyager, charles-cooper itch-order-book | Fastest per-op when tick range is bounded & dense; wastes memory / scans on sparse deep books |
| **Hashmap of price→level** (unsorted) | O(1) touch; best/next-best = separate tracking or scan | Databento MBO example, hftbacktest HashMap | O(1) updates, but no order → must track best separately |
| **Compression-map + slab arena + occupancy bitmap** | O(1) touch via compressed index; next-best via hierarchical bitmap O(depth=3) | **rsx-book** | Depth-invariant on *all* ops incl. next-best after a level empties; the bitmap is what fixes the array-ladder's scan weakness |
| **Adaptive radix tree (ART)** | O(k) by key length | exchange-core (Java) | Cache-friendlier than red-black at scale; JVM-resident |
| **Neighbor-aware balanced tree + PIN queue** | O(1) splice/graft using known in-order neighbor | arxiv "World's Fastest" (2026) | Claims to remove the log-n root-to-leaf walk; research, ARM Graviton4 |

rsx-book's insight vs the flat-array camp (QuantCup, itch-order-book):
those are O(1) on touch but pay an O(range) **scan** to find the
next-best level when the touch empties — exactly the bug the occupancy
bitmap fixed in rsx-book (`reports/20260704_book-bench.md`, `da9a2b4`).
rsx-book keeps the array's O(1) touch *and* gets O(depth=3) next-best.

## The full comparison table

| Impl | Lang | Data structure | Best / next-best | Published match/add/cancel latency | Source | Fair Rust bench? |
|---|---|---|---|---|---|---|
| **rsx-book** | Rust | compression-map + slab + occupancy bitmap | O(1) cached + O(3) bitmap | match 60–65 ns; ins/cxl 15–33 ns; best 1.47 ns | `reports/20260704_book-bench.md` | — (this is us) |
| **naive BTreeMap** | Rust | `BTreeMap<i64, VecDeque>` | O(log M) + cached | match 22 ns; ins+cxl 20–33 ns; best 2.69 ns | benched, `compare_all_bench.rs` | **YES — done** |
| **hftbacktest** BTree | Rust | `BTreeMarketDepth` (L2 agg) | `best_bid_tick` cached | touch 18–20 ns; best 1.77 ns (no matching) | benched; [repo](https://github.com/nkaz001/hftbacktest) | **YES — done** |
| **hftbacktest** HashMap | Rust | `HashMapMarketDepth` (L2 agg) | cached tick | touch 22–24 ns; best 1.49 ns (no matching) | benched | **YES — done** |
| **lob** (lob-rs 0.1.0) | Rust | order-level book, HashMap+tree | not public | ins+cxl 98 ns→1.22 µs (grows); match 81 ns | benched; [crate](https://crates.io/crates/lob) | **YES — done** (partial API) |
| **orderbook** (inv2004 0.1.9) | Rust | vec-based | — | — (crashes on empty-then-refill) | [repo](https://crates.io/crates/orderbook); `compare/orderbook-inv2004.md` | evaluated, **crashes** |
| **OrderBook-rs** (joaquinbejar) | Rust | `DashMap` + `crossbeam SkipMap` | skiplist | 168K ops/s aggregate @30 threads (contended); 31.6M ops/s hot-spot | [repo](https://github.com/joaquinbejar/OrderBook-rs) | wrong axis (contended); cite only |
| **QuantCup / voyager** (2011 winner) | C | intrusive linked lists + price-indexed array | O(1) array index | **~639 ns/op** | [gist](https://gist.github.com/druska/d6ce3f2bac74db08ee9007cdf98106ef) | **YES via port** ([Rust port](https://github.com/brettfazio/orderbook)) |
| **charles-cooper/itch-order-book** | C++ | flat vectors/arrays, single symbol | O(1) index | **61 ns/tick, 16M msgs/s** (2012 i7-3820, warm cache) | [repo](https://github.com/charles-cooper/itch-order-book) | stretch (same-box C++ rebuild) |
| **matching-engine-rs** (amankrx) | Rust | ITCH order book | — | **88 ns/msg, 11.3M msgs/s** (ITCH replay) | [repo](https://github.com/amankrx/matching-engine-rs) | medium (full ITCH pipeline, not level API) |
| **rust-orderbook** (TechieBoy) | Rust | tree-based | O(log n) | **~10 µs/match at 1M orders** (slow — tree walk) | [repo](https://github.com/TechieBoy/rust-orderbook) | YES but a *slow* baseline |
| **liquibook** (OCI) | C++ | depth book, full lifecycle | cached best | **2.0–2.5M inserts/s** (~400–500 ns/insert, our derivation) | [repo](https://github.com/enewhuis/liquibook) | medium (header-only C++, same-box) |
| **exchange-core** | Java | Adaptive Radix Tree + LMAX Disruptor | O(k) ART | p50 0.5 µs / p99 4 µs @1M ops/s; **~150 ns/match** (large mkt); move ~0.5 µs, cxl ~0.7 µs, new ~1.0 µs | [repo](https://github.com/exchange-core/exchange-core) | hard (JVM warmup); cite only |
| **arxiv "World's Fastest ME"** (2026) | ? | PIN queue + neighbor-aware tree | O(1) splice | **p50 376 ns / p99 524 ns @5M msgs/s**; 32M msgs/s single core | [arxiv 2606.01183](https://arxiv.org/html/2606.01183) | no (ARM Graviton4, not released); cite only |
| **WK Selph design** | C/Py | BST-of-limits + list + id-hashmap | O(1) cached best | O(1) cxl/exec, O(1)/O(log M) add; ~100–200K msgs/s Nasdaq context | [gist](https://gist.github.com/halfelf/db1ae032dc34278968f8bf31ee999a25) | it's a design, not a build |
| **Chronicle Matching Engine** | Java | off-heap, LMAX-style | — | sub-µs (commercial); Chronicle Queue 660 ns/event persisted | [chronicle.software](https://chronicle.software/building-fast-trading-engines-chronicles-approach-to-low-latency-trading/) | no (commercial); cite only |
| **LMAX Disruptor** | Java | ring buffer (NOT an order book) | — | 52 ns/hop, 25M msgs/s | [wiki](https://github.com/LMAX-Exchange/disruptor/wiki/Performance-Results) | wrong layer (messaging = rsx SPSC/rtrb, not book) |

## Serious contenders — one paragraph each

**QuantCup / voyager (2011).** The most-cited open matching-engine
micro-result: the winning entry of the QuantCup competition, plain C
with intrusive linked lists, global state, and a **price-indexed flat
array** for levels. Benchmarked at **~639 ns/op**. It is the direct
intellectual ancestor of the flat-ladder design and has been ported to
[Rust (brettfazio)](https://github.com/brettfazio/orderbook),
[Go](https://github.com/rdingwall/go-quantcup),
[Python](https://github.com/kmanley/orderbook), and
[C++ (ajtulloch)](https://github.com/ajtulloch/quantcup-orderbook). Because
a Rust port already exists, this is the single best *new* same-box bench
to add: it's the canonical flat-array design, it has a famous number to
validate our harness against, and it does real matching (fill
generation), unlike hftbacktest's L2 depth.

**charles-cooper/itch-order-book (C++).** Already in
`compare/cross-language-cited.md`: **61 ns/tick, 16M msgs/s** replaying a
real Nasdaq TotalView-ITCH file on a 2012 i7-3820, warm page cache, flat
vectors, single symbol. It is parse + book-maintenance (no fill
generation) so the fair rsx-book line to place next to it is *insert/cancel*
(15–33 ns), not `match_*`. Same-box rebuild (clone, build with their bench
flags, run our ITCH-replay adapter through rsx-book on the identical box)
is the single most bulletproof comparison available and remains the
`PLAN.md` stretch goal.

**exchange-core (Java).** The most rigorously benchmarked open engine:
LMAX Disruptor + Eclipse Collections + Agrona + **Adaptive Radix Trees**.
Published percentiles — p50 0.5 µs / p99 4 µs / p99.99 31 µs at 1M ops/s
on a 2010-era Xeon X5690 (isolated, tickless, mitigations off) — plus a
headline "**~150 ns per matching for large market orders**". JVM
GC/JIT warmup makes it a fundamentally different latency *shape* than a
native Rust Criterion run; use as an order-of-magnitude sanity check
("a production-grade engine's own micro-claim sits in the 100–200 ns
neighborhood as rsx-book's match"), never a head-to-head.

**liquibook (OCI, C++).** The other well-known open matching engine:
header-only C++, full order lifecycle (accept/reject/fill/cancel/replace
+ depth book). Self-reports **2.0–2.5M inserts/s**; any "~400–500 ns/insert"
is *our* arithmetic and must be labeled as a derivation, not their claim.
No HW spec, no percentiles. Header-only C++ makes a same-box rebuild
feasible if a cross-language number is wanted; medium effort.

**arxiv "The World's Fastest Matching Engine Algorithm" (2026).** The
current state-of-the-art *claim*: a Priority-Indicated Node fixed-capacity
queue plus "neighbor-aware balanced trees" that splice/graft in O(1)
using known in-order neighbors, avoiding the log-n root-to-leaf walk.
**p50 376 ns / p99 524 ns at 5M msgs/s, 32M msgs/s single core** on AWS
r8g.metal (ARM Graviton4); claims 11× liquibook, 4.7–6× exchange-core,
2.96× QuantCup on identical HW. No public release, ARM not our box —
cite only, but it is the most credible modern number and validates that
rsx-book's ~60 ns bare-match is in the right universe (their 376 ns is a
full ack-path including more than the isolated match step).

**hftbacktest / lob / naive-BTree (Rust).** Already benched fairly in
`compare_all_bench.rs` on the same box, same Criterion harness, same op
stream. See `compare/README.md` — rsx-book wins level-touch vs both
hftbacktest variants, wins match vs lob, and trails a bare BTreeMap by a
small constant (the documented cost of the compression/slab/bitmap
machinery that buys depth-invariance). No further work needed.

## Niche / long-tail census (one-liner each)

**Order books / matching engines**
- **WK Selph "How to Build a Fast LOB"** — the canonical design essay
  (BST-of-limits + doubly-linked list per level + order-id hashmap;
  add O(1)/O(log M), cancel/exec O(1), best O(1)).
  [gist](https://gist.github.com/halfelf/db1ae032dc34278968f8bf31ee999a25)
- **HFT-Orderbook** (Crypto-toolbox / Kismuz / wardbradt) — WK Selph in
  Python3 + C. [repo](https://github.com/Crypto-toolbox/HFT-Orderbook)
- **Databento "Constructing the LOB"** — reference MBO reconstruction
  guide (hashmap + sorted map); defines MBP-1/MBP-10 schemas everyone
  cites. [docs](https://databento.com/docs/examples/order-book/limit-order-book)
- **rust-orderbook** (TechieBoy) — tree-based Rust book, ~10 µs/match at
  1M orders; a *slow* baseline (no cached best, O(log n) walk).
  [repo](https://github.com/TechieBoy/rust-orderbook)
- **matching-engine-rs** (amankrx) — Rust ITCH engine, claims 88 ns/msg
  (11.3M msgs/s). [repo](https://github.com/amankrx/matching-engine-rs)
- **yiweichi/matching-engine** — Rust, 7 Criterion scenarios; small.
  [repo](https://github.com/yiweichi/matching-engine)
- **matchcore** — single-threaded deterministic Rust matcher, Criterion
  on Apple M4. [crate](https://crates.io/crates/matchcore)
- **RustQuant LOB tutorial** — pedagogical Rust build.
  [blog](https://rustquant.dev/blog/limit-order-book/)
- **da-bao-jian/fast_limit_orderbook** — WK-Selph-style simulator.
  [repo](https://github.com/da-bao-jian/fast_limit_orderbook)
- **CoinTossX** — open-source low-latency academic matching engine
  (JVM/Scala), published in SoftwareX.
  [paper](https://www.sciencedirect.com/science/article/pii/S2352711022000875)
- **Project Parity** (paritytrading) — open JVM exchange around
  ITCH/OUCH (also in rsx-cast/compare/niche.md).
- **Chronicle Matching Engine** — commercial Java, off-heap, sub-µs; the
  productized OpenHFT stack.
  [chronicle.software](https://chronicle.software/building-fast-trading-engines-chronicles-approach-to-low-latency-trading/)

**Design references (not benchmarkable order books)**
- **Carl Cook / Optiver, "When a Microsecond Is an Eternity"** (CppCon
  2017) — the canonical HFT-C++ talk; array-of-price-levels, cache/branch
  discipline. No isolated book ns.
  [PDF](https://github.com/CppCon/CppCon2017/blob/master/Presentations/When%20a%20Microsecond%20Is%20an%20Eternity/When%20a%20Microsecond%20Is%20an%20Eternity%20-%20Carl%20Cook%20-%20CppCon%202017.pdf)
- **"C++ design patterns for low-latency applications incl. HFT"** —
  arxiv survey. [arxiv 2309.04259](https://arxiv.org/pdf/2309.04259)
- **LMAX Disruptor** — 52 ns/hop ring buffer; a *messaging* layer, not a
  book. Compare against rsx's rtrb SPSC (50–170 ns), NOT rsx-book.
  [wiki](https://github.com/LMAX-Exchange/disruptor/wiki/Performance-Results)
- **Crypto-exchange architecture writeups** (Coinbase/Binance/Kraken
  style) — converge on red-black tree / skiplist, single-threaded per
  pair, event-sourced append log, shard by symbol. Directly mirror
  rsx's design (ME per symbol, WAL). No isolated per-op ns published.

## Recommendation — what to bench vs what to cite

**Already benched fairly (in `compare_all_bench.rs`, same box):** naive
BTreeMap, hftbacktest BTree, hftbacktest HashMap, lob. That covers the
sorted-map baseline, both L2-depth shapes, and one order-level Rust book.

**Add one new same-box bench — the QuantCup/voyager flat-array port.**
It is the missing data-structure shape (price-indexed array with real
matching), a Rust port already exists (brettfazio), and its 639 ns
number lets us validate the harness against a famous result. Wiring it
is one `impl BenchBook` + one line in `all_contenders()`.

**Optional second add — matching-engine-rs (amankrx):** real Rust ITCH
engine claiming 88 ns/msg; medium effort (it's a full ITCH pipeline, not
a level-touch API, so it needs an adapter or an ITCH-replay bench group).

**Stretch (cross-language, same-box rebuild):** itch-order-book (C++,
61 ns/tick) and liquibook (C++, header-only). Both buildable on our box;
both remove the HW variable if pursued. Flagged stretch in `PLAN.md`.

**Cite only, never head-to-head:** exchange-core (JVM warmup),
arxiv "World's Fastest" (unreleased ARM Graviton4), Chronicle
(commercial), OrderBook-rs (contended-throughput, wrong axis), LMAX
Disruptor (wrong layer — that's rsx's SPSC ring, not the book).

## Return summary

**Shortlist to benchmark in-repo (feasible, Rust, same box):**
1. Already done — naive BTreeMap, hftbacktest ×2, lob (`compare_all_bench.rs`).
2. **Add: QuantCup/voyager Rust port** — the flat-array shape + a famous
   639 ns number; trivial to wire as a `BenchBook`.
3. Optional: matching-engine-rs (88 ns/msg ITCH, needs an adapter).
4. Stretch cross-language same-box: itch-order-book (C++), liquibook (C++).

**Already covered by `compare_all`:** rsx-book, naive BTreeMap,
hftbacktest BTreeMarketDepth, hftbacktest HashMapMarketDepth, lob.
orderbook(inv2004) evaluated but crashes; orderbook-rs skipped (contended
axis, scope creep). Numbers in `compare/README.md`: rsx-book match 30.8 ns,
level-touch 15.5 ns, best-read 1.47 ns — beats both hftbacktest variants
and lob, trails a bare BTreeMap by a small constant.

**The 2–3 most credible published numbers to match/beat (all sourced,
all cross-machine so directional):**
1. **QuantCup/voyager: ~639 ns/op** (C, price-indexed array, 2011) —
   [gist](https://gist.github.com/druska/d6ce3f2bac74db08ee9007cdf98106ef).
   rsx-book's 60 ns match is ~10× under it; buildable to make it fair.
2. **charles-cooper/itch-order-book: 61 ns/tick, 16M msgs/s** (C++, flat
   arrays, 2012 i7) —
   [repo](https://github.com/charles-cooper/itch-order-book). The closest
   published peer to rsx-book's insert/cancel path (15–33 ns), different
   HW/op; same-box rebuild is the bulletproof stretch.
3. **exchange-core: ~150 ns/match, p50 0.5 µs @1M ops/s** (Java, ART +
   Disruptor) —
   [repo](https://github.com/exchange-core/exchange-core). Best-documented
   production-grade percentiles; JVM shape, cite as order-of-magnitude.
4. **arxiv "World's Fastest": p50 376 ns / p99 524 ns @5M msgs/s** (2026,
   Graviton4) — [arxiv](https://arxiv.org/html/2606.01183). The current
   SOTA claim; confirms rsx-book's ~60 ns bare-match is in-universe (their
   number is a fuller ack-path).
</content>
</invoke>
