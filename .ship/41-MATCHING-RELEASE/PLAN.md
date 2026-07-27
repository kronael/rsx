# 41 — rsx-matching release pass (study)

**Release here = the internal presentable pass, not crates.io.** Make the
READMEs/docs align, confirm it demos, then **review → refine → bench →
post**. No registry publishing; rsx-cast is untouched/frozen and irrelevant
to this pass.

This doc is the study: what's actually needed, with the gaps already found.
Work top-to-bottom; each step has a green-gate.

State today: `PROGRESS.md` marks rsx-matching **done**; root `README.md`
marks it **release candidate**. It has `README.md` + `ARCHITECTURE.md` +
6 benches + a dated bench report (`reports/20260703_matching-benches.md`).
The code is solid; the gaps are **doc-vs-doc number drift** and an
unrefreshed bench/demo/review loop — exactly a presentable pass, not a
rewrite.

---

## Step 1 — Docs align (the real work)

The headline latency number is stated **four different ways** across the
tree. This is the single biggest release blocker: a reader hits a different
number in every doc.

| doc | says | what it actually measures |
|---|---|---|
| `rsx-matching/README.md:9`, `ARCHITECTURE.md:61` | **266 ns** full accept, **3.6M orders/s** | dedup+WAL-accept+match+events+index, 1 fill |
| `rsx-matching/ARCHITECTURE.md:56`, `reports/20260703_matching-benches.md:100` | **~30 ns**, flat 1→100k depth | the match step alone |
| root `README.md:97` | **54 ns per fill** | stale (older `20260530_component-benches.md` figure) |
| `BUGS.md:14` | "a 60 ns match" | stale, in prose |

**Fix:** pick one current, dated, per-measurement story and make every doc
cite it with an explicit label — *match ~30 ns · full accept 266 ns ·
3.6M orders/s* — sourced to the latest bench report (refreshed in Step 4).
The rule is house-standard: authoritative doc wins, concepts mirror it.

Also open (from `BUGS.md` "stale latency/size numbers" section, record-only):

- **DOC-RT-FLOOR-DRIFT** — in-process round-trip floor stated 3 ways:
  `README.md:234` 7.5 µs p50 / 16.9 µs p99; `docs/benches.md:128` 9.58 µs;
  `reports/20260703_cast-benches.md:13` 8.8 µs (cast RTT *alone*, can't be
  below a full RT). Reconcile README ↔ benches.md ↔ report to one dated
  number. Touches matching's e2e story.
- **DOC-PRICELEVEL-24B-STALE** — `specs/2/21-orderbook.md:309` says
  `PriceLevel` 24 B; code is 32 B (`rsx-book/src/level.rs:20`). Book-owned
  but shows up in the matching depth narrative.
- **Maturity mismatch** — reconcile `PROGRESS.md` (**done**) vs root
  `README.md` component blurb (**release candidate**). Pick one and apply
  everywhere (PROGRESS, README, ARCHITECTURE).

**Surfaces to sweep for consistency:** `rsx-matching/README.md`,
`rsx-matching/ARCHITECTURE.md`, root `README.md` component section,
`PROGRESS.md`, `docs/concepts/tiles-and-pinning.md`,
`docs/concepts/05-slab-and-compression.md`, `specs/2/17-matching.md`,
`specs/2/21-orderbook.md`, `docs/benches.md`.

**Gate:** every latency/size/maturity claim about matching reads the same
across all nine surfaces, each labelled by what it measures, sourced to the
Step-4 report. Docs house rules: no history, no fluff, describe what-is.

## Step 2 — Demo works

Confirm the engine demos end-to-end and the speed story is honest.

- `scripts/demo-trade.sh` — start-all-minimal, maker+taker IOC pair, assert
  a fill lands in the ME WAL. Run it green.
- Playground book + the `⚡` speed surface (dashboard latency card is
  browser→proxy→gateway ms, **not** the µs internal number — keep that
  distinction; see `specs/2/22-perf-verification.md`).
- The `speed-demo` skill / `make local` + a taker as the manual walkthrough.

**Gate:** `demo-trade.sh` exits 0; the demo narrative cites the same
headline number as the docs, and never passes the browser-latency card off
as the engine's µs latency.

## Step 3 — Review

- Run `cto-eval` (adversarial) and/or `refine` on `rsx-matching/` — dead
  code, cross-boundary leaks, hot-path minimization, unsafe review.
- Triage `BUGS.md`. The one substantive open item is
  **RECENTER-EAGER-TAIL-SPIKE** (MED, latency/design): `maybe_recenter`
  migrates the whole book eagerly (O(old-slot-count ≈ 617k for BTC-PERP) —
  a bounded spike far above a 30 ns match, on the order that trips a >2.5%
  mid drift, i.e. during volatility). It is **not a correctness bug** — the
  eager migration preserves price-time priority; lazy would break crossing
  liquidity — and it's already documented honestly in
  `docs/concepts/05-slab-and-compression.md`. **Decision for release:**
  ship with it as a documented tradeoff (recommended) vs. gate release on
  the incremental-migration redesign (non-trivial ME change; needs design +
  adversarial review before code — out of scope for a presentable pass).

**Gate:** review findings recorded in this dir (`critique-*.md`); each is
fix-now, doc-as-tradeoff, or `BUGS.md`-defer with a reason. No silent
carry.

## Step 4 — Refine + bench

- Apply Step-1 doc fixes and any fix-now review findings. Keep `make lint`
  (clippy -D warnings), `make test`, `make fmt-check` green.
- Re-run the 6 benches: `matching_bench`, `process_order_bench`,
  `match_n_levels_bench`, `match_depth_bench`, `match_by_type_bench`,
  `wal_replay_bench` (`make perf`). Note the runners:
  `wal_replay_bench` + integration use **testcontainers (Docker/Postgres)**;
  accurate numbers need a **pinned core** (`core_affinity`,
  `RSX_ME_CORE_ID`) on a quiet box.
- `make bench-gate` (Criterion regression, >10% = fail) — confirm no
  regression vs the `tmp/` baseline.
- Write a fresh **`reports/YYYYMMDD_matching-benches.md`** (supersedes the
  2026-07-03 one): what was measured, the table, headline, caveats
  (in-process, single-box, Docker-gated benches). This report becomes the
  single source the Step-1 docs cite.

**Gate:** benches green, gate passes, new dated report committed, docs cite
it.

## Step 5 — Post

- The narrative/blog pass: a matching section in `BLOG.md` (or the
  presentable narrative), grounded in the refreshed report — the
  depth-independent match, the zero-alloc/pinned/busy-spin story, the honest
  266 ns accept.
- **External posting is founder-only** (no-publish rule): draft it, mark it
  "leave for the founder" — do not post to any external host.

**Gate:** narrative drafted and internally consistent with the docs; no
external publish action taken.

---

## Summary of concrete gaps (start here)

1. **Number drift** — 30 / 54 / 60 / 266 ns stated inconsistently; pick the
   dated per-measurement set, sweep 9 doc surfaces. *(Step 1 — main task)*
2. **Maturity mismatch** — PROGRESS "done" vs README "release candidate".
3. **Stale figures** — DOC-RT-FLOOR-DRIFT, DOC-PRICELEVEL-24B-STALE.
4. **Bench report stale** — refresh to a new dated report; make it the
   citation source.
5. **Demo** — run `demo-trade.sh` green; keep the µs-vs-ms honesty.
6. **Review** — cto-eval/refine; decide RECENTER-EAGER-TAIL-SPIKE
   (ship-as-documented-tradeoff recommended).
7. **Post** — draft narrative, leave external posting to the founder.

Nothing here is a code rewrite: the engine is done. The release pass is
doc-alignment + a fresh bench/demo/review loop.
