# rsx-matching benches — uniform harness baseline (Phase 1)

**Date:** 2026-07-03
**Crate:** `rsx-matching`
**Sprint:** `.ship/31-BOOK-MATCH-CAST-TREATMENT` Phase 1 (Measure)
**Status:** harness + bench set landed and compiling; **NUMBERS PENDING**
(box under heavy docker load at capture time — see Caveats). Structure,
methodology, and per-figure attribution are final; p50/throughput columns
are to be filled on a quiet box.

## What this is

Phase 1 of the "cast treatment" for the matching engine: consolidate the
four existing `rsx-matching` benches onto ONE shared harness so every
figure is measured against identically-constructed state, with the same
core pinning and Criterion statistics — a clean, directly-comparable set
of rsx-matching's OWN numbers. No competitor baselines yet (that is
Phase 2).

All numbers are **single-box, in-process microbenchmarks** — the matching
algorithm and its accept path in isolation, no UDP / WS / cross-process
transport. They are compute floors, not wire-to-wire latencies. Run it
yourself: `cargo bench -p rsx-matching`.

## The uniform harness (`rsx-matching/benches/harness.rs`)

One module, included by every bench via `#[path = "harness.rs"] mod
harness;`. It fixes the things that, if they drifted per-bench, would make
the numbers unfair:

- **Core pinning:** the timed Criterion thread pins to core 2
  (`harness::pin()`, `core_affinity`) — same convention as the cast and
  book harnesses, so cross-crate runs share the core.
- **Criterion config:** `harness::criterion()` = `sample_size(50)` —
  matches the cast/book convention so statistics are comparable.
- **Symbol config:** tick 1, lot 1 → raw fixed-point units; `MID =
  100_000` (same mid the prior matching benches used, so carried-over
  numbers line up).
- **Shared fixtures:**
  - `build_book(depth)` — a book of `depth` resting asks laddered up from
    `MID+1`, best level `BIG_QTY` (1e9). A qty-1 taker at `MID+1` does one
    **non-draining** partial fill, so the match work is held constant
    while only resting depth varies. Deterministic.
  - `single_ask(qty)` — a one-level book for the by-order-type benches.
  - `Me` — the full ME critical section as a reusable fixture (real
    `Orderbook` seeded to a depth + real `WalWriter` on a tempdir + real
    `DedupTracker` + real FxHashMap order index). `Me::accept()` runs the
    exact sequence the ME main loop runs between `me_in` and `me_out`
    (sans cast send): dedup check → `OrderAcceptedRecord` WAL append →
    `process_new_order` → `write_events_to_wal` → order-index update.

No bench re-rolls its own config, pin, or symbol — drift is how unfair
numbers creep in.

## The bench set

Six Criterion groups across six bench files, each measuring one concern.

| Group | File | What it measures |
|-------|------|------------------|
| `match_by_depth/n={1,100,1k,10k,100k}` | `match_depth_bench.rs` | One qty-1 taker fill vs resting book depth. Match work held constant; isolates whether a fuller book/slab slows a single match (should be O(1) best-level access, depth-independent). |
| `match_by_order_type/{gtc_full_cross,ioc,fok,post_only_rest,reduce_only}` | `match_by_type_bench.rs` | Cost of each order type's distinct path: full cross, IOC residual-done, FOK liquidity check + fill, post-only that rests, reduce-only sell reducing a real long position against a resting bid. |
| `sweep_n_levels/n={1,5,20,100}` | `match_n_levels_bench.rs` | One aggressor sweeping N single-order price levels (partial fills across levels) — how the match loop scales with fills (O(consumed)). |
| `dedup/{insert_new,hit_duplicate,cleanup_10k}` | `matching_bench.rs` | The duplicate-order guard every accepted order pays (FxHashMap insert / hit / bulk 10k cleanup). |
| `wal_events/{append_1_fill,drain_10_fills,drain_100_fills}` | `matching_bench.rs` | Serializing a match's emitted events to WAL (`write_events_to_wal`, no fsync) + draining the event buffer the risk/mkt fan-out iterates. |
| `me_accept_path/full`, `me_throughput/orders` | `process_order_bench.rs` | The full `Me::accept()` critical section — per-order latency (p50) and orders/s. Each accept does one fill, so fills/s == orders/s here. |
| `wal_replay_30k_records` | `wal_replay_bench.rs` | Cold-start WAL replay: drain 30k records (10k accepted + 10k fill + 10k bbo) — the cost an ME restart pays before its first order. |

Pure orderbook data-structure micro-benches (slab alloc/free,
price→index compression) were dropped from the matching set — they belong
to `rsx-book`'s bench set and would double-count here.

## Numbers

**PENDING.** Not recorded in this run. See Caveats — the box was under
heavy docker oversubscription at capture time, so a pinned-core microbench
baseline would be contended noise, not a clean directly-comparable set
(the whole point of Phase 1).

| Group / point | p50 | throughput | bench |
|---------------|-----|-----------|-------|
| `match_by_depth/n=1` | _pending_ | — | `match_depth_bench.rs` |
| `match_by_depth/n=100` | _pending_ | — | `match_depth_bench.rs` |
| `match_by_depth/n=1000` | _pending_ | — | `match_depth_bench.rs` |
| `match_by_depth/n=10000` | _pending_ | — | `match_depth_bench.rs` |
| `match_by_depth/n=100000` | _pending_ | — | `match_depth_bench.rs` |
| `match_by_order_type/gtc_full_cross` | _pending_ | — | `match_by_type_bench.rs` |
| `match_by_order_type/ioc` | _pending_ | — | `match_by_type_bench.rs` |
| `match_by_order_type/fok` | _pending_ | — | `match_by_type_bench.rs` |
| `match_by_order_type/post_only_rest` | _pending_ | — | `match_by_type_bench.rs` |
| `match_by_order_type/reduce_only` | _pending_ | — | `match_by_type_bench.rs` |
| `sweep_n_levels/n={1,5,20,100}` | _pending_ | — | `match_n_levels_bench.rs` |
| `dedup/{insert_new,hit_duplicate,cleanup_10k}` | _pending_ | — | `matching_bench.rs` |
| `wal_events/{append_1_fill,drain_10,drain_100}` | _pending_ | — | `matching_bench.rs` |
| `me_accept_path/full` | _pending_ | — | `process_order_bench.rs` |
| `me_throughput/orders` | _pending_ | _pending_ orders/s | `process_order_bench.rs` |
| `wal_replay_30k_records` | _pending_ | — | `wal_replay_bench.rs` |

### Indicative-only validation figures (NOT the baseline)

To confirm the new fixtures execute correctly (no panic in the reduce-only
position path, the post-only rest path, or the `Me` full path), a short
contended run was taken (sample_size 10, 0.5s measurement, cluster tiles
still up). These are **contended noise, deliberately withheld from the
table above** and shown only to demonstrate the harness works and roughly
where numbers land:

- `match_by_depth`: ~28–33 ns **flat** across n=1 / 100 / 1k / 10k —
  confirms depth-independence (consistent with the ~52 ns flat deep-book
  match in `20260530_component-benches.md`; ours is a single-fill IOC,
  slightly cheaper).
- `me_accept_path/full`: ~220–280 ns — consistent with the ~210 ns
  `me_process_order_full_path` floor recorded 2026-05-30.
- order types: gtc/ioc/reduce_only ~400–550 ns (includes iter_batched
  batching overhead), fok showed a 12 µs contended outlier — exactly the
  kind of artifact that makes recording-under-load dishonest.

Carried-over context (measured 2026-05-30, `20260530_component-benches.md`,
not re-run): ME in-process match floor ~**210 ns** p50; deep-book match
~**52 ns** flat at 100k/1m/10m resting (depth-independent, O(consumed));
per-fill increment ~54 ns.

## Caveats (honesty guardrails)

- **Single box, in-process microbench.** No UDP/WS/cross-process. These
  are compute floors; the full GW→ME→GW round-trip is transport-bound
  (~4 casting hops), not compute-bound — see `20260530_e2e-ws-probe.md`.
- **Numbers withheld due to host contention.** At capture time the box (4
  cores) was under docker oversubscription (load ~6.25; three docker
  containers + dockerd consuming >1 core). The specific RSX busy-spin
  tiles (`risk-0` / `me-*`) that peg cores 2/3 were down by the final
  check, but the box was still oversubscribed, so a pinned-core baseline
  would not be reproducible or directly comparable. Re-run on a quiet box
  and fill the table.
- **Reproduce:** `cargo bench -p rsx-matching` (all groups) or per file,
  e.g. `cargo bench -p rsx-matching --bench match_depth_bench`. Pins to
  core 2; ensure no busy-spin tile is pinned there.
- **Fixture design charges every point equally:** same harness, same core,
  same Criterion config, same non-draining best level for the depth and
  accept-path benches, so depth is the only variable in `match_by_depth`.
- **`fills/s == orders/s`** in `me_throughput` only because each accept
  does exactly one fill by construction; not a claim about multi-fill
  sweeps (see `sweep_n_levels`).

## Next (Phase 2)

Competitor baselines under a shared compare-harness (LMAX Disruptor-style
matcher, liquibook if feasible, naive matcher) — `rsx-matching/compare/`
with the `[lib]/[reimpl]/[pub]` provenance taxonomy. Not started.
