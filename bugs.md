# Bug queue

The review queue: **OPEN** and **DEFERRED** items only. Resolved bugs live
in git (commit refs below) and `CHANGELOG.md` — not here.

## Status — 2026-05-30

**OPEN (triage):**
- **ME-FAULTED-NO-REPLAY-ADDR** (MED) — parallel load FAULTs the ME → panic;
  no replay source wired. Blocks parallel-load benchmarking. Detail below.
- **IOC-NOT-HONORED** (MED) — empty-book IOC rests instead of cancelling; `tif`
  lost on the GW→risk→ME propagation (matching core is correct). Detail below.
- **GATEWAY-LATENCY** (HIGH, *mitigated*) — egress-drain poll 10ms→500µs shipped
  (`5a578d3`); the zero-poll tile-split is the scale path, deferred per founder
  ("make the tile correct, don't split now"). Detail below.

**DEFERRED — book session** (founder: "solve once we're dealing with book"):
BOOK-BBO-COMPRESSED-INDEX, BOOK-SCAN-NEXT-BID-OFFBY, BOOK-SLAB-FREE-UNGUARDED,
BOOK-STALE-HANDLE-REUSE, ME-REDUCEONLY-IOC-FILLEDQTY, BOOK-FAR-PRICE-BUCKETING.
Detail below.

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

- **BOOK-BBO-COMPRESSED-INDEX `[V]`.** `book.rs:184-194` (`best_bid/ask`) and
  `scan_next_bid/ask` (`339-378`) compare the *compressed tick index* as a price
  proxy. The compression map is sawtooth, not globally price-monotonic (mid=100,
  a 95 bid → index 10 while a 99 bid → index 3), so with resting orders in >1
  zone, `best_bid/ask` and crossing detection are wrong. Fix: track best by raw
  price per side, or make `price_to_index` globally monotonic.
- **BOOK-SCAN-NEXT-BID-OFFBY `[V]`.** `book.rs:340` — `scan_next_bid` guards
  `if from < 2 { return NONE }`; for `from==1` it should still check tick 0.
  Cancelling the best bid at tick 1 with a resting bid at tick 0 drops that bid
  from the BBO. Fix: guard `from == 0 || from == NONE`.
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
