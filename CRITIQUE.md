# Critique (Current State)

This critique reflects the repo as it exists now. I did not run tests.

## Summary (Top Issues)

1) **Price feeds into risk are broken.**
   Mark does not send CMP to risk; risk has no ME BBO ingestion.
   Result: `mark_prices` and `index_prices` stay stale/zero.
   - Files: `rsx-mark/src/main.rs`, `rsx-risk/src/main.rs`

2) **Frozen margin release is incorrect.**
   `OrderDoneEvent.frozen_amount` is always 0 and cancel path does not
   release margin at all. This over-reserves margin indefinitely.
   - Files: `rsx-risk/src/main.rs`, `rsx-risk/src/shard.rs`

3) **Client-facing order status is inconsistent with ME outcomes.**
   Gateway ignores `OrderDoneRecord.final_status` and always reports
   status=filled; cancels routed through OrderDone are misreported.
   - Files: `rsx-gateway/src/main.rs`, `rsx-dxs/src/records.rs`

4) **Reject reason codes don’t match WEBPROTO.**
   Risk encodes reject reasons as 1..3; WEBPROTO expects different
   numeric mapping (e.g., insufficient margin = 4). Gateway forwards raw.
   - Files: `rsx-risk/src/main.rs`, `rsx-gateway/src/main.rs`, `specs/v1/WEBPROTO.md`

5) **Gateway sends a “pending” ack despite WEBPROTO’s no‑ack rule.**
   Spec says first response is from matching; implementation sends an
   immediate pending OrderUpdate.
   - Files: `rsx-gateway/src/handler.rs`, `specs/v1/WEBPROTO.md`

## Component-by-Component Mismatch Review

### Gateway

**Spec expectations (v1):** No pre-trade ack; order updates only from ME
path; failure codes align with WEBPROTO; optional heartbeat.

**Implementation:**
- Sends immediate pending ack on new order.
- Forwards OrderFailed from risk (pre-trade) as status=failed.
- Reports OrderDone as filled regardless of final_status.
- JWT auth enforced in WS handshake; fallback `X-User-Id`.

**Mismatches:**
- Pending ack violates WEBPROTO.
- Status mapping for OrderDone ignores final_status.
- Failure reason codes don’t match WEBPROTO enum.

### Risk

**Spec expectations (v1):** Ingest fills+BBO, mark feeds; release frozen
margin on cancel/done; forward CONFIG_APPLIED to gateway; replica sync via
SPSC or defined channel.

**Implementation:**
- Ingests fills, cancels, done, config_applied from ME over CMP.
- Ingests mark price from CMP (expects Mark to send it).
- No BBO ingestion path.
- Sends OrderFailed to gateway on reject.
- Replica mode: advisory lease + CMP tip sync channel (record_type 0x20).

**Mismatches:**
- Mark feed not actually connected (Mark doesn’t send CMP).
- No BBO ingestion, so index price never updates.
- Frozen margin not released on cancel/done.
- Tip sync protocol is ad‑hoc (0x20) and undocumented in v1.

### Matching

**Spec expectations (v1):** Fan-out events; BBO to risk; WAL + DXS replay;
config polling; dedup, reduce-only, etc.

**Implementation:**
- Sends fills/insert/cancel/done/config to risk; fills/insert/cancel to
  marketdata.
- WAL + DXS replay present.
- ConfigApplied emitted at startup (version=1).

**Mismatches:**
- BBO not emitted to risk (no RECORD_BBO path).
- Config polling behavior not present (only startup emit).

### Marketdata

**Spec expectations (v1):** Shadow book; snapshots on subscribe; seq gap
handling; backpressure resubscribe; replay from DXS.

**Implementation:**
- Replay bootstrap from DXS supported (optional).
- Seq gap detection and snapshot resend implemented.
- Snapshot only sent if book exists (empty-book subscribe yields none).
- No explicit backpressure resubscribe policy; outbound drops silently.

**Mismatches:**
- Empty-book snapshot handling not defined/implemented.
- Backpressure policy still weak (drops without resubscribe signal).

### Mark

**Spec expectations (v1):** Mark aggregator publishes mark prices (ideally
to risk) and via DXS.

**Implementation:**
- Aggregates and writes MarkPrice to WAL; exposes DXS replay server.
- No CMP sender; risk’s CMP mark receiver never gets updates.

**Mismatch:**
- Mark → Risk live feed not implemented.

### DXS/CMP/WAL

**Spec expectations (v1):** CMP header + payload; seq in payload; flow
control; WAL flush.

**Implementation:**
- WalHeader = (record_type, len, crc32, reserved[8]); payload seq for data.
- Flow control enforced in `send` (returns false when window closed).
- WAL flush fsyncs; retention by timestamp.

**Mismatch:**
- Some spec text still assumes “stall on send” instead of bool return;
  updated in v1 spec already.

## Verified Improvements Since Last Critique

- Marketdata now has replay bootstrap, seq-gap detection, and heartbeats.
- Risk forwards ME events to Gateway and emits OrderFailed on reject.
- Gateway handles OrderFailed and adds heartbeat broadcast.
- Specs updated to reflect CMP/UDP inter-process links.

## Test Reality

- Not run in this pass.

## Bottom Line

The core plumbing is largely in place, but risk price inputs and margin
release semantics are still incorrect. Client-facing status/reason mapping
also needs alignment with WEBPROTO. Marketdata is closer, but empty-book
snapshot and backpressure semantics remain undefined.
