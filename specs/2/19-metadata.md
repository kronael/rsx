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
- `min_notional` (smallest `price * qty` of a new order, owned and
  enforced by risk, in the unit `10^-(price_decimals + qty_decimals)`
  quote; 1 USD is `10^(price_decimals + qty_decimals)`, set to that for
  every preset by risk's `007_min_order_notional.sql`; risk requires every
  symbol's `price_decimals + qty_decimals` to be 10)
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

- Matching engine polls the schedule every `RSX_ME_CONFIG_POLL_S` seconds (default 10).
- It queues future entries per symbol and applies when:
  - `effective_at_ms <= now_utc_ms`, and
  - `config_version > current_version`.
- Once applied, **never revert**, even if clocks drift.
- Emit `CONFIG_APPLIED(symbol_id, config_version, effective_at_ms, applied_at_ns)` on the normal stream.

## Propagation

- Risk and Gateway update caches on `CONFIG_APPLIED` events.
- Older versions are ignored.
- Gateway may validate basic constraints with its cached config, but matcher is authoritative.

**Cold start bootstrap:** ME writes each applied config's tick,
lot and decimals to `symbol_config_applied` alongside the
`CONFIG_APPLIED` event and reloads from it on cold start; ME is the
only writer to that table. Risk cold-starts from
`symbol_config_schedule` instead — the highest version per symbol
whose `effective_at_ms` has passed — because the risk columns
(margin rates, leverage, fees) live only there; on `CONFIG_APPLIED`
it fetches exactly the version the matcher named. Both readers
therefore converge on the version the matcher applied, whether or
not they saw the stream event.

## Notes

- v1 allows changes to all fields except `symbol_id`.
- If v2 ever needs immutable/slow‑changing groups, split schedule tables by mutability.
