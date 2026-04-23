---
status: shipped
---

# Management Dashboards (Split)

This spec is intentionally split into four separate dashboards:

1. User support dashboard:
- `DASHBOARD.md`

2. Exchange risk/ops dashboard:
- `RISK-DASHBOARD.md`

3. Systems health dashboard:
- `HEALTH-DASHBOARD.md`

4. Playground dashboard (dev/test):
- `PLAYGROUND-DASHBOARD.md`

Reason for split:

- support workflows are user-centric (balances, positions, trading history, corrective user actions)
- risk/ops workflows are exchange-centric (symbol controls, risk parameters, operational health)
- systems health workflows are infra-centric (load, CPU, memory, disk, network, service latency/errors)
- playground workflows are dev/test-centric (scenario control, fault injection, observe/act/verify loops)

Shared requirements across all dashboards:

- existing Postgres backend (no new DB stack)
- shared backend service (single binary) with per-module feature flags
- dashboard shell served from `/`
- all APIs served under `/v1/api/*`
- service liveness endpoint is `GET /health`
- idempotent writes
- atomic business write + audit insert where writes exist
- feature flags for any write capability

## Platform Decisions

1. One shell UI for all modules (`Vite + Tailwind + shadcn/ui`).
2. One backend service with module loading flags.
3. Playground is dev-only and blocked in production.
4. Scenarios and all dev features live only in Playground.
5. Shared `audit_log` table with a required `module` field.
