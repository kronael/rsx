# rsx-risk

Risk engine binary. One instance per user shard.

## What It Does

Pre-trade margin checks, position tracking, fill processing,
funding payments, liquidation detection, and Postgres
persistence. Supports main/replica failover via advisory lock.

## Running

```
RSX_RISK_SHARD_ID=0 \
RSX_RISK_SHARD_COUNT=4 \
RSX_RISK_MAX_SYMBOLS=64 \
RSX_RISK_CORE_ID=4 \
RSX_RISK_WAL_DIR=./tmp/wal \
RSX_RISK_CAST_ADDR=127.0.0.1:9000 \
RSX_GW_CAST_ADDR=127.0.0.1:8000 \
RSX_ME_CAST_ADDR=127.0.0.1:9100 \
RSX_RISK_MARK_CAST_ADDR=127.0.0.1:9400 \
DATABASE_URL=postgres://... \
cargo run -p rsx-risk
```

Main/replica role is decided by advisory-lock acquisition at
startup, not an env var.

## Environment Variables

| Env Var | Purpose |
|---------|---------|
| `RSX_RISK_SHARD_ID` | Shard ID |
| `RSX_RISK_SHARD_COUNT` | Total shard count |
| `RSX_RISK_MAX_SYMBOLS` | Max symbol count |
| `RSX_RISK_CORE_ID` | CPU core to pin to |
| `RSX_RISK_WAL_DIR` | WAL directory |
| `RSX_RISK_CAST_ADDR` | casting bind address |
| `RSX_GW_CAST_ADDR` | Gateway casting address |
| `RSX_ME_CAST_ADDR` | ME casting address |
| `RSX_RISK_MARK_CAST_ADDR` | Mark price casting bind address |
| `RSX_RISK_REPLICA_ADDR` | Replica tip sync address |
| `DATABASE_URL` | Postgres connection string |

## Deployment

- One instance per shard; users map to shards via fixed virtual shards +
  a `shardmap` (`hash(user_id) % N_VSHARDS` → node), **not** live modulo —
  see `specs/2/28-risk.md` §Sharding & scale-out
- Pin to dedicated CPU core (via `core_affinity` + `RSX_RISK_CORE_ID`)
- Needs Postgres for state persistence and advisory lock
- Connects to Gateway, ME(s), and Mark via casting/UDP
- Run a replica alongside for failover (~500ms detection)

## Postgres & migrations

**Expected setup.** Risk needs one Postgres reachable at `DATABASE_URL`
(dev default `postgres://rsx:rsx@127.0.0.1:5432/rsx`). It holds the durable
state — accounts, positions, frozen-margin records — that the in-RAM hot
state is rebuilt from on boot. No other service writes risk's tables.
Postgres is **off the hot path**: the pinned loop never touches it; a tokio
sidecar drains the persist ring and write-behinds (see Internal architecture).

**Migrations run at boot, automatically.** Each node connects, takes a
dedicated `MIGRATION_LOCK_KEY` advisory lock (distinct from the per-shard
main lock), runs the pending SQL, then releases it. The lock **serializes
concurrent node boots** so migrations can't race — necessary under the
eager warm-standby protocol, where the per-shard main lock is won late
(after catch-up), so every node would otherwise migrate at once. Files
apply in order:

| File | Adds |
|------|------|
| `001_base_schema.sql` | accounts, positions, fills |
| `002_rename_tables.sql` | table renames (not idempotent — relies on the lock) |
| `003_users.sql` | users table |
| `004_frozen_orders.sql` | frozen-margin durable records |

**Boot sequence** — identical for a fresh shard, a replica, or a migration
target; there is no separate path:

1. Connect to Postgres; run migrations under `MIGRATION_LOCK_KEY`.
2. Load state from Postgres (accounts/positions) **without** the shard main
   lock — every node is a warm candidate.
3. Replay the local WAL, then warm-catch-up from the ME replication stream
   to the tip.
4. Once caught up, take the non-blocking per-shard advisory lock. Win → go
   `Live`; lose → stay a warm standby and retry.

Moving a vshard between nodes (migration) reuses exactly this path.

## Internal architecture

- Single pinned hot thread driving `Shard::run_once`
- 7 SPSC rings (rtrb) between hot thread and helpers:
  fill, order, mark, bbo (consumers); response, accepted
  (producers to gateway/ME); plus an 8192-slot persist ring
  to the sidecar
- **Persist sidecar:** dedicated tokio task with its own
  Postgres client. Drains `PersistEvent` from the ring and
  writes accounts/positions/fills behind the hot thread.
  Ring full → hot path stalls (per WAL.md backpressure rule).

## Testing

```
cargo test -p rsx-risk
cargo test -p rsx-risk -- --test-threads=1
```

Use `--test-threads=1` for tests with global state.

20 test files covering: account, cast_ingest, fees, funding,
insurance, insurance_liquidation_e2e, insurance_persist,
liquidation, liquidator_e2e, margin, margin_recalc,
me_cast_addrs, missing_integration, persist, position, price,
replica, replication_e2e, shard, shard_e2e.
See `specs/2/42-testing-risk.md`.

## Dependencies

- `rsx-types` -- shared types
- `rsx-cast` -- WAL, casting, replication consumer
- Postgres (runtime)

## Gotchas

- Frozen margin is in-memory only. On crash, frozen margin
  for in-flight orders is lost. Orders will time out at
  the gateway.
- Advisory lock ensures single-writer. If two mains acquire
  the lock simultaneously (split brain), data corruption is
  possible. Postgres advisory locks prevent this.
- Replica buffers fills in memory. Long outages can grow
  the buffer. There is no cap.
- `--test-threads=1` is required for some tests due to
  shared DashMap/RwLock global state.

## How to read this crate

- **What** — this README (running, env vars, deployment) +
  `specs/2/28-risk.md` (formal spec).
- **How** — [ARCHITECTURE.md](ARCHITECTURE.md): main loop,
  margin calculation, position tracking, funding,
  liquidation, persistence, replication.
- **Why** — [`notes/`](notes/): design rationale for
  non-obvious choices (SPSC ring usage, the persist-sidecar
  boundary, UDS-vs-shared-mem transport study). Read when a
  decision looks arbitrary.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) -- main loop, margin
  calculation, position tracking, funding, liquidation,
  persistence, replication
- `specs/2/28-risk.md`
