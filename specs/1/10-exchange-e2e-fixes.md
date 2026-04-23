---
status: shipped
---

# Exchange E2E Fixes

## Goal

Five bugs block end-to-end exchange operation. Fix all five. Read
each referenced file fully before editing.

## Bug 1 — Gateway WS push blocked on read

**File:** `rsx-gateway/src/handler.rs`

**Problem (lines 69-98):** The handler loop drains the outbound queue
then calls `ws_read_frame` with a 10ms timeout. If the client sends no
frame for >10ms, the loop retries immediately (timeout returns
`Err(_elapsed)` → `continue`). This is already correct: the 10ms
timeout lets the loop drain outbound periodically even with no client
traffic. **Verify** by checking if fills queued by the CMP task are
actually drained and sent within one loop iteration. If the drain at
lines 70-83 already runs before every read attempt, the bug is already
fixed. If not, confirm the loop structure is:

```
loop {
    drain_outbound → send all pending msgs
    timeout(10ms, read_frame)
    if timeout → continue (re-drain)
    if frame → process
}
```

If the above structure is already present, mark this fix as verified
(no code change needed). If the drain runs AFTER the read, swap the
order.

## Bug 2 — ME drops cancels silently

**File:** `rsx-matching/src/main.rs` (CMP receive loop)

**Problem:** `RECORD_CANCEL_REQUEST` is never matched in the CMP recv
loop. Find the match arm that handles `RECORD_ORDER_REQUEST` and add a
parallel arm for `RECORD_CANCEL_REQUEST`.

**Fix:** In the CMP receive dispatch (wherever `RECORD_ORDER_REQUEST`
is matched), add:

```rust
RECORD_CANCEL_REQUEST => {
    // parse CancelRequest from payload, call book.cancel()
    // emit OrderDone/OrderFailed back via CMP to risk
}
```

Read `rsx-types/src/` for the `CancelRequest` record type and
`rsx-book` for the cancel API. Follow the exact same pattern as
`RECORD_ORDER_REQUEST` dispatch.

## Bug 3 — ME starts with empty book

**File:** `rsx-matching/src/main.rs`

**Problem:** `rsx-book` has snapshot restore logic but `main.rs` never
calls it at startup. After any restart, all resting orders are lost.

**Fix:** Before the CMP receive loop, call the snapshot restore
function. Check `rsx-book/src/snapshot.rs` for the restore API. Then
load the WAL tip and replay any records after the snapshot seq.
Follow the existing `WalReader` pattern in the codebase.

## Bug 4 — Per-tick liquidation missing

**File:** `rsx-risk/src/shard.rs` (or wherever `process_bbo` lives)

**Problem:** `process_bbo` updates index/mark prices but never
triggers a margin check. Users can breach margin from price movement
and not be liquidated until the next fill.

**Fix:** After updating the BBO/mark price in `process_bbo`, iterate
all open positions for the affected symbol and call the margin check
function. If a user is below maintenance margin, queue a liquidation.
Follow the existing liquidation path triggered by fills.

Read `rsx-risk/src/shard.rs` and `rsx-risk/src/liquidation.rs` (or
equivalent) fully before editing. Reuse the existing margin/
liquidation functions — do not duplicate logic.

## Bug 5 — Python maker stale active_cids

**File:** `rsx-playground/market_maker.py`

**Problem:** `active_cids` accumulates stale IDs when orders are
rejected or filled without the maker seeing the fill frame. The cancel
loop sends invalid cancels for these IDs, which the exchange rejects,
and errors accumulate.

The `_quote_cycle` drain already evicts cids on fill (`"F"`) and done
(`"D"`/`"U"`) frames (lines 333+). The remaining issue: when the
process restarts, `active_cids` is empty but the exchange may have
resting orders from the previous run. On restart, the maker doesn't
know their IDs so they stay orphaned.

**Fix:** On startup (before first quote cycle), send a
`GET /api/orders` or parse the WS stream for resting orders of
user_id=99 and populate `active_cids` from them. If no such endpoint
is trivially available, just send cancels for any cids seen in the
maker WAL tip. The simplest correct fix: on the first cycle after
startup, do NOT send cancels (active_cids is empty, that's correct —
the gateway will have forgotten the session anyway since WS is per-
connection and orders are tracked by user+cid in risk, not WS conn).
The real fix is to ensure `active_cids.clear()` at line 282 only
clears cids that were acknowledged, and rejected cids (error frame
`"E"`) are removed immediately.

Read `market_maker.py` fully. In the drain loop after new orders
(lines 325-350), when an error frame `"E"` is received for a cid,
evict that cid from `active_cids` immediately instead of leaving it.

## Acceptance Criteria

1. `cargo build -p rsx-gateway -p rsx-matching -p rsx-risk` — zero
   errors.
2. `cargo test --workspace` — all tests pass.
3. Run the exchange stack + maker for 30s; confirm no "reorder buffer
   full" warnings in `log/me-pengu.log` after the fix (NAK
   retransmit now works, so gaps are healed).
4. A limit order placed via WS followed immediately by a cancel
   receives an `OrderUpdate(CANCELLED)` or error frame within 2s.
5. After restarting the ME process (kill + restart), new orders can
   still be placed and matched.
6. `log/risk-0.log` shows no "liquidation scan skipped" type warnings
   on BBO updates when a user is underwater.
7. `cat tmp/maker-status.json` shows `errors` list is empty or
   contains only transient network errors (not repeated `order: qty
   not lot aligned` or stale-cid errors).
