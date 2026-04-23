---
status: shipped
---

# CODEPATHS (v1)

This document enumerates major end-to-end codepaths and maps them to
implementation files and tests. It is descriptive, not normative.

## 1) New Order (WS -> Gateway -> Risk -> Matching -> Events -> Gateway)

Spec intent:
- Client sends `{N:[...]}`.
- Risk validates, forwards to ME.
- ME emits OrderInserted/Fill/OrderDone; Gateway pushes updates to client.

Implementation path:
- WS parse/validate/rate-limit: `rsx-gateway/src/handler.rs`
- CMP send to Risk: `rsx-gateway/src/handler.rs` (`RECORD_ORDER_REQUEST`)
- Risk pre-trade check & freeze: `rsx-risk/src/shard.rs::process_order`
- Risk -> ME: `rsx-risk/src/main.rs` (`OrderMessage`)
- ME match + emit events: `rsx-matching/src/main.rs`
- Risk forwards ME events to Gateway: `rsx-risk/src/main.rs`
- Gateway routes events to users: `rsx-gateway/src/main.rs`

Tests:
- WS frame parsing: `rsx-gateway/tests/protocol_test.rs`
- Risk pre-trade checks/margin: `rsx-risk/tests/margin_test.rs`, `rsx-risk/tests/shard_test.rs`
- Matching fanout/unit: `rsx-matching/tests/fanout_test.rs`
- Gateway order lifecycle routing: `rsx-gateway/tests/order_lifecycle_test.rs`
- No full WS->CMP->ME->WS e2e test present.

## 2) Cancel Order (WS -> Gateway -> Risk -> ME -> Cancel/Done)

Spec intent:
- Client sends `{C:[cid_or_oid]}`.
- Cancel routed to ME; ME emits OrderCancelled or OrderDone.

Implementation path:
- Cancel parse + pending lookup: `rsx-gateway/src/handler.rs`
- CMP cancel request: `rsx-gateway/src/handler.rs` (`RECORD_CANCEL_REQUEST`)
- ME cancel handling: `rsx-matching/src/main.rs` (via order flow)
- Risk forwards cancel/done: `rsx-risk/src/main.rs`
- Gateway routes cancel/done: `rsx-gateway/src/main.rs`

Tests:
- Cancel parsing: `rsx-gateway/tests/protocol_test.rs`
- Pending lookup: `rsx-gateway/tests/pending_test.rs`
- Gateway cancel routing: `rsx-gateway/tests/order_lifecycle_test.rs`

## 3) Pre-Trade Reject (Risk -> Gateway -> Client)

Spec intent:
- Risk rejects invalid/margin-insufficient orders and client receives failure.

Implementation path:
- Reject reason decided in `rsx-risk/src/shard.rs::process_order`
- `OrderFailedRecord` emitted in `rsx-risk/src/main.rs`
- Gateway routes `OrderFailedRecord` in `rsx-gateway/src/main.rs`

Tests:
- Margin reject logic: `rsx-risk/tests/margin_test.rs`
- No explicit WS failure code mapping tests found.

## 4) Fill Flow (ME -> Risk -> Gateway)

Spec intent:
- ME emits Fill; Risk updates positions; Gateway sends WS fill.

Implementation path:
- ME fill emit: `rsx-matching/src/main.rs`
- Risk ingest: `rsx-risk/src/main.rs` -> `FillEvent` -> `rsx-risk/src/shard.rs::process_fill`
- Gateway route fill: `rsx-gateway/src/main.rs::route_fill`

Tests:
- Risk fill logic: `rsx-risk/tests/position_test.rs`, `rsx-risk/tests/fee_test.rs`
- Matching fanout test (unit): `rsx-matching/tests/fanout_test.rs`
- Gateway fill routing: `rsx-gateway/tests/order_lifecycle_test.rs`

## 5) Order Completion (OrderDone / Cancelled -> Margin Release)

Spec intent:
- Frozen margin is released on completion/cancel.

Implementation path:
- Track frozen per order: `rsx-risk/src/shard.rs` (`frozen_orders` map)
- Release on ME cancel/done: `rsx-risk/src/main.rs`

Tests:
- Unit tests exist for freeze/release at account level: `rsx-risk/tests/account_test.rs`
- Margin tests cover release scenarios but not ME cancel/done integration.
- Gateway done routing clears pending: `rsx-gateway/tests/order_lifecycle_test.rs`

## 6) Mark Price Feed (External -> Mark -> Risk)

Spec intent:
- External feeds -> mark median -> risk updates `mark_prices`.

Implementation path:
- Connectors: `rsx-mark/src/source.rs`
- Aggregation + WAL + CMP send: `rsx-mark/src/main.rs`
- Risk CMP receiver: `rsx-risk/src/main.rs`

Tests:
- Mark aggregation: `rsx-mark/tests/aggregator_test.rs`
- Mark->risk CMP ingest: `rsx-risk/tests/cmp_ingest_test.rs`

## 7) BBO / Index Price Feed (ME -> Risk)

Spec intent:
- ME emits BBO; Risk updates index price for funding/margin.

Implementation path:
- BBO emit in ME: `rsx-matching/src/main.rs` (BboRecord)
- Risk ingests RECORD_BBO: `rsx-risk/src/main.rs`
- Index update: `rsx-risk/src/shard.rs::process_bbo`

Tests:
- Index price unit tests: `rsx-risk/tests/price_test.rs`
- ME BBO->risk CMP ingest: `rsx-risk/tests/cmp_ingest_test.rs`

## 8) Marketdata (ME -> Marketdata -> WS)

Spec intent:
- Shadow book from inserts/cancels/fills; BBO/L2/trades to WS.
- Seq gaps trigger snapshot resend; replay bootstrap supported.

Implementation path:
- CMP ingest & seq-gap: `rsx-marketdata/src/main.rs`
- Shadow book: `rsx-marketdata/src/shadow.rs`
- WS protocol: `rsx-marketdata/src/protocol.rs`, `rsx-marketdata/src/handler.rs`

Tests:
- Shadow book: `rsx-marketdata/tests/shadow_test.rs`
- Seq gap: `rsx-marketdata/tests/seq_gap_test.rs`
- Replay: `rsx-marketdata/tests/replay_test.rs`, `replay_e2e_test.rs`
- Empty-book snapshot/backpressure: `rsx-marketdata/tests/state_resync_test.rs`

## 9) DXS Replay (WAL -> TCP -> Consumers)

Spec intent:
- WAL records replayed via DXS server; consumers bootstrap state.

Implementation path:
- WAL: `rsx-dxs/src/wal.rs`
- DXS server: `rsx-dxs/src/server.rs`
- Marketdata replay bootstrap: `rsx-marketdata/src/replay.rs`

Tests:
- WAL tests: `rsx-dxs/tests/wal_test.rs`
- DXS client tests: `rsx-dxs/tests/client_test.rs`
- Marketdata replay tests: `rsx-marketdata/tests/replay_test.rs`
- CMP flow control/NAK: `rsx-dxs/tests/cmp_test.rs`

## 10) Risk Replica Sync

Spec intent:
- Replica keeps up with tips, can promote on lease loss.

Implementation path:
- Tip sync CMP record_type 0x20: `rsx-risk/src/main.rs`
- Replica buffers fills: `rsx-risk/src/main.rs` (replica loop)

Tests:
- Replica unit/e2e: `rsx-risk/tests/replica_test.rs`, `replication_e2e_test.rs`

## 11) Auth + Heartbeats

Spec intent:
- JWT auth required; heartbeat to close idle connections.

Implementation path:
- JWT validation in WS handshake: `rsx-gateway/src/ws.rs`
- Gateway heartbeat broadcast: `rsx-gateway/src/main.rs`
- Marketdata heartbeat + timeouts: `rsx-marketdata/src/main.rs`, `state.rs`

Tests:
- JWT unit/e2e: `rsx-gateway/tests/jwt_test.rs`, `jwt_ws_e2e_test.rs`
- Marketdata heartbeat: `rsx-marketdata/tests/heartbeat_test.rs`
