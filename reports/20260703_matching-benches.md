# rsx-matching benches — uniform harness baseline (Phase 1)

**Date:** 2026-07-03
**Crate:** `rsx-matching`
**Sprint:** `.ship/31-BOOK-MATCH-CAST-TREATMENT` Phase 1 (Measure)
**Status:** harness + bench set + numbers captured (cluster off; indicative
on a shared 4-core docker host). `match_by_depth` / dedup / WAL / accept are
trusted; the order-type + sweep figures were a depth-10k fixture artifact and
are excluded (see Numbers).

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

Captured 2026-07-03, RSX cluster **STOPPED** (busy-spin tiles off), `sample_size
50`, timed thread pinned to core 2. The box is a shared 4-core docker host with
residual docker load, so these are **indicative** p50s (robust over 50 samples),
not an isolated-box baseline — re-run on a quiet box for a citable baseline.
Grouped by how much I trust each figure.

### Trusted (clean, single-op or well-scoped)

| Point | p50 | Note |
|---|---|---|
| `match_by_depth/n=1` | **30.4 ns** | match algorithm only |
| `match_by_depth/n=100` | 30.8 ns | |
| `match_by_depth/n=1000` | 29.3 ns | |
| `match_by_depth/n=10000` | 32.7 ns | |
| `match_by_depth/n=100000` | **29.7 ns** | **depth-INDEPENDENT** |
| `me_accept_path/full` | **266 ns** | full `Me::accept` (dedup+match+buffered WAL+index), 1 fill |
| `me_throughput/orders` | 281 ns | ≈ **3.6M orders/s** (1 fill each) |
| `dedup/insert_new` | 147 ns | FxHashMap insert |
| `dedup/hit_duplicate` | **3.7 ns** | duplicate rejected |
| `dedup/cleanup_10k` | 522 µs | bulk 10k prune |
| `wal_events/append_1_fill` | 84 ns | serialize 1 fill (no fsync) |
| `wal_events/drain_10_fills` | 518 ns | |
| `wal_events/drain_100_fills` | 556 ns | |
| `wal_replay_30k_records` | 32.8 ms | ≈ 915k records/s cold replay |

**Headline:** the match itself is **~30 ns, flat across depth 1→100k**
(depth-independent, consistent with the 52 ns deep-book figure in
`20260530_component-benches.md`); a full single-order accept is **266 ns**;
duplicate rejection is **3.7 ns**.

### Order-type / multi-level-sweep: not measured here

The `match_by_order_type` and `sweep_n_levels` groups used a depth-10k
`iter_batched` fixture whose per-iteration allocate/drop cost bled into the
timed region, so their raw µs figures were the fixture's cost, not the
per-order-type dispatch or per-level sweep cost. They are excluded from this
report; a shallow-book (drop-excluded) rerun is needed before any order-type
latency is quoted. The trusted single-op floors below are unaffected.

## Caveats (honesty guardrails)

- **Single box, in-process microbench.** No UDP/WS/cross-process. These
  are compute floors; the full GW→ME→GW round-trip is transport-bound
  (~4 casting hops), not compute-bound — see `20260530_e2e-ws-probe.md`.
- **Indicative, not isolated-baseline.** Captured with the RSX cluster
  stopped (busy-spin tiles off cores 2/3), but on a shared 4-core docker
  host with residual load. p50 over 50 samples is robust for the trusted
  single-op figures. Re-run on a quiet box for a citable baseline.
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
