# Playground Dashboard (Dev/Test Control Plane)

## 1. Purpose

Developer/testing dashboard for local and staging playground workflows.

This module is for rapid observe/act/verify loops:

- observe process/book/risk/WAL/CMP state
- act: run scenarios, inject faults, submit scripted flows
- verify invariants and replay outcomes

It is not part of production support/risk/health control planes.

---

## 2. Scope and Environment

Primary environments:

- local developer machines
- isolated test/staging playground environment

Out of scope:

- direct production access
- user-support financial corrections
- production risk controls

Hard rule:

- Playground is disabled in production.

Mode flags:

- `PLAYGROUND_MODE=local|staging`
- `PLAYGROUND_WRITES_ENABLED=true|false`

Default: local enabled, staging disabled until explicitly enabled.

---

## 3. Capability Model (Observe / Act / Verify)

## 3.1 Observe

- process states and dependencies
- core affinity and resource usage
- book snapshots and BBO
- risk user/symbol state summaries
- WAL/tip/replay lag and file status
- CMP flow and gap/NAK statistics
- logs and event timeline

## 3.2 Act

- process lifecycle controls (start/stop/restart/kill)
- scenario launch/reset controls
- fault injection (pause, kill, network delay/drop, WAL corruption simulation)
- test order flows and scripted traffic
- snapshot/replay operations

## 3.3 Verify

- invariants (fills/order completion/FIFO/seq monotonicity/no crossed book)
- reconciliation checks (risk vs fills, shadow vs ME)
- latency regressions and budget checks

---

## 4. API

Base path: `/v1/api/play`

### Read endpoints

- `GET /processes`
- `GET /books`
- `GET /books/{symbol_id}`
- `GET /risk/users/{user_id}`
- `GET /wal/{stream}/status`
- `GET /wal/{stream}/events`
- `GET /cmp/flows`
- `GET /metrics`
- `GET /logs`
- `GET /events`
- `GET /verify/invariants`

### Action endpoints

- `POST /processes/{name}/start`
- `POST /processes/{name}/stop`
- `POST /processes/{name}/kill`
- `POST /processes/{name}/restart`
- `POST /scenarios/launch`
- `POST /scenarios/reset`
- `POST /faults/{kind}/inject`
- `POST /orders/test`
- `POST /wal/replay`
- `POST /verify/run`

---

## 5. Safety Rules

1. Playground writes/actions must be blocked in production.
2. Actions must require explicit environment guard confirmation in staging.
3. Every action writes to shared `audit_log` with `module=playground`.
4. Idempotency key required for non-read endpoints.

---

## 6. Data Sources

- process manager (`run.py` / start script state)
- service HTTP/CMP health endpoints
- Postgres read queries for state checks
- WAL files and DXS endpoints
- in-process metrics exporters

---

## 7. UI Surfaces

Required screens (initial):

1. Overview
2. Topology
3. Book
4. Risk
5. WAL
6. CMP
7. Logs
8. Invariants
9. Scenarios
10. Fault Injection

These map to `playground/SCREENS.md` and `playground/SPEC.md`.

---

## 8. Auth Model

- No RBAC required in Playground (local and staging).
- Access is environment-gated, not role-gated.
- If staging access is exposed beyond localhost, enforce network allowlist.

---

## 9. Acceptance

1. Developer can bootstrap full playground and inspect all core states from one UI.
2. Developer can run at least 3 canned scenarios and see verify results.
3. Destructive actions are environment-gated and auditable.
4. Invariant checks are informational (non-blocking) and report pass/fail with enough detail to debug.
