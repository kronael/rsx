---
status: shipped
---

# Risk Ops Dashboard (Exchange)

## 1. Purpose

Risk/operations dashboard for exchange-wide controls:

- monitor overall risk and operational health
- monitor per-symbol and per-shard risk posture
- apply risk controls (halt/resume symbols, tune risk parameters)

This dashboard is exchange-centric, not user-support-centric.

---

## 2. Scope

Read scope:

- risk shard health and lag
- tip progression by symbol
- liquidation volume/events
- insurance fund state
- funding summaries
- symbol status and config versions
- operational alerts and backpressure state

Control scope (writes):

1. pause/resume symbol trading
2. update risk params per symbol (bounded)
3. set emergency guardrails (rate/limit clamps)

Out of scope:

- direct user collateral edits
- user freeze edits

---

## 3. API

Base path: `/v1/api/risk`

Read endpoints:

- `GET /symbols`
- `GET /symbols/{symbol_id}/status`
- `GET /symbols/{symbol_id}/risk-metrics`
- `GET /symbols/{symbol_id}/liquidations?limit=&cursor=`
- `GET /shards`
- `GET /shards/{shard_id}/health`
- `GET /alerts`
- `GET /audit?actor=&action=&target=&limit=&cursor=`

Service liveness is global: `GET /health` (not namespaced).

Write endpoints:

- `POST /symbols/{symbol_id}/pause`
- `POST /symbols/{symbol_id}/resume`
- `POST /symbols/{symbol_id}/risk-params`
- `POST /controls/emergency-clamp`

Write requirements:

- JWT role auth (risk-admin)
- `X-Request-Id`
- `Idempotency-Key`
- mandatory `reason` + `ticket_id`
- config `expected_version`
- all write flags disabled by default

---

## 4. RBAC

| Action | viewer | risk_operator | risk_admin | auditor |
|---|---:|---:|---:|---:|
| read_risk_state | Y | Y | Y | Y |
| read_audit | N | N | Y | Y |
| pause_resume_symbol | N | Y | Y | N |
| update_risk_params | N | N | Y | N |
| emergency_clamp | N | N | Y | N |

Deny by default for all unlisted actions.

---

## 5. Safety Contracts

### 5.1 pause/resume symbol

Preconditions:

- symbol exists
- actor authorized
- no conflicting in-flight control op

Postconditions:

- symbol status persisted
- config/control event emitted
- audit row inserted atomically

### 5.2 update risk params

Preconditions:

- bounded parameter checks
- `expected_version` matches

Postconditions:

- config version increments
- applied event emitted
- audit row inserted atomically

### 5.3 emergency clamp

Preconditions:

- action approved by risk_admin policy

Postconditions:

- clamp persisted + active
- expiry policy set
- audit row inserted atomically

---

## 6. Operational KPIs

Must display at minimum:

- active symbols / halted symbols
- per-symbol liquidation rate
- per-symbol insurance balance
- shard backpressure/lag
- config apply lag
- alert severity counts

---

## 7. Rollout

### v1a

- read-only risk ops dashboard

### v1b

- symbol pause/resume controls enabled

### v1c

- bounded risk param updates + emergency clamps

All write phases must be feature-flagged.

---

## 8. Acceptance

1. ops can identify symbol/shard risk anomalies in seconds
2. ops can halt/resume a symbol with full audit trace
3. risk parameter changes are versioned, bounded, and auditable

## 9. Exclusions

- no scenario launch/reset
- no fault injection
- no dev-only playground actions
