# PROJECT.md — Gateway REST Endpoints (Full)

## Goal

All 5 REST endpoints (account, positions, orders, fills,
funding) work end-to-end with JWT auth, rate limits, CORS,
proper error responses. Unit + integration + e2e tests.
Productive quality.

## Architecture decision

**Playground is the REST host**, not rsx-gateway.

Rationale:
- Endpoints already implemented in `rsx-playground/server.py`
  lines 5867-6074 (FastAPI + asyncpg)
- rsx-gateway uses monoio (no tokio-postgres compatibility);
  adding Postgres to monoio is deep-water engineering
- Split: gateway = WS hot path (<50us); playground = REST
  cold path (<100ms acceptable)
- Matches what trade UI (rsx-webui) actually uses today

Update `specs/2/26-rest.md` to document this architecture.

## Non-goals

- Moving REST to rsx-gateway (deferred to v2)
- GraphQL or alternate protocols
- Webhooks (use WS)
- Admin endpoints (stay in playground's /api/ namespace)

## IO Surfaces

- `rsx-playground/server.py` FastAPI on :49171
- `/v1/account`, `/v1/positions`, `/v1/orders`, `/v1/fills`,
  `/v1/funding`, `/v1/symbols`, `/v1/candles`
- `Authorization: Bearer <JWT>` header (same secret as gateway WS)
- Postgres via asyncpg (already connected)
- WAL files via `parse_wal_*` helpers (fallback when PG unavailable)

## Current state (audit)

| Endpoint | Exists | Auth | Rate limit | Tests | Data source |
|----------|--------|------|------------|-------|-------------|
| /v1/symbols | ✅ | no | no | ? | in-memory symbol_configs |
| /v1/candles | ✅ | no | no | ? | WAL fills |
| /v1/funding | ✅ | no | no | ? | WAL funding records |
| /v1/positions | ✅ | no | no | ? | WAL fills (not PG!) |
| /v1/fills | ✅ | no | no | ? | WAL fills |
| /v1/account | ✅ | no | no | ? | ? |
| /v1/orders | ✅ | no | no | ? | ? |

## Tasks

### 1. Schema + data-source consistency pass

Each endpoint should:
- Return JSON envelope `{"data": ..., "pagination": ..., "request_id": ...}`
  OR keep flat array/object but document format
- Pull from Postgres (authoritative) where possible;
  WAL fallback only when PG down
- Include standard fields per spec 26-rest.md

Audit each handler, pick one shape, apply consistently.

### 2. JWT auth middleware

Extract `Authorization: Bearer <JWT>` header, validate
against `RSX_GW_JWT_SECRET` (same secret as WS), set
`user_id` on request state. Reject 401 on missing/invalid.

Decision: **protected endpoints** (account, positions,
orders, fills, funding) require auth. **Public endpoints**
(symbols, candles) do not.

Files: `rsx-playground/server.py` — add auth dependency,
FastAPI `Depends(verify_jwt)`.

### 3. Rate limits

Per-user: 60 req/min sustained, 10 req/s burst. Rejects
with 429 + `Retry-After` header.

Use an in-memory token bucket keyed by user_id. Cleared
hourly.

### 4. CORS

Permissive in dev; configurable origin allowlist via env
var. Preflight OPTIONS support.

### 5. Error response envelope

All error paths return `{"error": "...", "code": "...",
"request_id": "..."}`. 401/403/404/429/500. Request ID in
response header + body for traceability. Log 5xx with
request_id.

### 6. Docs / OpenAPI

Document endpoints in `specs/2/26-rest.md`. Include
request/response schema per endpoint, auth requirement,
rate limit cost.

Optional: generate OpenAPI JSON from FastAPI route schemas
for machine-readable spec.

### 7. Unit tests

Per endpoint handler:
- Happy path (returns expected shape)
- Missing auth → 401
- Invalid JWT → 401
- Non-existent user → 404
- Filter params work correctly

Location: `rsx-playground/tests/rest_unit_test.py`

### 8. Integration tests

Real Postgres via testcontainers-python. Seed data, get
JWT token, HTTP call, assert response. At least 2 tests
per endpoint.

Location: `rsx-playground/tests/rest_integration_test.py`

### 9. Playwright e2e

Exercise REST from trade UI. Positions panel, Orders
history, Funding tab should all hit REST endpoints and
render. Covered via existing `play_trade.spec.ts`.

### 10. Update 26-rest.md

Document finalized architecture. Change status from
`partial` to `shipped`. Remove "Deferred" section for
implemented endpoints.

## Acceptance

- All 5 protected endpoints: 401 without auth, 200 with
  valid JWT
- 429 under load (rate limit test)
- Trade UI positions/orders/funding fully working (no
  fallback to playground proxy — direct REST)
- Unit test suite: 35+ test cases
- Integration tests with testcontainers: 10+ cases pass
- Playwright tests pass
- `specs/2/26-rest.md` status = `shipped`

## Out of scope / follow-up

- Moving REST to rsx-gateway Rust (v2)
- GraphQL layer
- WS account-state push (v2)
- Admin endpoints (stay at `/api/`)
