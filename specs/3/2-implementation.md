---
status: reference
---

# v2 Implementation Notes (Observed)

This document captures notable implementation details that are now
material to how the system behaves. It does not replace v1 specs; it
records v1 code reality for future alignment.

## casting/WAL

- casting datagram = 16B `WalHeader` + payload.
- `WalHeader`: `record_type: u16`, `len: u16`, `crc32: u32`, `_reserved[8]`.
- Data payloads are `#[repr(C, align(64))]` and start with `seq: u64`.
- `CastSender::send` returns `false` if flow-control window is closed.
- `send_raw` bypasses `seq` assignment (control or pre-framed payloads).

## Gateway (rsx-gateway)

- WS auth: JWT in `Authorization` header; fallback `X-User-Id` (dev).
- No pre-trade pending ack; first update is from ME or risk `OrderFailed`.
- Validates tick/lot alignment using cached `symbol_configs`.
- Rate limits per IP and per user; circuit breaker on overload.
- Sends order lifecycle updates based on ME events:
  - `OrderInserted` -> status=1 (resting)
  - `OrderCancelled` -> status=2 (cancelled)
- `OrderDone` -> status mapped from `final_status`.
  - `OrderFailed` (from risk) -> status=3 with raw reason byte
- Heartbeats: server broadcasts every `heartbeat_interval_ms` and
  disconnects idle clients after `heartbeat_timeout_ms`.

## Risk (rsx-risk)

- casting/UDP between Gateway and Matching; no SPSC IPC between processes.
- Sends `OrderFailedRecord` to Gateway on pre-trade rejection.
- Forwards ME events to Gateway via casting: Fill, OrderInserted, Cancelled,
  Done, ConfigApplied.
- Tracks frozen margin per order_id; releases on Cancel/Done.
- Mark price input: listens on casting (`RSX_RISK_MARK_CAST_ADDR`) and updates
  `mark_prices` from `MarkPriceRecord`.
- BBO input: accepts `BboRecord` via casting and updates index prices.
- Replica mode exists:
  - Advisory lease in Postgres.
  - Tip sync over casting (record_type 0x20).
  - Buffers ME fills until promoted.

## Matching (rsx-matching)

- Receives `OrderMessage` via casting/UDP from risk.
- Emits events to risk and marketdata via casting/UDP.
- Writes WAL via `WalWriter` and serves replication.
- Emits `ConfigAppliedRecord` at startup (config_version=1).

## Marketdata (rsx-marketdata)

- Receives ME events via casting/UDP (Fill, Insert, Cancel).
- Optional replay bootstrap from replication (`replay_addr`, `tip_file`).
- Seq-gap detection per symbol; triggers snapshot resend.
- Per-connection heartbeat tracking and server broadcast.
- Shadow book keyed by `order_id` (maintains map for cancel/fill).

## Mark (rsx-mark)

- Aggregates external feeds, writes MarkPrice to WAL.
- Exposes replication server; sends MarkPrice over casting to risk.
- Source connectors push to aggregation via SPSC.
