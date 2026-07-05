# Bug queue

The review queue: **OPEN** and **DEFERRED** items only. Resolved bugs live
in git (commit refs below) and `CHANGELOG.md` — not here.

## Status — 2026-05-30

**OPEN (triage):**
- **STARTUP-ORDERING-FRAGILITY** (MED, ops) — "can't start the system via the
  playground" traced to a chain of order-dependencies, none self-healing:
  (1) **Postgres must be up first** — if the `rsx-postgres` container is stopped/
  gone, risk-0 crash-loops "error connecting to server" and nothing comes up;
  the playground still reports "started 6 processes" (misleading). Mitigated:
  `docker update --restart unless-stopped rsx-postgres` so it survives.
  (2) **Playground manages the cluster in-memory** — restarting the playground
  server orphans the processes it spawned and they die; the new instance starts
  with an empty process table. (3) **Playground marketdata subscriber
  circuit-breaks** if marketdata isn't live when the playground starts ("md
  subscriber circuit open: N failures; pausing fan-out") and does NOT
  auto-recover → `_book_snap` stays empty → orders can't price → rest/reject.
  (4) Fresh PG needs `seed_accounts` (runs at playground startup) + a risk
  restart to load them. (5) `/api/risk/users/N` mislabels an empty-positions
  result as "no postgres connection". Correct bring-up order: PG → playground
  server → cluster (start-all) → maker → deposits. Fix: a supervised start
  sequence (or make PG a compose service + the playground reconnect its md
  subscriber + not lose the process table on restart).

- **MATCHING-BENCH-ORDERTYPE-FIXTURE** (LOW, bench) — **Status: FIXED
  2026-07-04 (`da9a2b4`).** `match_by_type` (`ioc_full`, `gtc_full_cross`,
  `sweep_10_levels`) measured 32–120 µs where the match algo is ~30-60 ns. The
  original triage guessed "`iter_batched` fixture alloc/drop bleed" — WRONG.
  The real cause: `taker_fill` clears the touch level on every call (unlike
  `match_ioc_vs_1k_asks`'s replenish-before-clear pattern), and `scan_next_bid`/
  `scan_next_ask` were an O(compression-slots) linear scan that only ran when
  a level actually emptied — so any bench whose op clears a level paid the
  full ~100k-slot scan, while `post_only_reject` (crosses nothing, never
  clears a level, 6 ns on the SAME depth-10k fixture) proved the fixture
  itself was cheap. Fixed by `da9a2b4`: hierarchical occupancy bitmap
  (`rsx-book/src/occupancy.rs`), O(depth=3) find-next/find-prev. Confirmed
  2026-07-04 re-run: `match_ioc_vs_1k_asks` 4.37µs→145ns, `match_by_type/
  ioc_full` ~80µs→79ns, `match_by_type/sweep_10_levels` ~1ms→700ns. See
  `reports/20260704_book-bench.md` "post-scan-fix" section. `match_by_depth`
  (~60ns flat, never clears the touch level) was correctly unaffected all
  along.
- **FOK-AVAILABLE-LIQUIDITY-ON-SCAN** (MED, bench) — **Status: FIXED
  2026-07-04.** `match_by_type/fok_full` was ~296 µs after the occupancy-
  bitmap fix (`da9a2b4`), unlike every other order type (60-145 ns). Cause:
  the old `available_liquidity` was a SEPARATE O(N-resting-orders) full-book
  scan run on FOK's hot path — it walked every one of the ~100k active
  levels and every order on each, summing crossable qty BEFORE matching.
  Fix: no new structure. FOK is just "try to match it, take it or don't", so
  `can_fill_fully` (`rsx-book/src/matching.rs`) now walks only the *crossing*
  levels in price order — the same traversal a match performs, via the
  book's existing best-level index — summing each level's already-maintained
  `total_qty` and stopping the instant the running total reaches the order
  size. O(levels crossed, early-exit) instead of O(slots + orders); a whole
  level shares one price so `total_qty` counts it exactly (no per-order
  walk). Pinned to brute-force by `tests/fok_liquidity_test.rs` (3000 FOK
  probes over multi-zone random flow). Post-fix `fok_full`: ~118 ns
  (was ~296 µs, −99.95% per Criterion) — now in the same band as every
  other order type. (Number from a lightly-contended box; clean re-run
  pending, but the 300x magnitude is unambiguous.)
- **RECORDER-ARCHIVE-DEV-DISK** (MED, *reframed 2026-07-04*) — the recorder
  archives the FULL ME WAL stream (every order/fill/BBO/done record, verbatim)
  to `tmp/wal/archive/<sid>/<sid>_<date>.wal` as the permanent system-of-record.
  **Unbounded is BY DESIGN** — this is the exchange's audit trail + replay-from-
  genesis tier ("ARCHIVE handles long-term durability", hot tier keeps only 4h).
  Do NOT add retention/GC that deletes records — that destroys the point.
  The actual defect is PLACEMENT: it writes to the local dev root (237 GB) with
  no offload, so continuous maker quoting grew it to 59 GB and ENOSPC'd the box
  (killed a subagent + failed cluster stop/start) twice this session. Fix is
  storage, not deletion: put the archive on a separate/dedicated volume, offload
  to object storage (S3/GCS) with local pruning of already-offloaded segments,
  and/or a DEV-ONLY guard (cap or recorder-off in the playground) so the dev box
  can't fill. Currently mitigated by keeping the recorder stopped in dev.
  Cleared manually (find -delete + kill recorder to release the fd) → 85 G free.
- **BENCH-MOLD-SOUP-UNPINNED** (LOW, fairness) — `compare_moldudp64` +
  `compare_soupbintcp` never pin their threads (`TODO(pinning)` never done)
  while casting/raw-UDP/KCP/Aeron pin client→core2/echo→core3. Their numbers
  (mold 8.8µs, soup 11.2µs on 2026-07-03) are not strictly comparable. Fix in
  the uniform-harness refactor (.ship/31): shared harness pins all benches.

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
- **GATEWAY-LATENCY** (MED, *readiness fix landed*) — the cast-recv yield-spin is
  gone: the gateway now awaits io_uring readiness on the CastReceiver fd
  (`946b71d` + `7454187` exposes the fd), event-driven not polled. Earlier
  500µs conn-side egress poll (`5a578d3`) may remain; re-measure with
  `ws_order_latency` on a QUIET box to confirm the win + whether the conn poll
  still shows. Detail below.

**FIXED 2026-07-04** (detail sections below): ME-FAULTED-NO-REPLAY-ADDR,
IOC-NOT-HONORED.

**NEW 2026-07-04** (sonnet bug-hunt, all verified genuine — detail at end):
COMPRESSION-ZONE-TICK-UNIT (HIGH, latent), WAL-ROTATE-PREWRITE-MISLABEL (HIGH),
CLI-PTR-READ-UNALIGNED-UB (HIGH), CAST-SEND-RING-TOO-SMALL (MED),
GW-CANCEL-NO-RATELIMIT (MED), GW-CANCEL-NOT-USER-SCOPED (MED),
RISK-DEDUCT-FEE-UNCHECKED (LOW), MARK-PARSE-NEG-ZERO (LOW).

**NEW 2026-07-04** (T4 `.ship/33-TUI-SPEED-TESTS`, `rsx-tui/tests/
e2e_guarantees.rs` fixture debugging — detail at end): VERIFY-WAL-FILLS-
ALWAYS-ZERO (LOW), DEMO-TRADE-SUBMIT-ORDER-404 (MED).

**DEFERRED — book session** (founder: "solve once we're dealing with book"):
BOOK-FAR-PRICE-BUCKETING (`[D]` by-design, no action). Detail below.
**FIXED 2026-07-04** (book session): FOK-RESTS-IN-COMPRESSED-ZONES (new,
HIGH), BOOK-SLAB-FREE-UNGUARDED, BOOK-STALE-HANDLE-REUSE,
ME-REDUCEONLY-IOC-FILLEDQTY — detail below. (BOOK-BBO-COMPRESSED-INDEX +
BOOK-SCAN-NEXT-BID-OFFBY were fixed 2026-07-03 — see git/CHANGELOG.)

**BY-DESIGN (no action):** RISK-FUNDING-CROSS-SHARD (global zero-sum not
guaranteed across shards; demo is single-shard), GW-SINGLE-SHARD-NO-ROUTING
(one risk sender, no `user_id % shard_count`; demo limit), ME-REPLAY-SKIPS-
DOWNSTREAM (each consumer recovers independently via its own replay).

---

## ME-FAULTED-NO-REPLAY-ADDR — ME panicked on a dropped-packet order gap (MED)

**Status: FIXED 2026-07-04.** Resolved by the founder-blessed fault model:
the risk→ME **order** stream is drop-safe, so on FAULTED the ME now **skips the
gap and resumes live** rather than replay-or-panic. Rationale: a dropped
pre-ack order is re-sent by the client (no-ack-within-timeout,
`specs/2/49-webproto.md`) and deduped on the ME's WAL (`RECORD_ORDER_ACCEPTED`)
= exactly-once; and the ME re-sequences on output (its own WAL seq), so an
inbound gap is never an output gap (risk/recorder/marketdata see a contiguous
ME stream). The FAULTED handler (`rsx-matching/src/main.rs`) now counts skipped
seqs into `gauges.drops`, WARNs the gap range, and calls
`reset_after_replay(gap_end_inclusive)`. The dead consumer-side replay path
(`RSX_ME_REPLICATION_ADDR` + `drain_dxs_replay_into_book` + `apply_replayed_
record` + `replay_after_fault_test`) was removed (306-line `replay.rs` +
its test). **Still in place (different, still-required):** the ME's WAL
replication *server* (`RSX_ME_REPLICATION_BIND_ADDR`) that RISK pulls for
**fill** recovery, and ME cold-start replay from its own local WAL. Found
2026-05-30 under parallel-load e2e (single dropped UDP packet → FAULTED →
panic because `RSX_ME_REPLICATION_ADDR` was unset). Note: risk/marketdata/
gateway consumers still panic on FAULTED — those are separate streams with
their own recovery needs, out of scope here.

## IOC-NOT-HONORED — cancelled IOC surfaced to client as "resting" (MED)

**Status: FIXED 2026-07-04.** Root cause was NOT tif propagation (the original
triage guessed that and was wrong). Verified on the live cluster with a runtime
trace: `tif=1` reaches `risk_in` AND `me_in` intact, and the matching engine
**correctly cancels** the residual IOC (`rsx-book/src/matching.rs` residual
branch fires → `OrderDone { reason: REASON_CANCELLED }`). The order does NOT
actually rest in the book. The bug was a code-space collision: the ME wrote
`OrderDoneRecord.final_status = reason` (raw matching reason, `REASON_CANCELLED
= 1`), but per `specs/2/49-webproto.md` `final_status` is a webproto U-frame
status where **1 = RESTING, 2 = CANCELLED**. So a cancelled IOC's `final_status
= 1` was forwarded by the gateway as status 1 → the client saw "resting".
`REASON_FILLED = 0` happened to equal webproto FILLED(0), so fills looked fine
and hid the collision. **Fix:** `rsx-matching/src/wal_integration.rs` now
translates the matching reason → webproto status (`done_final_status`:
CANCELLED→2, FILLED→0) at both OrderDone-build sites; the gateway
`route_order_done` mapping was already spec-correct. Regression:
`rsx-matching` `tests/wal_integration_test.rs::
ioc_cancel_final_status_is_webproto_cancelled`. Live-verified: an empty-book /
non-crossing IOC now reports "cancelled" (fills still "filled", resting GTC
still "resting").

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

- **FOK-RESTS-IN-COMPRESSED-ZONES (HIGH, latent correctness).** Status: FIXED
  2026-07-04. `can_fill_fully` (`matching.rs`) summed a whole level's
  `total_qty` but tested only the HEAD order's price against the taker limit. In
  a compressed zone (≥1: 10/100/1000 ticks/slot, plus the zone-4 catch-all) a
  level holds orders at DISTINCT raw prices, so it over-counted crossable
  liquidity → FOK feasibility passed → `match_at_level` skipped the non-crossing
  makers → residual > 0 → and the residual branch only cancelled IOC, so the FOK
  fell through to `insert_resting` and RESTED (all-or-nothing violation, with a
  partial fill). Two-part fix: (a) `can_fill_fully` now walks the level's orders
  and sums only qty whose ACTUAL raw price crosses the limit whenever the slot
  is in a compressed zone (`t >= zone_slots[0]`); zone 0 keeps the O(1)
  `total_qty` shortcut (single price per slot, the near-BBO happy path), and
  early-exit is preserved (stop once a whole band sits beyond the limit). (b)
  the residual branch now rejects FOK (`OrderFailed(FAIL_FOK)`, book untouched)
  instead of resting — defense in depth behind the now-exact pre-check
  (`debug_assert!(false)` guards the can't-happen residual). Regression:
  `tests/fok_liquidity_test.rs` (`fok_compressed_zone_insufficient_true_
  liquidity_rejected`, `_tick50_insufficient_rejected`, `_sufficient_liquidity_
  fills`), each fails on the old `matching.rs`.
- **BOOK-SLAB-FREE-UNGUARDED `[V]` (hardening).** Status: FIXED 2026-07-04.
  `slab.free` now `debug_assert`s `idx < bump_next` (never-allocated) and
  `!is_free(idx)` (already on the freelist) — a double-free / freelist cycle is
  caught in debug rather than aliasing a slot handed out twice. O(free)
  is-free walk stays behind `debug_assert` (off in release; the ME bounds open
  orders upstream). Regression: `tests/slab_test.rs::slab_double_free_panics`,
  `slab_free_never_allocated_panics`.
- **BOOK-STALE-HANDLE-REUSE `[?]`.** Status: FIXED 2026-07-04 (defensive).
  Full generational handles would ripple the `u32` handle meaning across
  matching/events/index (too invasive), so instead: added
  `Orderbook::cancel_order_checked(handle, user_id, order_id_hi, order_id_lo)`
  which re-checks the slot's identity before cancelling (returns false, book
  untouched, on a reused/inactive slot), and documented on `cancel_order` the
  exact cross-crate invariant rsx-matching must uphold (verify identity, or use
  the checked path). rsx-matching's user-cancel path already does this drift
  check inline (`main.rs:1002-1017`); the WAL-replay path trusts its own
  `order_index`. Regression: `tests/book_test.rs::
  cancel_order_checked_rejects_stale_handle`.
- **ME-REDUCEONLY-IOC-FILLEDQTY `[?]`.** Status: FIXED 2026-07-04.
  `matching.rs` computed `filled = order.qty - order.remaining_qty`, counting
  the reduce-only clamp (remaining clamped down to the position) as execution —
  an empty-book reduce-only IOC with qty > position reported `filled = qty -
  position` with zero real fills. Fix: capture `fillable = order.remaining_qty`
  AFTER the clamp (before matching) and measure fills against it at every
  terminal site (IOC residual, FILLED). Regression:
  `tests/matching_test.rs::reduce_only_ioc_empty_book_reports_zero_filled`
  (old code reported filled=70, fixed reports 0).
- **BOOK-FAR-PRICE-BUCKETING `[D]`.** `compression.rs:48,118` — compression
  buckets far prices (10/100/1000 ticks per slot), so distinct prices share a
  level → price-time priority is coarse far from mid. Intentional compressed-book
  tradeoff; logged as a known design risk, not a defect.

### MIGRATE-SKIPS-NEW-MID-LEVEL — order resting at new_mid orphaned on recenter (HIGH)
**Status: FIXED 2026-07-04.** `trigger_recenter` now migrates the `new_mid`
level once, up front (no-op if empty, within-frontier so never migrated twice),
so an order resting exactly at new_mid survives recenter. Covers both the lazy
(`advance_frontier_to`) and eager (`migrate_batch`) paths — new_mid is empty in
`old_levels` by the time either runs. Regression: the adversarial tick-size test
now recenters ONTO a resting level (`new_mid = mid + 505*tick`) and asserts slab
no-leak. Found by the distribution tests. Original triage below.

**[Original — OPEN]** `migration.rs` —
`trigger_recenter` sets `bid_frontier = ask_frontier = new_mid`, then both
`migrate_batch` and `advance_frontier_to` (lazy path) step the frontier AWAY
from `new_mid` BEFORE migrating (bid: `bid_frontier -= tick` then migrate; ask:
`ask_frontier += tick` then migrate). So the OLD level covering exactly
`new_mid` is never visited — any order resting at `new_mid` is left in
`old_levels` and dropped when `complete_migration` clears them. Loss is silent:
one live order vanishes → violates invariant #8 (slab no-leak: allocated != free
+ active) and invariant #4 (position = sum of fills) downstream. Reproduced for
tick_size ∈ {1,10,50}: rest a fat book, recenter to a `new_mid` that coincides
with a resting level, migrate fully → exactly one order missing (len 800, free
40, active 759). Not triggered when `new_mid` falls between levels (the common
case), which is why it stayed latent. Fix: migrate the `new_mid` level once at
`trigger_recenter` (or seed the frontier so the first step includes `new_mid` —
e.g. migrate before decrement/increment). The distribution recenter test
(`rsx-book/tests/distribution_test.rs`) sidesteps it by recentering off a level;
remove that workaround once fixed.

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

## Sonnet bug-hunt findings — 2026-07-04 (all verified genuine)

Four read-only sonnet hunters over the whole tree; every finding below was
re-verified against source by the main agent. Recorded, not fixed (bug-triage
protocol — awaiting prioritization).

### COMPRESSION-ZONE-TICK-UNIT — zones mis-sized for tick_size != 1 (HIGH, latent)
**Status: FIXED 2026-07-04 (`9089e50`).** `CompressionMap::new` now stores zone
thresholds as RAW-price distances (matching `price_to_index`'s `price -
mid_price`), and converts to slot counts by dividing by `compression_ticks *
tick_size` — so a 5% move lands in the right zone for any tick_size, not the
2-slot catch-all. Verified: `tick50_five_pct_lands_in_zone0_not_catchall`,
`tick50_zone0_is_one_tick_per_slot`, `tick_size_stored_and_thresholds_raw`,
`adversarial_tick_sizes_matching_and_recenter` (tick 1/10/50 through real
matching + recenter). `migration.rs::should_recenter` checked too. Original
triage below.

**[Original triage — OPEN]** `rsx-book/src/compression.rs:29-45` computes zone thresholds
in TICK units (`pct_5 = mid*5/(100*tick_size)`, comment line 24 "…/ tick_size
ticks"), but `price_to_index` (`compression.rs:84-85`) compares them against a
RAW-price distance (`distance = price - mid_price`, never divided by
tick_size). For `tick_size=1` they coincide (every test/bench uses tick_size=1,
masking it). For `tick_size != 1` (BTC=50, ETH=10 in the symbol config) the
units diverge: a price only 5% from mid lands in the 2-slot zone-4 catch-all →
most of the book collapses into shared price levels → price-time priority
broken, wrong fills. Same mismatch reachable via `migration.rs:16-22`
(`should_recenter`) and recentering. Fix: divide `distance` by `tick_size` (or
store raw-price thresholds). Latent because the demo trades PENGU (tick=1).

### WAL-ROTATE-PREWRITE-MISLABEL — boundary records unreachable for NAK/replay (HIGH)
**Status: OPEN (rsx-cast bugfix candidate).** `rsx-cast/src/wal.rs:220-224` the
pre-write rotation fires before `write_all`, but `rotate()` (line 252-256) names
the old file with `self.last_seq`, which `append_framed` (line 197) already
advanced to include the still-buffered (unwritten) records. So the old file is
labeled `[first_seq..last_seq]` while physically holding only up to the
previously-flushed seq; the buffered records then go to the NEW file whose
`first_seq` is set to `next_seq` (line 282) — above them. Those boundary records
become unreachable via `read_record_at_seq` (NAK retransmit) and
`open_from_seq` (TCP replication catch-up) → both silently return nothing for a
record that exists. Pre-write is the PRIMARY rotation path (same threshold as
post-write, fires first); the existing `write_rotate_read_across_files` test only
hits the correct post-write path (file_size==0 on its single flush). Fix: label
rotate with the last seq actually written to that file, not `self.last_seq`.

### CAST-SEND-RING-TOO-SMALL — NAK fast-path dead for hot 128B records (MED)
**Status: OPEN (rsx-cast bugfix candidate).** `rsx-cast/src/cast.rs:77`
`SEND_RING_FRAME_BYTES=128`, but `FillRecord`/`BboRecord`/`OrderAcceptedRecord`
are 128-byte payloads (rsx-messages asserts) → total 16+128=144 > 128 → the send
path (cast.rs:261/318) takes the "large record" branch that zeroes the ring slot
(never caches). Every NAK for those (the 3 most-sent record types) misses the
in-memory ring and falls to a disk `read_record_at_seq`. The constant's comment
(cast.rs:72-77 "all <= 64 bytes payload … with headroom") is stale/false. Not a
correctness loss (disk fallback still works, modulo the WAL-ROTATE bug) but the
documented fast-path recovery cache is silently inert for the hottest traffic.
Fix: size the ring frame to cover current records (>=144).

### CLI-PTR-READ-UNALIGNED-UB — aligned ptr::read on unaligned WAL buffer (HIGH, soundness)
**Status: OPEN.** `rsx-cli/src/main.rs:343` (and every decode arm: 381,408,439,
468,499,519,537,574,595,619,643,674,700) uses `std::ptr::read(payload.as_ptr()
as *const _)` on `#[repr(C, align(64))]` records read from a `Vec<u8>` payload
(heap alloc, 8–16-byte aligned) — `ptr::read` requires the source satisfy the
type's alignment; violating it is UB. The canonical decoder
`rsx-cast/src/encode_utils.rs:53-55` uses `std::ptr::read_unaligned` for exactly
this reason; the CLI hand-rolled the decode with the wrong primitive. Can
produce garbage fields / crash under non-16-aligning allocators or higher opt
levels, undermining the WAL-inspection tool. Fix: `read_unaligned`.

### GW-CANCEL-NO-RATELIMIT — Cancel bypasses rate limit + circuit breaker (MED)
**Status: OPEN.** `rsx-gateway/src/handler.rs:547-649` the `WsFrame::Cancel` arm
calls none of `ip_limiter`, per-user `RateLimiter`, or `circuit.allow()` — unlike
the `NewOrder` arm (314-364). A client can flood `{"C":[...]}` frames straight to
risk/ME casting with zero throttling, even while throttled/tripped on the order
path. DoS gap on the casting channel. Fix: gate Cancel through the same limiters.

### GW-CANCEL-NOT-USER-SCOPED — cid collision breaks self-cancel (MED)
**Status: OPEN.** `rsx-gateway/src/handler.rs:580,618` `find_by_order_id` /
`find_by_client_order_id` scan the process-global `pending` queue with NO user
filter, and `build_cancel` (584,622) sends the requester's `user_id` + the found
order's id. cid is client-chosen and unnamespaced, so two users can collide:
user A's self-cancel-by-cid can find user B's same-cid order first → ME composite
key `(A, B_oid)` misses → silent no-op → A cannot cancel its own resting order.
Unauthorized cancel of B is blocked by the ME key (incidental), but the missing
user-scope + self-cancel breakage are real. Fix: scope the pending lookup to
`user_id`.

### RISK-DEDUCT-FEE-UNCHECKED — lone non-saturating ledger op (LOW)
**Status: OPEN.** `rsx-risk/src/account.rs:22` `deduct_fee` uses `self.collateral
-= fee` while every other money op in the crate uses saturating/i128-widened
arithmetic. If `collateral` and `fee` sit at opposite i64 extremes it overflows
(debug panic = DoS; release wrap = manufactured solvency). Reaching it needs
absurd values that order-entry notional caps prevent — latent consistency /
defense-in-depth gap, not live-impact. Fix: `saturating_sub`.

### MARK-PARSE-NEG-ZERO — sign lost for "-0.x" price strings (LOW, dead on real feeds)
**Status: OPEN.** `rsx-mark/src/source.rs:275-303` `parse_price("-0.5")`: `whole
= "-0"` parses to `0`, so `whole_val == 0` takes the add branch → `+frac` instead
of `-frac` (sign flip). Dead on real CEX spot feeds (never negative); flagged as
a latent edge only. Fix: track the sign separately from `whole_val`.

### LATENCY-TRACE-STAGES-AGGREGATION — mixed-population median subtraction (MED)
**Status: FIXED 2026-07-04 (`8b33bbc`).** My initial diagnosis was WRONG: I
claimed the return leg (`risk_out`/`gateway_out`) never emits and that taker
completions bypass `route_fill`. An opus trace disproved both — 9/9 spaced taker
FILLS emit `risk_out` (risk main.rs:560) AND `gateway_out` (route.rs:68 via the
real fill path ME→risk RECORD_FILL→forward_to_gw→gateway route_fill), all
anchored on the shared t0. The "zero samples" I saw were: (a) my probes RESTED
(a resting limit / IOC-cancel emits no Fill → correctly no egress sample), and
(b) the **real bug**: `/api/latency-stages` took independent per-stage medians
across a MIXED oid population then subtracted them — cold/rested orders carry
only forward stages (huge cold `me_in`≈5200µs), filled takers carry the return
leg, so subtracting medians from different oid sets yielded a meaningless egress
delta (`risk_out` clamped to 0, `me_out` polluted to ~5ms). Fix (Python only,
server.py `_segment_deltas`/`_cumulative_from_deltas`): each segment's median is
computed over exactly the oids carrying BOTH its endpoints. Sparse capture was
the token-bucket rate limiter (state.rs:69, cap 10 / 1 token per 100ms) dropping
rapid probes before `gateway_in` — space probes ≥1s. **Verified egress**
`me_out→gateway_out` per-oid: `[35,41,43,45,231,1846,2662,3437,6541]µs` —
BIMODAL: fast ~35-45µs (gateway spinning) + ms-scale parked tail (the
POLL_ADD-relevant signal). Caveat: verified by running the exact new computation
against real logs (the live playground couldn't be reloaded without risking the
child trace daemons). **Remaining (deferred, wire change):** non-fill records
(ORDER_INSERTED/DONE/CANCELLED/FAILED) carry only ME-emit `ts_ns`, not an origin
timestamp, so they can't compose into the forward-leg profile without adding an
origin-ts field to rsx-messages + WAL + parser; only the taker-fill leg
(`FillRecord.taker_ts_ns`) composes today. GATEWAY-LATENCY egress is now
measurable for fills; the bimodal tail is the next thing to chase.

## RETURN-PATH-INTERMITTENT-DROP — was a test-fixture bug, not a gateway/risk drop (RESOLVED)

**Status: RESOLVED 2026-07-04.** Root cause was a TEST FIXTURE bug, NOT a
gateway/risk/casting defect. `cluster::seed_book` posted a maker BUY @ 60_000,
but the shared long-lived book (rebuilt from WAL, never reset) already carries
resting asks ~50_000 — so the "maker" CROSSED those asks and filled instead of
resting as a bid. The crossing taker then had no resting bid to hit → no fill →
`wait_for(fills==1)` timed out, which read as a dropped return-leg. Fixed by
seeding the maker BELOW the asks (49_000) with qty matched to the taker so it
rests as the best bid and self-cleans on the fill (`rsx-tui/tests/support/
cluster.rs` + `e2e_orders.rs` price bands all <50_000 + a `LIVE_BOOK` mutex
serialising the two book-sharing fill tests). Verified: `submit_ioc_fills` +
`order_lifecycle_accepted_then_done` PASS against the live cluster. The
mid-investigation "confirmed real, persistent-WS-specific" call was WRONG — the
persistent-vs-transient difference was a coincidence (the playground probe used
a correct maker-sell/taker-buy cross; the e2e used a broken maker-buy).
Residual, NOT this bug: (a) casting is UDP so an occasional order/event
genuinely drops by design (`rsx-matching` FAULTED, "clients re-send dropped
pre-ack orders") — the tests carry a resubmit-once retry; (b) test-infra: the
long-lived shared book means a mid-way-failed run can leave a resting bid that
pollutes the next run's level — matched-qty self-cleaning avoids this in steady
state; a fresh cluster is pollution-free. Original (wrong) triage below.

**[SUPERSEDED — original triage guessed the root cause wrong]** Found writing `.ship/33-TUI-SPEED-TESTS` T3
(`rsx-tui/tests/e2e_orders.rs`) against the live minimal cluster
(gw-0/risk-0/me-pengu, symbol_id=10). Repro: two separate WS connections
(distinct seeded `user_id`s, e.g. 2 and 3), maker posts a resting GTC that
rests fine (`me-pengu.log` `me_in..me_out` completes, gateway relays the
`U` status=1 accept — confirmed working every time), then a second
connection submits a lot-aligned crossing IOC. `me-pengu.log` shows the
crossing order fully processed (`me_in -> me_dedup_done ->
me_wal_accepted_done -> me_match_done -> me_wal_events_done ->
me_index_done -> me_out` all present — ME believes it matched and emitted
Fill+Done), but `risk-0.log` shows only the inbound `risk_in` for that
oid and nothing after (no `risk_out`/`risk_cast_send_done`), and
`gw-0.log` shows only `gateway_in`, no `gateway_cast_recv` — the taker's
own `WsConn` never receives a `Fill` or terminal `U` frame and hangs until
test timeout. Independently reproduced outside the Rust suite with raw
`wscat` sessions against the same live cluster (see repro commands in the
session that filed this entry): a first order on a fresh connection got
no reply within 2s; an identical resubmit (same price, fresh cid, new
connection) was acked within the same short window. Ruled out: not
`InsufficientMargin` (all repros use `_SEED_USERS` with ample collateral),
not tick/lot misalignment (validated multiples of `lot=100000`), not the
gateway's `NewOrder` rate limiter (that path `send_error`s explicitly —
code 1006 "rate limited" — before minting an oid or logging
`gateway_in`; every failing oid here already has a `gateway_in` line, so
it passed rate-limit/circuit checks). Root cause not isolated — candidates
worth checking: WAL dedup replay on a resubmitted cid returning the
original accept silently without re-emitting a cast event; the in-progress
"risk return path RESPEC'd → ME→GW-direct" migration noted in project
memory (partial, not fully implemented) leaving the old ME→Risk→GW leg
half-wired for the fill/done case specifically (accept-path via risk
still works; fill-path does not); or plain casting/UDP loss on the
ME→Risk leg specifically for fills under the concurrent multi-connection
load this test suite generates. `rsx-tui/tests/e2e_orders.rs`'s
`submit_ioc_fills`/`order_lifecycle_accepted_then_done` work around this
with a resubmit-once retry (matching `rsx-matching`'s own documented
mitigation, "clients re-send dropped pre-ack orders (WAL dedup =
exactly-once)"), but even that isn't always enough — both tests can still
fail against this cluster instance. Not a T3 test-file defect; do not
"fix" by weakening the test assertions.

- **Severity:** high
- **Scope:** rsx-risk / rsx-matching / rsx-gateway return-path (ME→Risk→GW)
- **Affected:** fill/done confirmation delivery to the ordering client
- **Source:** `log/me-pengu.log`, `log/risk-0.log`, `log/gw-0.log` around
  2026-07-04T11:08-11:13 (oids `019f2cd110b679719c02d72391586007`,
  `019f2cd4930f7121bea15cf37753ad93`, `019f2cd4c6ad7cd1829930d0494aa843`,
  `019f2cd4da3b7ef3aea4fbf33401b3fa` and others); see also `.ship/
  33-TUI-SPEED-TESTS` session transcript for the raw `wscat` repro
- **Status:** open
- **Fix:** —

## VERIFY-WAL-FILLS-ALWAYS-ZERO — playground /api/verify never sees real WAL fills (LOW)

**Status: OPEN.** `_run_invariant_checks`'s "Fills precede ORDER_DONE (per
order)" check (`server.py:4631-4689`, `_wal_stream_dirs()` scan) reports
`"WAL fills=0 but session fills=183 — sources disagree"` (status `fail`)
even immediately after a real fill was driven through the gateway and
confirmed on the wire (`GwEvent::Fill` observed by an `rsx-tui` `WsConn`
client, T4 `.ship/33-TUI-SPEED-TESTS`). The ME's actual WAL directory is
`RSX_ME_WAL_DIR=./tmp/wal/pengu` (confirmed via `/proc/<me-pid>/environ`
+ `find`, landing at `tmp/wal/pengu/10/10_active.wal`, which does grow on
fills), but the playground's own WAL-dir resolution apparently looks
elsewhere and finds nothing, permanently reporting 0. The "session fills"
counter (Python-local, 183 in the same run) only counts orders submitted
through the playground's own REST endpoints, not real WAL state, so it
isn't a substitute either — net effect, this check is not a usable fill-
durability oracle for orders submitted via any route today.

- **Severity:** low
- **Scope:** rsx-playground/server.py `_run_invariant_checks`
- **Affected:** `/api/verify`, `/api/verify/run-json`, `/verify` page
- **Source:** rsx-playground/server.py:4631-4689; observed via
  `rsx-tui/tests/e2e_guarantees.rs`'s `fill_durability_recorded_in_wal`
  test, which works around it by reading the ME's active WAL file size
  directly instead of this endpoint.
- **Status:** open
- **Fix:** —

## DEMO-TRADE-SUBMIT-ORDER-404 — scripts/demo-trade.sh posts to a route that no longer exists (MED)

**Status: OPEN.** `scripts/demo-trade.sh` submits its maker/taker demo
pair via `curl -sf -X POST "${PLAYGROUND}/api/submit-order" ...`, but no
such route exists in `rsx-playground/server.py` today (`@app.post("/api`
shows `/api/orders/test`, `/api/orders/quick`, `/api/orders/random`,
`/api/orders/batch`, `/api/orders/{cid}/cancel` — no bare
`/api/submit-order`). Live probe: `curl -s -w '%{http_code}'` against
that exact path returns `404 {"detail":"Not Found"}`. The script's
`curl -sf` swallows the 404 silently and falls back to `echo "{}"`, so
its maker/taker submissions are silent no-ops; its actual pass/fail
signal comes only from the later WAL-file-growth poll, which happens to
still pass if something else already crossed the book on the shared
long-lived book — otherwise it hangs to its own 30s timeout and reports
`FAIL: no fill in WAL after 30s` with no hint the real cause was a 404 on
the submit step. Likely the REST route was renamed/removed (to one of
the `/api/orders/*` family above) without updating this script.

- **Severity:** medium
- **Scope:** scripts/demo-trade.sh
- **Affected:** the demo-trade.sh maker/taker submission step
- **Source:** scripts/demo-trade.sh:43-56; confirmed via direct curl
  against a running playground (`start-all minimal`) during T4
  (`.ship/33-TUI-SPEED-TESTS`) test debugging.
- **Status:** open
- **Fix:** —

## MARKETDATA-SHADOW-BOOK-UNBOUNDED-LEVEL-ALLOC [OPEN]
**Severity:** HIGH (crash-loop, takes cluster non-green)
**Where:** rsx-marketdata shadow book construction/recenter — `shadow.rs:29`
`Orderbook::new(config, capacity, mid_price, ...)` sizes the level array from
`mid_price`+`tick_size` via the compression map. `mid_price` for a symbol's
shadow book is derived from replayed events, and is NOT bounds-checked.
**Symptom:** `memory allocation of 47962384944 bytes failed` (repeating) in
`log/marketdata.log`, right after `replay bootstrap complete` (seq 11385).
48 GB = 1,998,432,706 × 24 B (`PriceLevel`) → a ~2-billion-slot level array.
**Root cause (suspected):** a torn/garbage WAL record (from the earlier
ENOSPC + OOM crashes) decodes to an extreme price during replay; the shadow
book is constructed/recentered around that price, so the compression map
computes ~2 B slots instead of ~120k, and the level-array alloc aborts.
**Immediate fix:** clean-state reset (fresh WAL) so replay has no poisoned
record → marketdata bootstraps at mid=50000 (~120k slots). Unblocks the
cluster; does NOT fix the underlying vulnerability.
**Real fix (defer, record only):** bound the shadow book's derived
mid_price / computed slot count — reject or clamp a replay-derived price that
would size the level array beyond a sane cap (e.g. a few million slots), and
harden replay record decode against torn records (length/price sanity).

## PLAYGROUND-MARKET-ORDER-REJECTED-BY-GATEWAY [OPEN]
**Severity:** MED (a whole order type is unusable from the playground)
**Where:** `rsx-playground/server.py` `api_orders_test` market path
(`order_type == "market"` → `price_int = 0`) → gateway.
**Symptom:** submitting a market order (`order_type=market`, `price=0`) via
`/api/orders/test` returns `rejected: price not tick aligned`. The dashboard's
own tick check is correctly skipped for market orders, so the rejection comes
from the **gateway** (an E frame): it treats `price_raw=0` as non-tick-aligned
rather than as a market marker.
**Repro:** `curl -X POST .../api/orders/test -d 'side=buy&order_type=market&price=0&qty=10&user_id=1'`
→ red "rejected: price not tick aligned" (200). A tick-aligned crossing LIMIT
(e.g. buy 51000 vs asks ~50150) fills fine.
**Impact:** market orders don't work end-to-end; the latency test was switched
to a crossing limit as a workaround (`play_latency.spec.ts`).
**Also observed:** an unaligned LIMIT (buy 50201, tick=50) FILLED — the gateway
does NOT strictly enforce tick alignment for limits, yet rejects price-0 market
orders for it. Inconsistent. Needs a spec decision: does the gateway support
market orders (price 0 / a market flag), or is the exchange limit-only?
**Fix (defer, record only):** either (a) gateway accepts a market order marker
(price 0 or an explicit type) and matches at best-opposite, or (b) the
playground sends a crossing tick-aligned limit for "market" and drops the
price-0 path. Decide the product answer first.

## PLAYGROUND-DASHBOARD-STALE-PID-RESTART-RACE [OPEN]
**Severity:** LOW (dev ergonomics)
**Where:** `rsx-playground/playground` restart vs a dashboard started outside
the wrapper (no PID file).
**Symptom:** if a dashboard is launched directly (not via `./playground
start`), `./playground restart` can't see it (no PID file) and aborts on a
stale-PID race; subs had to `kill` by PID and relaunch via the wrapper.
**Fix (defer, record only):** have restart fall back to port-owner lookup
(`ss -ltnp` on 49171) when the PID file is missing/stale.

## GATEWAY-CRASH-UNDER-WS-CHURN (F20) [OPEN]
**Severity:** MED (gateway stability under load; cascades into other tests)
**Where:** rsx-gateway, exercised by `play_readiness.spec.ts:189` "gw-0 survives
WS connection churn (F20)".
**Symptom:** during rapid WS connect/disconnect churn the test asserts gw-0 is
still running and it is NOT (`gw-0 not running during churn`). gw-0 died under
the churn. This also degrades the cluster for whatever test runs next
(`play_overview` button test then sees <4 baseline and fails a cascade — not a
button bug; the buttons work, cluster is 7/7 after).
**Unknowns:** whether the churn goes gateway-direct or via the dashboard
`/ws/*` proxy; whether it's a gateway accept-loop / fd-exhaustion issue or a
proxy-side amplification. NOT confirmed to be caused by recent changes — F20 is
a long-standing @long test.
**Also:** `--grep-invert "@long"` did NOT exclude the @long readiness soak/churn
tests in a `bunx playwright test <files>` run — the tag filter isn't scoping as
expected, so these heavy tests run (and destabilize) even when meant to be
skipped. Worth fixing the filter/tagging separately.
**Fix (defer, record only):** reproduce the churn in isolation, capture gw-0's
exit (panic vs OOM vs fd limit), harden the accept/close path. Separately, make
the button test establish its own clean baseline (Stop All → 0 first) so it
can't cascade-fail from a prior test's damage.

## RECORDER-DEAD-BUT-HEALTHY (durability + false health) [OPEN]
**Severity:** HIGH (a durability demo whose durability is silently broken)
**Where:** rsx-recorder + rsx-health; found by the 2026-07-05 playground audit
(`.ship/40-PLAYGROUND-AUDIT/FINDINGS.md` #1).
**Symptom:** recorder process table / topology / `/component/recorder` / recovery
feed all report **running / healthy / "WAL files found"**, but the recorder log
ends with `BLOCKED: 21 consecutive stream errors exhausted retry budget (20): No
such file or directory` — ME replication (`127.0.0.1:9710`) can't serve old seq
(`56844…58462`), recorder fell behind retention and gave up. ~29 min silent
while every health surface said "fine". Archival replication is dead + invisible.
**Fix (defer, record only):** (1) health must reflect **replication liveness**
(last-consumed-seq advancing), not just pid-alive; a recorder that's BLOCKED must
surface as degraded/red. (2) recorder must **catch up from cold WAL random-access**
when it starts behind the hot retention horizon, instead of exhausting a 20-retry
budget on the live stream. Part of Phase-2 recorder/marketdata → cast quality.

## MKTDATA-DROPS-SHADOW-BOOK-DIVERGENCE [OPEN]
**Severity:** MED-HIGH (real correctness divergence, surfaced by Verify)
**Where:** rsx-marketdata; audit #3/#26.
**Symptom:** Verify FAILs `WAL self-consistency (shadow vs WAL BBO) 1/1 mismatch`;
marketdata logs continuous `WRN seq gap sym=10 expected=N got=N+1` (dropped
casts) and me-pengu `flush took 10-14ms` (>10ms target). The shadow book is
missing events it dropped, so its BBO diverges from the WAL-derived BBO.
**Fix (defer):** mktdata rcvbuf/keep-up (drain the ME casting firehose without
RcvbufErrors); investigate the flush latency; surface a drops/gaps metric.

## MARK-PRODUCES-NO-INDEX / RISK-PHANTOM-POSITIONS / ARCHIVE-WAL-BALLOON [OPEN]
**Severity:** MED (grouped audit findings — see FINDINGS.md #18, #10, #28, #29)
- **#18 mark:** connects Binance but never writes/produces an index — Verify
  SKIPs "no index (mark down)", mark WAL 0 bytes, Risk INDEX 0, while mark shows
  "running" everywhere. Make mark produce/persist, or make health say "no index yet".
- **#10 risk phantom positions:** Risk Lookup shows a large long + PnL for user 1
  with **zero backing fills** (WAL FILL = "no WAL events yet") — stale persisted
  position data. One source of truth; clear/reconcile persisted positions on Reset.
- **#28 archive WAL balloon:** 6.9→10.2 GB in ~7 min from maker quote churn with
  no crosses — drives the Dump OOM + slow flushes + fills disk. Need archive
  retention/rotation so a demo doesn't accrue GB/min.
- **#29 high auto-restart counts** on a "healthy" cluster (gw-0 11, others 7) —
  surface as instability rather than buried in the recovery feed.
**Fix (defer, record only):** these are the Phase-2 recorder/marketdata/gateway
→ cast-quality work; fix carefully, not as dashboard patches.
