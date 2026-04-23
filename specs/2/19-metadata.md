---
status: shipped
---

# METADATA (Symbol Config Scheduling)

This spec defines how symbol configuration is scheduled and propagated. Matching engine is the source of truth for **applied** configs and emits an event when a config takes effect.

## Goals

- Single immutable key: `symbol_id` (array index).
- All other fields can change in v1.
- Scheduled changes apply by timestamp, with monotonic versioning.
- Risk and Gateway sync from matcher events (not direct DB polling).

## Data Model

### 1) `symbol_static`

Immutable identity and UI fields.

Fields:
- `symbol_id` (PK, immutable)
- `symbol_name`
- `description`

### 2) `symbol_config_schedule`

Scheduled config changes (timestamped).

Fields:
- `symbol_id` (FK)
- `config_version` (monotonic per symbol)
- `effective_at_ms` (UTC)
- `tick_size`
- `lot_size`
- `price_decimals`
- `qty_decimals`
- `status` (active/paused/halted)
- `min_notional`
- `max_order_qty`
- `maker_fee_bps`
- `taker_fee_bps`
- `initial_margin_rate_bps`
- `maintenance_margin_rate_bps`
- `max_leverage`
- `funding_interval_sec`
- `funding_rate_min_bps`
- `funding_rate_max_bps`
- `created_at_ms`

## Application Semantics

- Matching engine polls the schedule **every 10 minutes**.
- It queues future entries per symbol and applies when:
  - `effective_at_ms <= now_utc_ms`, and
  - `config_version > current_version`.
- Once applied, **never revert**, even if clocks drift.
- Emit `CONFIG_APPLIED(symbol_id, config_version, effective_at_ms, applied_at_ns)` on the normal stream.

## Propagation

- Risk and Gateway update caches on `CONFIG_APPLIED` events.
- Older versions are ignored.
- Gateway may validate basic constraints with its cached config, but matcher is authoritative.

**Cold start bootstrap:** ME writes each applied config to a
Postgres table (`symbol_config_applied`) alongside the
`CONFIG_APPLIED` event. On cold start, Risk and Gateway load
current configs from this table (not from the schedule). This
ensures new instances get current config even if they missed
the CONFIG_APPLIED stream event. ME is the only writer to
this table.

## Notes

- v1 allows changes to all fields except `symbol_id`.
- If v2 ever needs immutable/slow‑changing groups, split schedule tables by mutability.
