# Bug queue

## LATENCY-TRACE-ALWAYS-ON — per-stage trace runs unconditionally on the hot path (LOW)

**Status: OPEN.** Found 2026-05-30 in the hot-path audit. `rsx_log::latency::
sample` → `rsx_log::push` is **ungated**: every order pushes ≥2 records per
stage (e.g. ME does `me_in` + `me_dedup_done`, each preceded by a `time_ns()`
VDSO clock read + a thread-local `RefCell::borrow_mut` + SPSC push). Across all
stages (gateway/risk/ME emit several) that's ~tens of ns/order of trace
overhead on a ~340 ns match budget, present even in production where the trace
is unwanted. Violates the "no logging on hot path" discipline (notes/hot-path
.md). **Fix:** gate behind a process-wide `AtomicBool` read once from an env
var at startup (`RSX_LATENCY_TRACE`), early-return in `push`/`sample` when off
— one predictable branch, no syscall. NOT flipped here because the latency-
publish tooling + the reports/ stage timings consume these samples; changing
the default needs those updated in lockstep. Triage only.

## IOC-NOT-HONORED — IOC order with no liquidity rests instead of cancelling (MED)

**Status: OPEN.** Found 2026-05-30 building the WS order-latency bench. A
`{N:[10,0,1,100000,cid,1]}` (tif=1 = IOC) BUY submitted against a confirmed-
empty book returns `{"U":[oid,1,...]}` (status RESTING / OrderInserted) — it
inserts into the book instead of immediately cancelling. Per rsx-book
matching.rs:188-199, a `remaining_qty > 0` IOC must emit `OrderDone` with
`REASON_CANCELLED` (not `OrderInserted`). So tif=IOC is being dropped or
ignored somewhere on the GW→risk→ME path. Gateway parses arr[5]→tif correctly
(records.rs:200), risk forwards `tif: order.tif` (main.rs:1059), ME converts
1→IOC (wire.rs:36) — yet the order rests. Repro: empty-book IOC buy via WS.
Effect on bench: the latency harness uses GTC + explicit cancel-after instead
(book hygiene), which works; IOC would have been simpler. Triage only — not
fixed (no explicit fix request).
**Refined 2026-05-30 (oracle bug-hunt):** the matching CORE is correct — an
IOC residual is sent to `OrderDone` at `rsx-book/src/matching.rs:188` and the
empty-book IOC test covers it. So the bug is NOT the algorithm; it's in the
GW→risk→ME *propagation* of `tif` (a field is lost/defaulted to GTC somewhere
between the gateway parse and the book call). Narrows the search to wire
decode/encode at a shard boundary.

## Oracle bug-hunt 2026-05-30 (risk / matching / book / gateway)

4 background codex (gpt-5.5) passes, one per crate. Each finding below was
spot-verified against the source (confidence tagged). **None fixed** — review
queue. `[V]`=verified real, `[D]`=by-design/known-limitation (logged for
tracking, likely not actionable), `[?]`=needs one more verification.

### High

- **RISK-NO-PRICE-QTY-GUARD `[?]` (potential solvency).** `shard.rs:553`
  `process_order` → `margin.check_order` (`margin.rs:116`) computes
  `notional = price*qty` with **no `price>0 / qty>0` guard**, and the
  `rsx_types::validate_order` helper is called **nowhere** on the live path.
  A negative qty/price would yield negative `margin_needed` → a negative
  freeze that *increases* available margin. VERIFY whether the gateway (or ME
  config check) rejects `price<=0 / qty<=0` before risk; I could not locate
  such a guard. If absent → real solvency hole. Fix: reject non-positive
  price/qty at the gateway entry (cite trust boundary) and/or defensively in
  margin.
- **ME-NEXT-SEQ-REGRESSION `[V]`.** `rsx-matching/src/main.rs:333` — after a
  snapshot loads and **zero** WAL records replay past `start_seq`, the `Ok(_)`
  branch leaves `next_seq = 1` ("writer is fresh") even though the snapshot
  implies prior on-disk seqs. Next live append reuses/regresses WAL seqs →
  violates invariant #5 (tips monotonic). Fix: `set_next_seq(start_seq)` even
  when zero records replayed.
- **ME-SNAPSHOT-NO-INDEX-DEDUP-REBUILD `[?]`.** `main.rs:303` — snapshot
  restore rebuilds the book but not `order_index` or `dedup`. After restart:
  cancels for pre-snapshot resting orders miss, and order IDs accepted before
  the snapshot can be re-accepted (dup). Fix: scan snapshot orders into
  `order_index`; persist+restore dedup (or replay `RECORD_ORDER_ACCEPTED`).

### Medium

- **BOOK-BBO-COMPRESSED-INDEX `[V]`.** `book.rs:184-194` (`best_bid/ask`
  update) and `scan_next_bid/ask` (`339-378`) compare the **compressed tick
  index** as a price proxy (`tick > best_bid_tick`). The compression map is
  *sawtooth*, not globally price-monotonic: zone-1 bids get higher indices
  than zone-0 bids but represent *lower* prices (verified by hand: mid=100,
  a 95 bid → index 10 while a 99 bid → index 3). So with resting orders in
  >1 zone, `best_bid/ask` and crossing detection are wrong. Fix: track best
  by raw price per side, or make `price_to_index` globally monotonic.
- **BOOK-SCAN-NEXT-BID-OFFBY `[V]`.** `book.rs:340` — `scan_next_bid` guards
  `if from < 2 { return NONE }`; for `from==1` it should still check tick 0.
  Cancelling the best bid at tick 1 with a resting bid at tick 0 drops that
  bid from the BBO. Fix: guard `from == 0 || from == NONE`.
- **RISK-FREEZE-LEAK-ON-ME-SEND-FAIL `[V]`.** `main.rs:1079,1082` — the
  in-memory freeze is inserted pre-send (correct pre-trade gate); if the ME
  send fails or no `me_sender` exists for the symbol, the path only logs, so
  no confirming `OrderAccepted`/release ever arrives → the in-memory freeze
  leaks margin (distinct from ORPHAN-FREEZE, which was the *durable* PG side).
  Fix: `release_frozen_for_order` + reply ORDER_FAILED on send error / missing
  sender.
- **GW-U64-TRUNCATION `[V]`.** `records.rs:91` `as_u32` truncates JSON numbers
  > `u32::MAX` (e.g. 4294967296 → 0) → wrong symbol/user routing. Fix:
  range-check `n <= u32::MAX` before the cast.
- **GW-WS-FIN-IGNORED `[V]`.** `ws.rs:338` ignores the FIN bit — a fragmented
  text frame (`FIN=0, opcode=1`) is treated as a complete message → an order
  can be parsed from a partial frame. Fix: reject `FIN=0` or reassemble.
- **GW-PENDING-BEFORE-SEND `[V]`.** `handler.rs:459` inserts the pending order
  (+ `record_success`) before the cast send at 490; on send failure it only
  logs → stuck pending, client never told. Fix: send first, or roll back
  pending + surface the failure.
- **GW-CANCEL-SEQ-GAP `[V]`.** `handler.rs:692` advances the cast seq even when
  `send_raw` fails (695) → local seq gap + silently dropped cancel. Fix:
  `advance_seq()` only after a successful send.
- **GW-COMPLETION-ROUTE-BY-USERID `[V]`.** `route.rs:131` routes a completion
  by `rec.user_id` instead of pairing on the pending `order_id` → a wrong
  user_id misroutes the update and removes the real user's pending order. Fix:
  look up pending by order_id, then route to `pending.user_id`.
- **ME-REDUCEONLY-IOC-FILLEDQTY `[?]`.** `rsx-book/src/matching.rs:190` —
  `filled = order.qty - order.remaining_qty` counts the reduce-only clamp as
  an execution, so an empty-book reduce-only IOC can report nonzero filled
  qty. Fix: compute fills from actually-matched qty, separate from the clamp.

### Low / by-design

- **BOOK-FAR-PRICE-BUCKETING `[D]`.** `compression.rs:48,118` — compression
  buckets far prices (10/100/1000 ticks per slot), so distinct prices share a
  level → price-time priority is coarse for far-from-mid orders. Intentional
  compressed-book tradeoff; logged as a known design risk, not a defect.
- **BOOK-STALE-HANDLE-REUSE `[?]`.** `book.rs:241` — `cancel_order` only checks
  `is_active()`; the slab reuses freed indices, so a stale handle could alias a
  reused slot. Safe only if the ME's `order_index` never retains a freed
  handle. Fix (defensive): generational handles or `(handle, order_id)` check.
- **BOOK-SLAB-FREE-UNGUARDED `[V]` (hardening).** `slab.rs:49` — `free()`
  accepts any in-bounds index → double-free / freelist cycle possible. Add a
  debug assert (`idx < bump_next` and not already free).
- **RISK-INDEX-QTY-OVERFLOW `[V]`.** `price.rs:18` sums `bid_qty + ask_qty`
  before widening to i128 → debug panic / release wrap on huge quantities.
  Fix: widen to i128 before the add.
- **GW-WS-UNMASKED-ACCEPTED `[V]` (hardening).** `ws.rs:339` accepts unmasked
  client frames (RFC requires client→server masking). Fix: reject `!masked`
  inbound.
- **RISK-FUNDING-CROSS-SHARD `[D]`.** `shard.rs:955` — `settle_symbol` zeroes
  only the shard-local rounding residual; global zero-sum (invariant #9) isn't
  guaranteed across shards. Known multi-shard gap (demo is single-shard).
- **GW-SINGLE-SHARD-NO-ROUTING `[D]`.** `main.rs:166` — one `RSX_RISK_CAST_ADDR`
  sender, no `user_id % shard_count` selection. Known single-shard demo limit.
- **ME-REPLAY-SKIPS-DOWNSTREAM `[?]`.** `replay.rs:126` — FAULTED/DXS replay
  rebuilds the local book + ME WAL but does not re-send recovered
  fills/done/BBO to risk/marketdata. May be by-design (each consumer recovers
  independently via its own replay) — confirm before treating as a bug.

## GATEWAY-LATENCY — casting-recv poll-loop starvation dominates e2e (HIGH)

**Status: OPEN.** Single-order stage trace (live cluster): ME finishes +
response leaves Risk by ~571µs (`me_out`), but the gateway doesn't receive
it (`gateway_cmp_recv`) until ~4794µs — a **~4.2ms hole**. Across 1934
probes the `me_out → gateway_cmp_recv` gap is **~0.8ms p50 / ~10ms p90** —
i.e. essentially the entire e2e latency. The response sits in the gateway's
UDP socket buffer waiting for the casting-recv poll loop to get a turn on
the shared monoio reactor (WS accept + per-conn handlers + casting-recv all
on one reactor; `sleep(ZERO)` yields the core per empty poll). Risk (65µs),
ME match (~80µs), Risk return (556µs) are all sub-ms — NOT the bottleneck.
(NB: earlier "11ms is Python" was wrong — GW-only RTT is 143µs.)
**Fix:** tile-split the casting-recv response path to a dedicated pinned
busy-spin thread (off the reactor) → SPSC ring → WS writer tasks. Same
pattern as Risk/ME. Biggest single e2e win.

## ANSI-IN-LOGS — tracing writes color escapes into log files (LOW)

**Status: OPEN.** Process logs (`log/*.log`) contain ANSI color escapes
(`\x1b[2m…\x1b[0m`) because the tracing fmt layer has ANSI enabled even
when output is a file, not a TTY. Makes structured latency lines
(`stage="risk_in" t_us=…`) un-greppable without stripping. **Fix:**
`.with_ansi(false)` (or TTY-detect) in the tracing init across rsx binaries
(gateway/risk/matching/marketdata/mark/recorder), ideally via a shared
rsx-log init helper.

## ORPHAN-FREEZE — phantom margin hold survives risk recovery (correctness)

**Status: FIXED 2026-05-29** (commit: durable freeze on ME OrderAccepted).
`process_order` keeps the in-memory freeze (pre-trade gate) but no longer
write-behinds to PG pre-send; `shard::confirm_freeze` writes the durable
`FrozenInsert` only when ME's `RECORD_ORDER_ACCEPTED` returns. PG can no
longer hold a freeze the WAL never confirmed. 8 new tests in
`orphan_freeze_test.rs` (FM6/FM11/FM13a/FM14); rsx-risk 268 pass.
**Follow-up (minor, open):** PG rows written *before* this fix could still
hold legacy orphans — a one-time Option-A reconcile (drop PG freezes with
no WAL `OrderAccepted`) on next recovery would clear them. New orders are
closed at the source.

---

### (original report, now fixed)
`process_order` (`rsx-risk/src/shard.rs:580`) freezes
in-memory AND write-behinds a `FrozenInsert` to PG **before forwarding to
ME**. If risk dies (or the risk→ME send drops) after the PG write but
before ME accepts, recovery loads the PG `frozen_orders` snapshot with a
freeze that has **no `OrderAccepted` and no release in the WAL** → a
permanent phantom hold on the user's available margin.

**Fix (user-confirmed: "reconcile from the WAL"):** make the WAL
`OrderAccepted` stream the sole authority for freezes. Either (a) on
recovery reconcile the PG snapshot against the WAL and drop freezes the
WAL never confirms, or (b) write-behind the durable freeze on
`OrderAccepted` ingestion (ME-confirmed) instead of pre-send. The
in-memory pre-send freeze (pre-trade gate) stays; only its durable record
moves to the WAL-confirmed point. Spec: `specs/2/28-risk.md` Return Path.

**Related:** no `valid_until`/GTD exists (TIF = GTC|IOC|FOK only); adding
one would bound lost-reply/orphan resting-order lifetime but does NOT fix
this (an order ME never accepted has nothing to expire).

## SEQ-1 — casting seq-space collision on filtered fan-out streams (CRITICAL) — RESOLVED 2026-05-29

**Status: FIXED.** Steady-state FAULTED count is now 0/15s on all three
streams (was ~2/sec on risk+marketdata, ~34/min on gateway). The e2e latency
probe went from 5–17 clean samples (mostly timeouts) to **551/500 ok, 0 fail**.

**Fixes shipped:**
- ME `publish_events`: OrderDone, OrderFailed, and BBO now fan out to BOTH
  risk+marketdata on the single WAL seq (BBO was using CastSender's own
  next_seq counter — the desync root cause; now WAL'd).
- ME main loop: OrderAccepted (was WAL-only) and duplicate-reject (was
  cmp-only) now fan to both — they consume a WAL seq, so skipping a stream
  punched a hole. OrderAccepted fires per resting order → was the ~2/sec rate.
- risk→gateway: all sends go through `forward_to_gw`, which renumbers with
  gw_sender's own contiguous seq (fixes the seq=0 margin-reject the gateway
  dropped, and ME-seq-space holes). Gateway never replays from risk → safe.

Original diagnosis kept below for reference.

---

## SEQ-1 (original diagnosis)

**Symptom:** constant FAULTED replay storms on the ME→risk, ME→marketdata,
and risk→gateway casting streams, even with **zero kernel packet loss**
(verified: `RcvbufErrors`/`InErrors`/`SndbufErrors` all +0 under load;
`OutDatagrams == InDatagrams`). The "UDP packet loss" was a misdiagnosis —
the disease is sequence numbering.

**Root cause (ME, `rsx-matching/src/wal_integration.rs:publish_events`):**
A single casting stream carries records stamped from **two independent seq
counters**:
- WAL'd records (Fill/Inserted/Cancelled/Done) go via `fan_out → send_framed`,
  which stamps the **WAL writer's** seq and sets `CastSender.next_seq = wal_seq+1`.
- BBO goes via `cmp.send()` / `mkt.send()` (line 434/437), which uses the
  **CastSender's own `next_seq`** and increments it. BBO is **not** WAL'd, so
  it does not advance the WAL counter.

After a BBO, `next_seq` is ahead of the WAL counter; the next `send_framed`
resets `next_seq` backward to `wal_seq+1` → the receiver sees a seq regression
("cmp sender reset detected") and/or a duplicate seq → reorder thrash → FAULTED.
`cast.rs:243 send()` vs `cast.rs:~343 send_framed()` confirm the two counters.

**Compounding holes (same class):**
- `OrderFailed` consumes a WAL seq but is sent to **neither** stream
  (`wal_integration.rs:401`, WAL-only) → permanent hole on both streams.
- `OrderDone` is `fan_out(.., None, ..)` → cmp only → hole on the marketdata stream.
- risk→gateway: risk forwards only a **subset** of ME records via `send_raw`
  (preserving ME's WAL seq), so the gateway sees a filtered, gappy seq stream.
- risk's own margin-reject `OrderFailedRecord` is sent with `seq=0`
  (`rsx-risk/src/main.rs:~1045`) → gateway receiver drops `seq==0`
  (`cast.rs:907`) → client never told the order was rejected.

**Why it was masked:** the FAULTED→TCP-replay path stormed so hard (and the
unpinned `rsx-mark` busy-spinner starved the gateway core, causing *real*
RcvbufErrors on top) that the seq bug looked like UDP loss. Pinning/ergonomic
mark removed the real drops and exposed this as the residual.

**Fix options (need design sign-off — touches ME, risk, gateway, WAL, replay):**
1. **Full fan-out, single WAL seq:** every WAL'd record (incl. OrderFailed) →
   both cmp+mkt; give BBO a WAL seq (append it, or reserve a seq) and fan it
   out too. Both streams become the complete contiguous WAL-seq sequence;
   consumers already `match` on record_type and ignore the rest. Risk→gateway
   must likewise forward the full stream (or re-sequence). Fix risk's seq=0
   OrderFailed.
2. **Per-stream transport seq:** move the transport seq into the WAL *header*
   (wire-format bump) so the record's durable seq and the per-(sender→receiver)
   transport seq are independent; each CastSender owns its stream's contiguous
   seq. Cleaner long-term, bigger change.

Recommend (1) as the minimal correct fix for the demo.

## Fixed this session (for context)
- UDP RcvbufErrors from unpinned `rsx-mark` busy-spinner starving the gateway
  core — fixed by making mark ergonomic (sleep 250µs) + documenting core layout.
- marketdata FAULTED-panic crash loop (`RSX_MD_REPLAY_ADDR` unset, stream_id=1
  vs ME stream 10) — wired in `start`.
- WAL replay panicked on `REPLICATION_NOT_AVAILABLE` instead of retrying past
  the 10ms flush window — retry added in matching/risk/marketdata.
