# PROJECT.md — Dashboards (Ship All 5)

## Goal

Finalize sketches and ship all 5 dashboards per user
directive. Keep them **simple and user-friendly** — no
over-engineering. Each dashboard is a focused UI for its
audience:

1. **Support dashboard** (7-dashboard.md) — user-facing
   admin for customer support operations
2. **Health dashboard** (12-health-dashboard.md) — fleet
   health + metrics for ops
3. **Management dashboard** (14-management-dashboard.md) —
   top-level index / platform shell (partial today)
4. **Playground dashboard** (23-playground-dashboard.md) —
   dev dashboard (mostly shipped; finalize gaps)
5. **Risk dashboard** (27-risk-dashboard.md) — risk ops
   (margin oversight, liquidation admin)

## Non-goals

- RBAC / fine-grained roles (v1: env-gated, single admin)
- Multi-tenancy
- Real-time streaming (v1: HTMX polling every 2-5s)

## IO Surfaces

- React SPA(s) in rsx-webui OR HTMX pages in rsx-playground
- Gateway REST (08-REST-ENDPOINTS) for user-scoped data
- Postgres read replicas for aggregate ops data
- Audit log table (new) for admin actions

## Design constraints

- **Simple**: one HTML page per dashboard, Tailwind +
  HTMX. Avoid SPA unless React already serves similar view.
- **User-friendly**: lead with what user needs; no empty
  state overload; action buttons labeled by verb.
- **Safe**: all write actions require x-confirm: yes header.
- **Audited**: every POST/PATCH/DELETE logs to audit table.
- **Gated**: env var `DASHBOARD_MODE=production|dev` gates
  write endpoints.

## Tasks

### Phase A — Finalize specs

For each of 5 dashboard specs, finalize to a shippable
design:

- **A1**: 7-dashboard.md (support) — pick concrete
  endpoints + UI surfaces. Scope: find user, view
  positions/orders/fills, adjust collateral, freeze/unfreeze
  account, view audit log.
- **A2**: 12-health-dashboard.md — pick metrics: per-process
  up/down, RSS memory, CPU, event throughput, WAL lag.
  Derive from existing `/x/processes`, `/x/key-metrics`,
  `/x/wal-status` endpoints.
- **A3**: 14-management-dashboard.md — top-level shell:
  links to all dashboards, system health traffic light,
  version info. Simple.
- **A4**: 23-playground-dashboard.md — close remaining gaps:
  add CMP flows screen (`/x/cmp-flows` already exists, just
  expose as tab), align API base paths with spec or update
  spec, document actual `PLAYGROUND_MODE` behavior.
- **A5**: 27-risk-dashboard.md — finalize: pause/resume
  trading per symbol, force liquidation review, insurance
  fund balance, margin parameters table.

### Phase B — Implementation per dashboard

- **B1**: Support dashboard — new routes in rsx-playground
  or rsx-webui? Playground feels right (it's an admin
  tool). Add `/support` page + `/api/support/*` endpoints.
- **B2**: Health dashboard — extend `/overview` page in
  playground or new dedicated `/health` page. Reuse
  existing HTMX partials.
- **B3**: Management dashboard — simplest. New landing
  `/management` page with status cards + links.
- **B4**: Playground gaps — add CMP flows tab, align
  naming.
- **B5**: Risk dashboard — new `/risk-ops` page. Add
  pause/resume endpoints in rsx-gateway (admin-only JWT
  scope).

### Phase C — Audit log + safety

- **C1**: Create `audit_log` Postgres table (event_ts,
  actor_user_id, module, action, target, payload JSON).
- **C2**: Replace stdout `audit_log()` in playground with
  DB write. Keep stdout as fallback when DB unreachable.
- **C3**: All write actions from dashboards insert row.

### Phase D — Tests

- **D1**: Playwright tests per dashboard (happy path +
  one failure mode).
- **D2**: Python e2e tests for audit log insertion.
- **D3**: Ensure all 5 dashboards render without JS errors
  in Chromium headless.

### Phase E — Update specs

Mark all 5 dashboard specs as `status: shipped`. Cross-ref
shared patterns (auth model, audit log schema) in
14-management-dashboard.md as the canonical platform spec.

## Acceptance

- All 5 dashboards render and serve their documented
  features
- Audit log table populated by at least one action per
  dashboard
- Playwright suite covers each dashboard with ≥2 tests
- Specs marked shipped, content matches code
- User-friendliness check: untrained operator can find
  user + freeze account in <60s

## Out of scope / follow-up

- RBAC with real role enforcement (v2)
- Real-time push (WS) — v2
- Mobile layouts — v2
- Historical trend charts beyond current widgets
