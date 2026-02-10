# rsx-matching Architecture

Matching engine process. One instance per symbol. Receives
orders from Risk via CMP/UDP, matches against the orderbook,
fans out events. See `specs/v1/MATCHING.md`.

## Module Layout

| File | Purpose |
|------|---------|
| `main.rs` | Binary: CMP setup, WAL init, main loop, event routing |
| `wire.rs` | `OrderMessage`, `EventMessage` -- CMP wire types |
| `dedup.rs` | `DedupTracker` -- sliding-window duplicate detection |
| `fanout.rs` | `drain_and_fanout()` -- SPSC ring routing (future tile mode) |
| `config.rs` | `poll_scheduled_configs()`, `write_applied_config()` -- Postgres config polling |
| `wal_integration.rs` | `write_events_to_wal()`, `flush_if_due()` |

## Key Types

- `OrderMessage` -- `#[repr(C)]` inbound order from Risk
- `EventMessage` -- outbound event enum for SPSC fanout
- `DedupTracker` -- HashMap + VecDeque for 5-minute dedup
- `ScheduledConfig` -- config version from database

## Main Loop

Tight busy-spin on a pinned core:

1. Receive `OrderMessage` from Risk via `CmpReceiver` (UDP)
2. Dedup check (5-minute sliding window)
3. Write `ORDER_ACCEPTED` to WAL
4. Call `process_new_order()` from rsx-book
5. Write events (Fill, OrderInserted, etc.) to WAL
6. Send events to Risk via `CmpSender` (all events)
7. Send events to Marketdata via `CmpSender`
   (Fill, Insert, Cancel only)
8. Poll for config updates from Postgres (every 10 minutes)
9. Flush WAL every 10ms

## Config Hot Reload

Config changes (tick_size, lot_size) are polled from
`symbol_config_schedule` table and applied live. A
`CONFIG_APPLIED` record is written to WAL and sent to
downstream consumers.

## Event Fanout

Fixed array `[Event; 10_000]` on Orderbook struct. Reset per
cycle. Two independent CmpSenders:
- ME -> Risk: fills, BBO, order done/failed
- ME -> Marketdata: inserts, cancels, fills
