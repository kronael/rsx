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
