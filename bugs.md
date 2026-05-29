# Bug queue

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
