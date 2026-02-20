# RSX Exchange E2E Integration

## Goal

Prove the exchange works end-to-end: maker provides liquidity, takers cross
orders, fills are delivered via WS, positions are accurate, and liquidation
fires on BBO updates without waiting for the next fill.

## Stack

- **Binaries (Rust):** `rsx-gateway`, `rsx-risk`, `rsx-matching`,
  `rsx-marketdata`, `rsx-mark`
- **Maker (Python):** `rsx-playground/market_maker.py`
- **Wire:** CMP/UDP (hot path), WAL replication over TCP (cold path)
- **Client API:** WEBPROTO WebSocket + REST (`specs/v1/WEBPROTO.md`,
  `RPC.md`, `MESSAGES.md`)

## Bugs to Fix (5 concrete)

| # | File | Fix |
|---|------|-----|
| 1 | `rsx-gateway/src/handler.rs:69-98` | Interleave WS read + outbound drain via monoio select or join |
| 2 | `rsx-matching/src/main.rs` CMP recv loop | Dispatch `RECORD_CANCEL_REQUEST` alongside `RECORD_ORDER_REQUEST` |
| 3 | `rsx-matching/src/main.rs` startup | Call WAL snapshot restore before accepting CMP messages |
| 4 | `rsx-risk/src/shard.rs:drain_stashed_bbos` | Scan exposed users for margin breach on every BBO/mark update |
| 5 | `rsx-playground/market_maker.py` | Evict filled cids via WS fills, align prices to tick size |

## Acceptance Criteria (8, all runnable)

| # | Name | Observable pass condition |
|---|------|--------------------------|
| 1 | Build | `cargo build` exits 0, zero unused warnings |
| 2 | Unit tests | `cargo test --workspace` all pass |
| 3 | WS fill round-trip | `ws_new_order_fill_update_complete`: both sides receive `WsFrame::Fill` ≤1 s |
| 4 | Cancel round-trip | `WsFrame::OrderUpdate{CANCELLED}` ≤1 s; order absent from `GET /x/book` |
| 5 | Restart safety | 3 resting orders survive ME kill+restart; new orders match against them |
| 6 | BBO liquidation | Risk shard emits liquidation CMP within one BBO cycle after margin breach |
| 7 | Maker integration | ≥5 bid + ask levels after 3 s; `orders_placed > 0`; crossing fill in WAL ≤2 s |
| 8 | Stress test | `accept_rate > 0.8` at 50 rps/10 s; no crash; WAL tip monotonic |

## IO Surfaces

- **Inbound:** WS frames (client orders, cancels), CMP UDP datagrams (ME↔risk)
- **Outbound:** WS push frames (fills, order updates), WAL records, REST responses
- **Startup:** WAL snapshot read → book state (criterion 5 depends on this)

## Constraints

- Debug builds only (`cargo build`, no `--release`)
- All timing assertions: wall-clock timeouts (1 s fills, 2 s WAL)
- Fixed-point i64 prices throughout; maker quotes must be tick-aligned
- No new crate dependencies without explicit justification
- `stress_client.py` must be created if absent (criterion 8 depends on it)

## Out of Scope

- Performance tuning, new features, or refactoring beyond the 5 listed bugs
- Release builds, benchmarking, or deployment changes
