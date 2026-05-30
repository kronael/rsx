# Bug queue

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
