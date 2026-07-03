# Bug queue

The review queue: **OPEN** and **DEFERRED** items only. Resolved bugs live
in git (commit refs below) and `CHANGELOG.md` — not here.

## Status — 2026-05-30

**OPEN (triage):**
- **BOOK-BENCH-DEEP-PANIC** (LOW, bench) — `deep_book_bench` panics
  `assert!("slab exhausted")` at `slab.rs:39` during `cancel_depth` (≥10k) /
  `deep_flat` (1M/10M). Harness `build(n)` sizes the slab to `n+1024`, but the
  cancel_depth refill churn + big-N flat benches exceed it. Result: the depth
  curves, `match_by_depth`, and the depth-independence flatness proof (Phase 1's
  headline) were NOT captured. Fix: size the slab for churn/max-N in the harness
  (or bound the refill). Book Phase 1 numbers incomplete until fixed.
- **BOOK-BENCH-MICROOPS-OPTIMIZED** (LOW, bench) — several `book_bench` micro-ops
  measure implausible picoseconds/zero because the op is elided (missing
  `black_box`): `modify_order_qty_down` 0 ps, `slab_alloc_bump` 285 ps,
  `slab_alloc_from_freelist` 735 ps, `compression_price_to_index_*` ~460 ps.
  Also `match_single_fill` is mislabeled (5 µs — sweeps a 1k-ask book, not one
  54 ns fill) and `insert_depth` is inverted (pair measurement). Fix: `black_box`
  inputs/outputs + relabel/rescope. Quarantined in the 20260703 report.
- **MATCHING-BENCH-ORDERTYPE-FIXTURE** (LOW, bench) — `match_by_type_bench` +
  `match_n_levels_bench` measure 32–120 µs where the match algo is ~30 ns and a
  full single accept is 266 ns. `post_only_rest` (crosses nothing) at 69 µs is
  the tell: the `iter_batched` depth-10k book fixture's alloc/drop cost bleeds
  into the timed region (or, less likely, an O(depth) accept path — itself a
  finding). Order-type/sweep numbers quarantined in the 20260703 report, NOT
  cited as per-order-type latency. Fix: shallow-book fixture or exclude the
  fixture drop from timing, then re-run under the Phase-2 codex faithfulness
  audit. `match_by_depth` (~30ns flat) is unaffected + trusted.
- **RECORDER-ARCHIVE-UNBOUNDED** (MED) — `tmp/wal/archive` grew to **59 GB**
  during this session's demo runs (continuous maker quoting → BBO/fill records
  archived with no retention), filling the disk → ENOSPC that failed cluster
  stop/start via the playground API. WAL retention (`RETENTION_NS`=4h) covers
  the hot tier (`tmp/wal/pengu` stayed 39 MB), but the recorder's ARCHIVE
  stream has no rotation/GC — "ARCHIVE handles long-term durability" was taken
  literally as keep-forever. Fix: archive rotation/retention (size or age cap),
  or document that archive needs external lifecycle management + a dev cap.
  Cleared manually (find -delete + kill recorder to release the fd) → 100 G free.
- **BENCH-MOLD-SOUP-UNPINNED** (LOW, fairness) — `compare_moldudp64` +
  `compare_soupbintcp` never pin their threads (`TODO(pinning)` never done)
  while casting/raw-UDP/KCP/Aeron pin client→core2/echo→core3. Their numbers
  (mold 8.8µs, soup 11.2µs on 2026-07-03) are not strictly comparable. Fix in
  the uniform-harness refactor (.ship/31): shared harness pins all benches.
- **CLUSTER-HEALTH-ADDR-UNSET** (LOW) — the `start` spawn plan never sets
  `RSX_*_HEALTH_ADDR`, so no daemon /health/metrics server binds (only cast/
  WS/replication ports listen). Playground cast-flows gw/risk counters fall
  back to "—" (honest) instead of live numbers; /ready and HPA metrics also
  unavailable. Fix: set health addrs in the spawn plan. Found during ceo-eval.
- **PLAYGROUND-DOCS-SIDEBAR-TEST** (LOW, pre-existing) — `api_e2e_test.py::
  test_docs_has_sidebar` fails: GET `/docs/README` renders a client-side
  `marked.parse` loader whose HTML lacks `href="./"`/`/docs/` sidebar links
  the test asserts. Bare `/docs/README` (depth 1) likely skips the sidebar
  branch that `/docs/guide/README` (depth 2) hits. Pre-existing — the docs
  route is byte-identical to session start (cf79e1d) and predates the
  rsx-webui removal. Fix is either the route (sidebar for bare names) or the
  test (hit the depth-2 URL). Deferred to a playground pass.
- **MAKE-WAL-STALE-CRATE** (LOW) — `Makefile:300` `make wal` target runs
  `cargo test -p rsx-dxs`, but `rsx-dxs` was renamed to `rsx-cast` in May.
  The target errors ("package ID rsx-dxs did not match"). CLAUDE.md already
  documents `make wal` = `cargo test -p rsx-cast`. One-word fix; logged not
  patched per bug-triage protocol.
- **BENCH-QUINN-ACCEPT-BI** (LOW, *unmasked by the KCP fix*) — with KCP no
  longer aborting the run, `compare_all` now panics at
  `benches/compare_all.rs:356`: `srv_conn.accept_bi().await.unwrap()`. QUIC
  opens a bidirectional stream lazily — the client's `open_bi()` sends nothing
  on the wire until the first `write`, so the server's `accept_bi()` never sees
  the stream (resolves to a connection error / hang). The Quinn row was masked
  by the earlier KCP abort, so this has likely been broken since KCP regressed;
  the README "~37 µs" is last-measured 2026-05-24, not reproducible now. Fix:
  have the client write one priming byte after `open_bi()` before the server
  `accept_bi()`, or restructure the stream handshake. Bench-only. Flagged, not
  patched (separate from the KCP one-liner; QUIC stream-lifecycle surgery).
- **ME-FAULTED-NO-REPLAY-ADDR** (MED) — parallel load FAULTs the ME → panic;
  no replay source wired. Blocks parallel-load benchmarking. Detail below.
- **IOC-NOT-HONORED** (MED) — empty-book IOC rests instead of cancelling; `tif`
  lost on the GW→risk→ME propagation (matching core is correct). Detail below.
- **GATEWAY-LATENCY** (HIGH, *mitigated*) — egress-drain poll 10ms→500µs shipped
  (`5a578d3`); the zero-poll tile-split is the scale path, deferred per founder
  ("make the tile correct, don't split now"). Detail below.

**DEFERRED — book session** (founder: "solve once we're dealing with book"):
BOOK-SLAB-FREE-UNGUARDED, BOOK-STALE-HANDLE-REUSE, ME-REDUCEONLY-IOC-FILLEDQTY,
BOOK-FAR-PRICE-BUCKETING. Detail below. (BOOK-BBO-COMPRESSED-INDEX +
BOOK-SCAN-NEXT-BID-OFFBY were fixed 2026-07-03 — see git/CHANGELOG.)

**BY-DESIGN (no action):** RISK-FUNDING-CROSS-SHARD (global zero-sum not
guaranteed across shards; demo is single-shard), GW-SINGLE-SHARD-NO-ROUTING
(one risk sender, no `user_id % shard_count`; demo limit), ME-REPLAY-SKIPS-
DOWNSTREAM (each consumer recovers independently via its own replay).

---

## ME-FAULTED-NO-REPLAY-ADDR — ME FAULTED recovery has no replay source (MED)

**Status: OPEN.** Found 2026-05-30 during e2e re-measurement (parallel WS
workload). Under parallel load a single dropped UDP packet on loopback (seq gap,
e.g. 258→259) puts the ME's `CastReceiver` into FAULTED recovery. The ME then
panics for two stacked reasons: (1) `RSX_ME_REPLICATION_ADDR` is unset in the
ME spawn env — the `start` script gives that var to Risk (so Risk can replay
from ME's WAL on Risk-side FAULTED) but not to the ME; and (2) Risk exposes no
TCP replication server for the ME to pull from, so the ME's FAULTED→replay path
is unimplementable as wired even if the addr were set. Single-stream is
unaffected (no gaps). **Impact:** blocks any sustained parallel-load measurement
(the GW→ME→GW p50/p99 under load that PROGRESS lists as not-done). **Fix
sketch:** decide the ME's cold-path replay source (a Risk-side replication
server, or replay from the ME's own WAL tip), then wire the corresponding
`*_REPLICATION_ADDR` into the ME spawn env in `start`. Triage — design decision
first. Companion to per-consumer FAULTED recovery (only `rsx-matching` has the
POC path; risk/marketdata/gateway still panic).

## IOC-NOT-HONORED — IOC order with no liquidity rests instead of cancelling (MED)

**Status: OPEN.** A `{N:[10,0,1,100000,cid,1]}` (tif=1 = IOC) BUY submitted
against a confirmed-empty book returns status RESTING / OrderInserted — it
inserts into the book instead of immediately cancelling. Per
`rsx-book/src/matching.rs:188-199`, a `remaining_qty > 0` IOC must emit
`OrderDone` with `REASON_CANCELLED`. The matching **core is correct** (the
empty-book IOC test covers it, residual → `OrderDone` at `matching.rs:188`), so
the bug is in the GW→risk→ME *propagation* of `tif`: gateway parses arr[5]→tif
(`records.rs:200`), risk forwards `tif: order.tif` (`main.rs:1059`), ME converts
1→IOC (`wire.rs:36`) — yet the order rests, so a field is lost/defaulted to GTC
at a shard boundary. Narrows to wire decode/encode at GW→risk→ME. Repro:
empty-book IOC buy via WS. Triage only.

## GATEWAY-LATENCY — casting-recv poll-loop starvation dominates e2e (HIGH, mitigated)

**Status: MITIGATED (full fix deferred).** Single-order stage trace (live
cluster): the response left Risk by ~571µs (`me_out`) but the gateway didn't
receive it (`gateway_cast_recv`) until much later — the response sat in the
gateway UDP socket buffer waiting for the casting-recv poll loop to get a turn
on the shared monoio reactor (WS accept + per-conn handlers + casting-recv all
on one reactor). The egress-drain poll was tightened 10ms→500µs (`5a578d3` +
handler), which dropped WS single-stream p50 from 11.5ms to 2.25ms
(`reports/20260530_e2e-ws-latency.md`). **Remaining fix (deferred):** tile-split
the casting-recv response path to a dedicated pinned busy-spin thread (off the
reactor) → SPSC ring → WS writer tasks (same pattern as Risk/ME). Biggest single
e2e win; deferred per founder ("don't split now").

## Deferred book-session bugs (detail)

Founder: solve these when we next work the book. All verified against source;
`[V]` real, `[?]` needs one more check, `[D]` by-design/known-limitation.

- **BOOK-SLAB-FREE-UNGUARDED `[V]` (hardening).** `slab.rs:49` — `free()` accepts
  any in-bounds index → double-free / freelist cycle possible. Add a debug
  assert (`idx < bump_next` and not already free).
- **BOOK-STALE-HANDLE-REUSE `[?]`.** `book.rs:241` — `cancel_order` only checks
  `is_active()`; the slab reuses freed indices, so a stale handle could alias a
  reused slot. Safe only if the ME's `order_index` never retains a freed handle.
  Fix (defensive): generational handles or `(handle, order_id)` check.
- **ME-REDUCEONLY-IOC-FILLEDQTY `[?]`.** `rsx-book/src/matching.rs:190` —
  `filled = order.qty - order.remaining_qty` counts the reduce-only clamp as an
  execution, so an empty-book reduce-only IOC can report nonzero filled qty. Fix:
  compute fills from actually-matched qty, separate from the clamp.
- **BOOK-FAR-PRICE-BUCKETING `[D]`.** `compression.rs:48,118` — compression
  buckets far prices (10/100/1000 ticks per slot), so distinct prices share a
  level → price-time priority is coarse far from mid. Intentional compressed-book
  tradeoff; logged as a known design risk, not a defect.

## Dashboard stability + RSX process flapping (2026-05-31, task 29-#12)
- Playground dashboard runs as a SINGLE uvicorn worker (server.py:8127,
  workers=1 reload=False) with no self-watchdog. Any kill = full outage
  until manual `./playground start`. NOTE: several apparent "crashes" this
  session were self-inflicted (`pkill -f server.py` matched the running
  shell; `fuser -k 49171/tcp` deliberate) — the server is more stable than
  it appeared; the real gap is no supervisor.
- RSX processes flap (e.g. 4/7 running): auto-restart supervisor with a
  circuit-breaker opening after 5 crashes -> `blocked` (server.py:344-357).
  Root crash causes: Postgres down (10.0.2.1:5432 unreachable),
  marketdata rcvbuf -> FAULTED, maker-induced cast FAULTED (per MEMORY).
- FIX options (needs decision; edit blocked while overview agent owns
  server.py): (a) run dashboard under systemd/pm2 or a small watchdog;
  (b) bring Postgres up so PG-dependent processes stop crashing;
  (c) `make tune-host` (rmem_max) before enabling the auto-maker to avoid
  the rcvbuf FAULTED loop.

## Control Stop/Start buttons don't work (2026-05-31, task 29)
- Owner reports per-process Stop/Start (control grid + faults page) do
  nothing. Suspect: buttons post ./api/processes/{name}/{action} WITHOUT
  an x-confirm header (only the walkthrough all/start at pages.py:663 sends
  hx-headers x-confirm), while the all/* endpoints require check_confirm.
  Verify the {name}/{action} handler's confirm/run_id gate and either send
  the header from the buttons or drop the gate for single-process actions.
  Must be covered by the audit + Playwright play-tests.

  CORRECTION: the {name}/{action} handler (server.py:3942) needs only
  loopback (no confirm gate), so it's NOT a header issue. Likely cause:
  (a) the raw-PID stop fallback (3958-3972, hit when name not in `managed`
  after a dashboard restart drops the in-memory dict) SIGTERMs but does NOT
  mark intentional/update _restart_state, so the auto-restart watcher
  revives the process; and/or (b) no visible feedback. Fix: route single-
  process stop through the intentional-flag path (like stop_process) even
  on the raw-PID branch, and surface the result. Confirm by clicking Stop
  and watching whether the process reappears.
