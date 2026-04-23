---
status: shipped
---

# NAMING

Canonical names across DB tables, API endpoints, HTMX partials,
and Rust types. One name per concept. No aliases.

## Concepts

| Concept         | DB table      | Endpoint          | Rust (ring)   | Rust (persist)      |
|-----------------|---------------|-------------------|---------------|---------------------|
| Fills           | `fills`       | `/x/fills`        | `FillEvent`   | `FillRecord`        |
| Positions       | `positions`   | `/x/risk-user`    | `Position`    | `Position`          |
| Accounts        | `accounts`    | `/x/risk-user`    | `Account`     | `Account`           |
| Funding         | `funding`     | `/x/funding`      | —             | `FundingRecord`     |
| Liquidations    | `liquidations`| `/x/liquidations` | —             | `LiquidationRecord` |
| Insurance fund  | `insurance_fund` | —              | `InsuranceFund` | `InsuranceFund`   |
| Tips            | `tips`        | —                 | —             | —                   |

## Rules

- **No `_events` suffix on tables.** `liquidation_events` → `liquidations`.
  Each row IS an event; the suffix is redundant.
- **No `_payments` suffix on tables.** `funding_payments` → `funding`.
- **No `Persist` prefix on DB structs.** `PersistFill` → `FillRecord`.
- **No `Record` suffix on ring types.** `FillEvent` stays `FillEvent`
  (it's a hot-path repr(C) ring message, not a DB row).
- **`Event` suffix only on ring/CMP messages**: `FillEvent`, `OrderDoneEvent`,
  `BboUpdate`. Not on DB structs, not on tables.
- **Endpoint = table name** where there's a 1:1 mapping.

## Applied changes (sync log)

### Migrations
- `002_rename_tables.sql`: renames `liquidation_events` → `liquidations`
  and `funding_payments` → `funding`.

### rsx-playground/server.py
- `/x/live-fills` → `/x/fills`
- SQL `FROM liquidation_events` → `FROM liquidations`
- Fixed missing space in `"SELECT * FROM positions" "WHERE ..."` (x2)

### rsx-playground/pages.py
- `hx-get="./x/live-fills"` → `hx-get="./x/fills"`

### rsx-risk/src/persist.rs
- `PersistFill` → `FillRecord`
- `FundingPaymentRecord` → `FundingRecord`
- `LiquidationEventRecord` → `LiquidationRecord`
- `PersistEvent::FundingPayment` → `PersistEvent::Funding`
- `PersistEvent::LiquidationEvent` → `PersistEvent::Liquidation`

### rsx-risk/src/shard.rs + tests
- Same Rust renames as above propagated.
