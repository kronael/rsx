---
status: shipped
---

# User Management Dashboard (Support)

## 1. Purpose

Support-facing dashboard for user-level operations:

- inspect balances, positions, PnL, fills, funding, freezes
- investigate user complaints/incidents
- apply tightly controlled user-state corrections

This dashboard is user-centric, not exchange-risk-centric.

---

## 2. Scope

Read scope:

- `accounts`, `positions`, `fills`, `funding_payments`, `order_freezes`, `tips`
- derived views: unrealized PnL estimate, frozen-margin mismatch

Write scope (v1b minimal):

1. `adjust_collateral`
2. `freeze_upsert`
3. `freeze_delete`
4. `note_add`

Deferred:

- position correction
- direct frozen-margin set
- symbol/risk config edits

---

## 3. API

Base path: `/v1/api/support`

Read endpoints:

- `GET /users?query=&limit=&cursor=`
- `GET /users/{user_id}/account`
- `GET /users/{user_id}/positions`
- `GET /users/{user_id}/fills?symbol_id=&limit=&cursor=`
- `GET /users/{user_id}/funding?symbol_id=&limit=&cursor=`
- `GET /users/{user_id}/freezes`
- `GET /users/{user_id}/reconcile/frozen-margin`
- `GET /audit?actor=&action=&target=&limit=&cursor=`

Write endpoints:

- `POST /accounts/{user_id}/adjust-collateral`
- `POST /freezes/upsert`
- `POST /freezes/delete`
- `POST /notes`

Write requirements:

- JWT role auth
- `X-Request-Id`
- `Idempotency-Key`
- `reason` + `ticket_id`
- `expected_version` where applicable

---

## 4. RBAC

| Action | viewer | operator | admin | auditor |
|---|---:|---:|---:|---:|
| read_user_state | Y | Y | Y | Y |
| read_audit | N | N | Y | Y |
| write_note | N | Y | Y | N |
| adjust_collateral | N | N | Y | N |
| freeze_upsert | N | N | Y | N |
| freeze_delete | N | N | Y | N |

Deny by default for all unlisted actions.

---

## 5. Safety Contracts

### 5.1 adjust_collateral

Preconditions:

- account exists
- `expected_version` matches
- `abs(delta) <= ADMIN_MAX_COLLATERAL_DELTA`

Postconditions:

- collateral updated
- version incremented
- audit row inserted (same transaction)

### 5.2 freeze upsert/delete

Preconditions:

- valid order key
- row existence checks for delete

Postconditions:

- freeze row changed
- audit row inserted (same transaction)

---

## 6. Audit

Use shared `audit_log` table for all dashboard writes.

Required fields:

- module=`support`, actor, action, target, before_json, after_json, reason, ticket_id, request_id, idempotency_key, ts

Rule:

- business write + audit insert must be atomic.

---

## 7. Rollout

### v1a

- read-only screens + reconciliation + audit viewer

### v1b

- enable write endpoints behind flags:
- `SUPPORT_WRITES_ENABLED`
- `SUPPORT_ENABLE_ADJUST_COLLATERAL`
- `SUPPORT_ENABLE_FREEZE_MUTATIONS`

---

## 8. Acceptance

1. support can locate any user and inspect account/trading state in one place
2. support can reconcile frozen mismatch quickly
3. all writes are traced and idempotent
