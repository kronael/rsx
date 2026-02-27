# Exchange End-to-End Integration Spec

## Goal

The exchange works end-to-end: a market maker provides resting
liquidity, takers cross orders, fills are delivered back to WS
clients, positions are accurate, and liquidation fires on margin
breach without waiting for the next fill.

## Stack

- Rust binaries: `rsx-gateway`, `rsx-risk`, `rsx-matching` (one per
  symbol), `rsx-marketdata`, `rsx-mark`
- Python maker: `rsx-playground/market_maker.py`
- Wire: CMP/UDP (hot path), WAL replication over TCP (cold path)
- Client API: WEBPROTO WS + REST (see specs/v1/WEBPROTO.md,
  RPC.md, MESSAGES.md)

## Known Issues to Fix

Five concrete bugs block acceptance criteria:

1. **Gateway WS push blocked on read** (`rsx-gateway/src/handler.rs:69-98`)
   The per-connection loop drains the outbound queue then blocks on
   `ws_read_frame` indefinitely. Fills queued by the CMP task sit
   unsent until the client next sends a frame. Fix: interleave WS
   read and outbound drain (monoio select or join).

2. **ME drops cancels silently** (`rsx-matching/src/main.rs` CMP recv loop)
   Only `RECORD_ORDER_REQUEST` is matched; `RECORD_CANCEL_REQUEST` is
   never handled. Cancels from GW→risk→ME are silently discarded.
   Fix: add cancel dispatch in the CMP receive loop.

3. **ME starts with empty book** (`rsx-matching/src/main.rs`)
   `rsx-book` has snapshot restore logic but `main.rs` never calls it.
   After any restart, all resting orders are lost. Fix: load WAL
   snapshot on startup before accepting new CMP messages.

4. **Per-tick liquidation missing** (`rsx-risk/src/shard.rs:drain_stashed_bbos`)
   `process_bbo` updates index price only. No liquidation scan runs
   on BBO or mark price updates. Users can breach margin from price
   movement alone and are not liquidated until the next fill.
   Fix: iterate exposed users and check margin on every BBO update
   (RISK.md §7).

5. **Python maker stale order cleanup** (`rsx-playground/market_maker.py`)
   `active_cids` is never cleaned on fill receipt — stale cancels are
   sent for already-filled orders on the next cycle. On restart,
   `active_cids` resets to empty, leaving orphan resting orders in the
   book. Computed prices are not aligned to tick size.
   Fix: consume fill WS frames, evict filled cids, align prices to
   tick size before submission.

## Acceptance Criteria

All criteria are runnable and observable. Ship must not close the
spec until every criterion passes.

### 1. Build

```
cargo build -p rsx-gateway -p rsx-risk -p rsx-matching \
            -p rsx-marketdata -p rsx-mark
```

Succeeds with zero errors, zero `unused` warnings.

### 2. Unit tests

```
cargo test --workspace
```

All tests pass. No new tests are required for this criterion.

### 3. WS round-trip fill

A gateway E2E test (new test, not TODO):

1. Start gateway + risk + ME for symbol 10 in process.
2. Connect two WS clients: maker and taker.
3. Maker places a limit bid at price P, qty 1.
4. Taker places a market ask qty 1 (or limit ask ≤ P).
5. Assert: taker receives `WsFrame::Fill` within 1 s.
6. Assert: maker receives `WsFrame::Fill` within 1 s.

Test name: `ws_new_order_fill_update_complete`.

### 4. Cancel round-trip

1. Client places a limit order (resting, not immediately filled).
2. Client sends cancel for that order.
3. Assert: client receives `WsFrame::OrderUpdate` with status
   `CANCELLED` within 1 s.
4. Assert: order is absent from the book (confirm via `GET /x/book`).

### 5. Restart safety

1. Start ME, place 3 resting limit orders, checkpoint WAL.
2. Kill ME process, restart it.
3. Assert: all 3 resting orders are present in the restored book
   (verified by `GET /x/book` or direct book inspection).
4. New orders can still match against restored resting orders.

### 6. BBO-triggered liquidation

A risk shard E2E test:

1. Fund a user to minimum margin for a long position.
2. Insert a fill to open the position.
3. Drop the BBO mark price below the liquidation threshold
   (without any new fill).
4. Assert: within one BBO processing cycle the shard emits a
   liquidation CMP message for that user.

### 7. Maker integration

With all five binaries running and `market_maker.py` connected:

1. Wait 3 s after maker startup.
2. `GET /x/book?symbol_id=10` returns ≥ 5 bid levels and ≥ 5 ask
   levels, all with non-zero qty.
3. `GET /api/maker/status` returns `{"running": true,
   "orders_placed": N}` where N > 0.
4. Submit a crossing limit order via WS.
5. Assert: the order produces a fill record in the WAL within 2 s.

### 8. Stress test

With all binaries running:

```
python rsx-playground/market_maker.py &
python rsx-playground/stress_client.py --rate 50 --duration 10
```

Assert: `accept_rate > 0.8` (fills + resting accepted / total
submitted). No process crash or OOM. WAL tip advances monotonically
throughout the run.
