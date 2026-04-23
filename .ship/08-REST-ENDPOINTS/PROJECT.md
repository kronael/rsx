# PROJECT.md — Gateway REST Endpoints (Full)

## Goal

Full implementation of the gateway REST API per
`specs/2/26-rest.md`. All 5 endpoints working end-to-end:
account, positions, orders, fills, funding. Plus JWT auth,
rate limits, CORS, proper error responses. Unit tests +
integration tests. Must be productive quality (not prototype).

## Non-goals

- GraphQL or alternate protocols
- Webhooks / server-push (use WS instead)
- Admin / internal endpoints (those live in playground)

## IO Surfaces

- **rsx-gateway REST server** — port per `RSX_GW_HTTP_LISTEN`
  env var (new — currently REST runs alongside WS on same
  process; decide on port strategy)
- **Postgres** read path — positions, orders, fills, account,
  funding history via rsx-risk persistence layer
- **JWT Bearer** in `Authorization` header (same secret as WS)

## Tasks

### 1. Finalize endpoint schemas

Review `specs/2/26-rest.md` §API section. For each of the
5 endpoints, lock down:
- URL path (`/v1/<endpoint>`)
- Query params (symbol filter, time range, pagination)
- Response schema (fields, types, pagination envelope)
- Error codes (401, 403, 404, 429, 500)

Update spec to match final decisions.

### 2. Wire JWT auth on REST

Existing WS JWT validation in `rsx-gateway/src/jwt.rs`.
Add middleware that extracts Bearer token from
`Authorization` header, validates HS256, sets user_id on
request context. Reject 401 if invalid/expired.

### 3. Implement GET /v1/account

Query rsx-risk Postgres for user account state
(collateral, frozen_margin, free_collateral, version).
Cache per-user with short TTL (1s) to avoid hammering PG.

### 4. Implement GET /v1/positions

Query positions table filtered by user_id. Support
`?symbol_id=X` filter. Include unrealized PnL computed
from current mark.

### 5. Implement GET /v1/orders

Query open (not fully filled/cancelled) orders for user.
Support `?status=open|closed|all` and `?symbol_id=X`.
Paginate by order_id.

### 6. Implement GET /v1/fills

Query fill history for user. Support `?from_ts=X&to_ts=Y`
time range, `?symbol_id=X` filter. Paginate by ts.

### 7. Implement GET /v1/funding

Query funding history (settlement events). Same filters
as fills. Include rate, quantity, pnl_delta per event.

### 8. Rate limits

Per-user: 100 req/s burst, 1000 req/min sustained.
Return 429 with Retry-After header when exceeded. Reuse
`rsx-gateway/src/rate_limit.rs` patterns.

### 9. CORS

Permissive in dev (allow all origins), configurable via
env var in prod. Preflight support.

### 10. Error responses

Consistent JSON shape: `{"error": "...", "code": "...",
"request_id": "..."}`. Log 5xx with request_id for
traceability.

### 11. Unit tests

Per endpoint: handler logic with mocked PG. Cover happy
path, 401, 403, 404, pagination, filtering.

### 12. Integration tests (testcontainers)

Real Postgres via testcontainers-rs. Full flow: seed data,
JWT token, HTTP call, assert response. At least 2 tests
per endpoint.

### 13. Playwright e2e

Exercise REST from the trade UI. Ensure Positions panel,
Orders history, Funding tab all hit the REST endpoints
and render correctly.

### 14. Documentation

Update `specs/2/26-rest.md` from `partial` to `shipped`.
Regenerate OpenAPI-style schema doc or keep as markdown
(simpler — keep markdown).

## Acceptance

- All 5 endpoints return valid responses for seeded test
  users
- JWT auth rejects unauthenticated (401) and unauthorized
  (403) requests
- Rate limits trigger 429 under load test
- Integration test suite passes: at least 10 test cases
  across all endpoints
- `specs/2/26-rest.md` status = `shipped`, content matches
  code
- Trade UI positions/orders/funding tabs work against
  gateway REST (not playground proxy fallback)

## Out of scope / follow-up

- GraphQL layer if needed later
- Websocket-based subscription to account state changes
- Admin endpoints (stay in playground)
