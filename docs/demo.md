# RSX 60-Second Demo

Boot the RSX exchange from a clean slate and see live order
flow in under a minute. Single command, single dashboard.

## Prerequisites

- Built debug binaries (`cargo build --workspace`).
- Postgres reachable at `postgres://rsx:rsx@127.0.0.1:5432/rsx`
  (the Playground runtime bootstraps the schema).
- Python venv at `rsx-playground/.venv` with deps installed
  (one-time: `make prepare`).

## Steps

```bash
# Start Playground and the minimal E2E RSX stack
./rsx-playground/playground demo minimal

# Optional: start the maker so the book has quotes
curl -X POST 'http://localhost:49171/api/maker/start' -H 'x-confirm: yes'

# Watch live depth
open http://localhost:49171/        # Process Control + Book
# or, for the SPA Trade UI:
open http://localhost:49171/trade/
```

Within ~30 seconds the order book at symbol PENGU (id=10)
populates with two-sided quotes around mid=50000 and the
Trade UI's connection dot turns green.

## Verifying

```bash
# Maker status (orders_placed grows ~20/sec):
curl -s http://localhost:49171/tmp/maker-status.json \
  || curl -s http://localhost:49171/api/maker/status

# WAL records flowing (count grows over time):
cargo run -q -p rsx-cli -- dump tmp/wal/pengu/10/10_active.wal | wc -l

# All processes running:
curl -s http://localhost:49171/api/processes \
  | python3 -c 'import json,sys; [print(p["name"], p["state"]) for p in json.load(sys.stdin)]'
```

## Stopping

```bash
curl -X POST http://localhost:49171/api/processes/all/stop -H 'x-confirm: yes'
./rsx-playground/playground stop
```

## Auth in the demo

`./rsx-playground/playground demo` injects
`RSX_GW_JWT_SECRET=rsx-dev-secret-not-for-prod` for the
playground server when the operator hasn't already set one;
the same secret is emitted by the Playground runtime plan
when it spawns gateway / risk / ME. The maker, stress
client, and Trade UI all mint JWTs against that secret.
Production must override `RSX_GW_JWT_SECRET`; the demo
value is not safe for any internet-facing deploy.

## Troubleshooting

- **Gateway panics with `RSX_GW_JWT_SECRET must be set`** —
  you started the gateway outside Playground or unset the env. Re-run
  from `./rsx-playground/playground demo minimal`.
- **Empty book / WAL** — historically caused by the
  frozen-margin leak (fixed in commit `9ca6f10`). If you see
  it, check that migration `004_frozen_orders.sql` ran
  against your Postgres and that no stale `accounts.frozen_margin`
  column exists.
- **Trade UI dot stays red** — the `/ws/private` proxy needs
  `PLAYGROUND_ALLOW_INSECURE_USER_ID=1` (set by default in the
  Playground dev path) to mint a guest JWT for the unauthenticated
  Trade UI session.

## Security caveats (dev-only flags)

The dev path uses two flags that MUST be cleared for any
internet-facing deploy:

- `RSX_GW_JWT_SECRET=rsx-dev-secret-not-for-prod` — replace
  with a real secret minted by the auth service. The
  playground server, market maker, and stress client all
  fail-fast if the env var is unset; the launcher injects
  the dev value if it isn't set in the parent shell.
- `PLAYGROUND_ALLOW_INSECURE_USER_ID=1` — when set, any
  loopback caller can spoof user identity via the
  `x-user-id` request header. The server prints a WARN line
  on first use. Production must leave this unset.

For current state and the in-flight surface-honesty work,
see [PROGRESS.md](../PROGRESS.md) and
[.ship/12-SHOWCASE-HONEST/](../.ship/12-SHOWCASE-HONEST/).
