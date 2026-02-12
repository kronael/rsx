# Health Dashboard (Systems Ops)

## 1. Purpose

Systems operations dashboard for platform health:

- host and process load visibility
- service latency/error health
- resource saturation early warning
- incident triage entrypoint

This dashboard is infra/ops-centric, not user-support or risk-control workflow.

---

## 2. Scope

Read-only in v1.

Primary metrics:

- CPU usage (host + process)
- memory usage (RSS, heap, swap pressure)
- disk usage and IO latency
- network throughput and packet errors/drops
- service request rates, error rates, p95/p99 latency
- queue/ring buffer depth and backpressure flags
- WAL flush lag, replay lag, DB write lag
- open file descriptors, thread count, restart count

Views:

1. fleet overview
2. per-service drilldown (`gateway`, `risk`, `matching`, `marketdata`, `mark`, `dxs`, `postgres`)
3. incident timeline + alert state

---

## 3. API

Base path: `/v1/api/health`

- `GET /overview`
- `GET /services`
- `GET /services/{service}/metrics`
- `GET /hosts`
- `GET /hosts/{host}/metrics`
- `GET /alerts`
- `GET /timeseries?metric=&from=&to=&step=`

Pagination required for event/alert feeds.

---

## 4. RBAC

| Action | viewer | ops | admin | auditor |
|---|---:|---:|---:|---:|
| read_health | Y | Y | Y | Y |
| acknowledge_alert | N | Y | Y | N |

No write controls on exchange state in this dashboard.

This module does not include scenario execution or fault injection.

---

## 5. Alert Classes

- `P0`: data-loss/correctness risk
- `P1`: trading degraded/unavailable
- `P2`: redundancy degraded

Alert state machine:

`firing -> acknowledged -> resolved`

---

## 6. Acceptance

1. ops can identify saturation source (CPU/memory/disk/network) in <60s
2. ops can identify failing service and top error class in <60s
3. dashboards show lag/backpressure metrics for all core services
