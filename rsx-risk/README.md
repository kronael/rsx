# rsx-risk

Risk engine binary. One instance per user shard.

## What It Does

Pre-trade margin checks, position tracking, fill processing,
funding payments, liquidation, the insurance fund, and Postgres
persistence. Every process is a warm candidate main: it catches up
from the matching engines' replication streams and goes live only
when it wins the per-shard advisory lock.

## Running

```
RSX_RISK_SHARD_ID=0 \
RSX_RISK_SHARD_COUNT=1 \
RSX_RISK_MAX_SYMBOLS=16 \
RSX_RISK_CORE_ID=4 \
RSX_RISK_WAL_DIR=./tmp/wal \
RSX_RISK_CAST_ADDR=127.0.0.1:9101 \
RSX_GW_CAST_ADDR=127.0.0.1:9102 \
RSX_ME_CAST_ADDRS=127.0.0.1:9110,127.0.0.1:9111 \
RSX_RISK_ME_RECV_ADDRS=10=127.0.0.1:28310,11=127.0.0.1:28311 \
RSX_ME_REPLICATION_ADDRS=10=127.0.0.1:9510,11=127.0.0.1:9511 \
RSX_RISK_MARK_CAST_ADDR=127.0.0.1:9105 \
RSX_MARK_CAST_ADDR=127.0.0.1:9106 \
DATABASE_URL=postgres://rsx:rsx@127.0.0.1:5432/rsx \
cargo run -p rsx-risk
```

Main/standby role is decided by advisory-lock acquisition after
catch-up, not by an env var. A malformed value for any knob below
fails the boot; nothing falls back to a default silently.

## Environment Variables

| Env Var | Purpose |
|---------|---------|
| `RSX_RISK_SHARD_ID` | Shard ID (default 0) |
| `RSX_RISK_SHARD_COUNT` | Total shard count (default 1) |
| `RSX_RISK_MAX_SYMBOLS` | Size of the per-symbol arrays; must exceed every `symbol_id` (default 64) |
| `RSX_RISK_CORE_ID` | CPU core to pin the hot thread to (unset = unpinned) |
| `RSX_RISK_WAL_DIR` | Directory for replay tip files and the cast senders (default `./tmp/wal`) |
| `RSX_RISK_CAST_ADDR` | Cast receive address for gateway orders |
| `RSX_GW_CAST_ADDR` | Gateway cast address (responses and forwarded ME records) |
| `RSX_GW_REPLICATION_ADDR` | Gateway WAL replication endpoint for gap recovery; unset skips the gap |
| `RSX_ME_CAST_ADDRS` | ME cast addresses, comma-separated; `symbol_id = port - 9100` |
| `RSX_RISK_ME_RECV_ADDRS` | Per-symbol receive binds, `sid=addr,...` |
| `RSX_ME_REPLICATION_ADDRS` | Per-symbol ME WAL replication endpoints, `sid=addr,...` |
| `RSX_ME_CAST_ADDR`, `RSX_RISK_ME_RECV_ADDR`, `RSX_ME_REPLICATION_ADDR` | Singular forms for a one-symbol deployment |
| `RSX_RISK_ME_SEND_ADDR` | Optional bind for the ME-bound sender |
| `RSX_RISK_MARK_CAST_ADDR` | Mark price cast receive bind |
| `RSX_MARK_CAST_ADDR` | Mark process cast address |
| `RSX_MARK_REPLICATION_ADDR` | Mark WAL replication endpoint for gap recovery; unset skips the gap |
| `RSX_RISK_HEALTH_ADDR` | `/health`, `/ready`, `/metrics` listen address |
| `RSX_RISK_LEASE_POLL_MS`, `RSX_RISK_LEASE_RENEW_MS` | Lock poll interval while warm; renewal interval while live |
| `RSX_LIQUIDATION_BASE_DELAY_NS`, `RSX_LIQUIDATION_BASE_SLIP_BPS`, `RSX_LIQUIDATION_MAX_SLIP_BPS`, `RSX_LIQUIDATION_MAX_ROUNDS`, `RSX_LIQUIDATION_SOCIALIZED_COOLDOWN_NS` | Liquidation cascade knobs (see `specs/2/13-liquidator.md`) |
| `RSX_CAST_TLS_*` | Replication TLS material (`rsx_cast::TlsConfig`) |
| `DATABASE_URL` | Postgres connection string |

Per-symbol risk parameters are **not** environment variables; they
come from Postgres (below).

## Deployment

- One instance per shard; v1 routes `user_id % shard_count` (the
  vshard map in `specs/2/28-risk.md` §Sharding is forward design)
- Pin to a dedicated CPU core (`RSX_RISK_CORE_ID`)
- Needs Postgres for state, symbol config and the advisory lock
- Connects to Gateway, every ME, and Mark via casting/UDP; opens the
  ME replication streams over TLS for catch-up and gap recovery
- Run one or more extra instances with the same shard id for
  failover: they warm up and take the lock when the main drops it

## Postgres, migrations and symbol config

**Expected setup.** Risk needs one Postgres reachable at `DATABASE_URL`
(dev default `postgres://rsx:rsx@127.0.0.1:5432/rsx`). It holds the
durable state — accounts, positions, frozen-margin records, fills,
funding, liquidations, insurance fund, tips — that the in-RAM hot
state is rebuilt from on boot, plus the symbol catalog. Risk owns the
schema; the matching engine and the playground are readers. Postgres
is **off the hot path**: the pinned loop never touches it; a tokio
sidecar drains the persist ring and write-behinds (see Internal
architecture).

**Migrations run at boot, automatically, on every node, with no
lock.** Each file is version-guarded, idempotent and a single
transaction, so concurrent boots are safe (`migrations/CLAUDE.md`).
Files apply in order:

| File | Adds |
|------|------|
| `001_base_schema.sql` | accounts, positions, fills, tips, funding, insurance fund, liquidations |
| `002_rename_tables.sql` | `liquidation_events` → `liquidations`, `funding_payments` → `funding` |
| `003_users.sql` | users table |
| `004_frozen_orders.sql` | frozen-margin durable records |
| `005_symbol_config.sql` | `symbol_static`, `symbol_config_schedule`, `symbol_config_applied` |
| `006_seed_symbols.sql` | the eight playground symbols with a usable v1 risk config |
| `007_min_order_notional.sql` | a new version per symbol with `min_notional` raised to 1 USD (a higher operator minimum is kept) |

**Symbol risk config comes from `symbol_config_schedule`.** For each
symbol the active row is the highest `config_version` whose
`effective_at_ms` has passed; risk reads `initial_margin_rate_bps`,
`maintenance_margin_rate_bps`, `max_leverage`, `min_notional`,
`taker_fee_bps` and `maker_fee_bps` from it
(`config::load_symbol_configs`); a row with a NULL or non-positive
`min_notional` fails the load, and so does a row whose
`price_decimals + qty_decimals` is not `config::NOTIONAL_DECIMALS` (10):
risk sums `price * qty` products across symbols, so they share one unit.
A symbol with no row is unconfigured: every order on it is rejected with
`SymbolNotFound`. The seed migration is the preset; the playground
Config tab schedules new versions. A new version reaches a live shard
when the matching engine applies it and emits `CONFIG_APPLIED`: the
hot loop asks the persist sidecar to fetch that version and applies
it on the next tick (`config::SymbolRiskConfig`,
`RiskShard::apply_symbol_config`).

**Boot sequence** — identical for a fresh shard, a standby, or a
migration target; there is no separate path:

1. Connect to Postgres; run migrations.
2. Load state (accounts, positions, tips, insurance, frozen orders)
   and the active symbol config **without** the shard lock — every
   node is a warm candidate.
3. Warm-catch-up from each ME's replication stream to its tip,
   draining mark prices meanwhile.
4. Once caught up, take the non-blocking per-shard advisory lock.
   Win → reload symbol config, go `Live`; lose → stay warm and retry.

## Internal architecture

- One pinned hot thread (`cast_loop`) draining three cast receivers
  (ME per symbol, gateway, mark) straight into `RiskShard`; std
  non-blocking UDP busy-spin, no async runtime on that thread
- Three SPSC rings (rtrb) out of the shard: `response` and
  `accepted` back to the loop's cast senders, and an 8192-slot
  `persist` ring to the sidecar; one 64-slot `config` ring back from
  the sidecar
- **Persist sidecar:** its own OS thread and current-thread tokio
  runtime with its own Postgres client. Drains `PersistEvent` and
  write-behinds every 10 ms; answers `ConfigReload` requests over the
  config ring. Ring full → the hot path stalls (never drops)
- **Lease thread:** renews the advisory lock every
  `RSX_RISK_LEASE_RENEW_MS`; on loss the live loop tears down both
  sidecars and the node re-enters warm catch-up

## Testing

```
cargo test -p rsx-risk
make integration        # the #[ignore] testcontainers suites
```

Test files cover: account, cast_ingest, config, crash_guarantees,
failover_replay, fee, funding, insurance, insurance_liquidation_e2e,
insurance_persist, liquidation, liquidation_persist, liquidator_e2e,
margin, margin_recalc, me_cast_addrs, missing_integration,
orphan_freeze, persist, persist_resilience, position, price,
replay_drain, replication_e2e, shard, shard_e2e, symbol_config.
See `specs/2/42-testing-risk.md`.

## Dependencies

- `rsx-types` -- shared types
- `rsx-cast` -- casting, replication consumer
- `rsx-messages` -- wire records
- `rsx-health` -- health server and gauges
- Postgres (runtime)

## Gotchas

- The pre-trade freeze is in memory until the ME confirms the order;
  the durable `frozen_orders` row is written on `ORDER_ACCEPTED` and
  deleted on done/cancelled/failed. On restart, freezes are rebuilt
  from the ME replication stream's `ORDER_ACCEPTED` records.
- At most one main per shard is enforced by the Postgres advisory
  lock alone; catch-up only decides when a node tries for it.
- Gap recovery on the ME streams replays and applies every record;
  gateway and mark gaps are skipped when their replication endpoint
  is unset (client resend and latest-wins make that safe).
- `RSX_RISK_MAX_SYMBOLS` must be larger than every `symbol_id` on
  every node, including standbys; the per-symbol arrays are indexed
  by id.

## How to read this crate

- **What** — this README (running, env vars, deployment) +
  `specs/2/28-risk.md` (formal spec).
- **How** — [ARCHITECTURE.md](ARCHITECTURE.md): module layout, the
  live loop, margin calculation, position tracking, funding,
  liquidation, persistence, promotion and failover.
- **Why** — [`notes/`](notes/): design rationale for
  non-obvious choices (SPSC ring usage, the persist-sidecar
  boundary, UDS-vs-shared-mem transport study). Read when a
  decision looks arbitrary.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md)
- `specs/2/28-risk.md`, `specs/2/13-liquidator.md`
