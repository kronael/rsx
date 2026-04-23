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
RSX_RISK_CMP_ADDR=127.0.0.1:9000 \
RSX_GW_CMP_ADDR=127.0.0.1:8000 \
RSX_ME_CMP_ADDR=127.0.0.1:9100 \
RSX_RISK_MARK_CMP_ADDR=127.0.0.1:9400 \
DATABASE_URL=postgres://... \
cargo run -p rsx-risk
```

For replica mode, add `RSX_RISK_IS_REPLICA=true`.

## Environment Variables

| Env Var | Purpose |
|---------|---------|
| `RSX_RISK_SHARD_ID` | Shard ID |
| `RSX_RISK_SHARD_COUNT` | Total shard count |
| `RSX_RISK_MAX_SYMBOLS` | Max symbol count |
| `RSX_RISK_IS_REPLICA` | `true` for replica mode |
| `RSX_RISK_CORE_ID` | CPU core to pin to |
| `RSX_RISK_WAL_DIR` | WAL directory |
| `RSX_RISK_CMP_ADDR` | CMP bind address |
| `RSX_GW_CMP_ADDR` | Gateway CMP address |
| `RSX_ME_CMP_ADDR` | ME CMP address |
| `RSX_RISK_MARK_CMP_ADDR` | Mark price CMP bind address |
| `RSX_RISK_REPLICA_ADDR` | Replica tip sync address |
| `DATABASE_URL` | Postgres connection string |

## Deployment

- One instance per shard (user_id % shard_count == shard_id)
- Pin to dedicated CPU core
- Needs Postgres for state persistence and advisory lock
- Connects to Gateway, ME(s), and Mark via CMP/UDP
- Run a replica alongside for failover (~500ms detection)

## Testing

```
cargo test -p rsx-risk
cargo test -p rsx-risk -- --test-threads=1
```

Use `--test-threads=1` for tests with global state.

17 test files covering: account, fees, funding, insurance,
liquidation, main loop, margin, order processing, persist,
position, price, replica, replication e2e, shard, shard e2e,
and more. See `specs/2/42-testing-risk.md`.

## Dependencies

- `rsx-types` -- shared types
- `rsx-dxs` -- WAL, CMP, DXS consumer
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

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) -- main loop, margin
  calculation, position tracking, funding, liquidation,
  persistence, replication
- `specs/2/28-risk.md`
