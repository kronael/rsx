# PROJECT.md — Gateway REST Endpoints (Full)

## Goal

Production REST API on rsx-gateway. 5 endpoints
(account, positions, orders, fills, funding) with JWT
Bearer auth, rate limits, CORS, proper error envelope.
Pure monoio — no Postgres in the gateway. Data comes from
rsx-risk via CMP query messages (risk has all state in
memory).

## Non-goals

- Postgres client in gateway (risk owns the DB)
- Admin / internal endpoints (those live in playground)
- GraphQL / WebSocket push for account state (use WS)
- Hosting in playground (playground is dev-only)

## Architecture

```
  trade UI                                rsx-risk
    |                                    (in-memory)
    |-- GET /v1/positions ---+              ^
    |   Bearer <JWT>         |              |
    v                        v              |
  rsx-gateway (monoio) ---- CMP/UDP query --+
    :8081 HTTP               QUERY_POSITIONS
                                            |
                                            v
                             (reply) CMP: POSITIONS_RESPONSE
    |                        ^
    |<-- JSON response ------|
    |
```

Gateway REST is a thin adapter:
1. parse HTTP request
2. validate JWT → user_id
3. send CMP query to risk shard (routed by user_id hash)
4. await CMP response (pending map keyed by request_id)
5. serialize to JSON → return HTTP

risk keeps everything in memory; no DB round-trip on
reads. Postgres is the recovery store, not the read path.

## IO Surfaces

- **rsx-gateway REST server** — port `RSX_GW_HTTP_LISTEN`
  (new; e.g. `:8081`)
- **CMP query/response** — new record types for each endpoint
- **JWT Bearer** — Authorization header; same secret as WS

## New CMP record types (specs/2/18-messages.md)

Request messages sent gateway → risk:
- `RECORD_QUERY_ACCOUNT { req_id, user_id }` (0x40)
- `RECORD_QUERY_POSITIONS { req_id, user_id, symbol_filter }` (0x41)
- `RECORD_QUERY_ORDERS { req_id, user_id, symbol_filter, status_filter }` (0x42)
- `RECORD_QUERY_FILLS { req_id, user_id, symbol_filter, from_ts, to_ts, limit }` (0x43)
- `RECORD_QUERY_FUNDING { req_id, user_id, symbol_filter, limit }` (0x44)

Response messages sent risk → gateway:
- `RECORD_ACCOUNT_RESPONSE { req_id, collateral, frozen_margin, equity, unrealized_pnl, ... }` (0x50)
- `RECORD_POSITIONS_RESPONSE { req_id, count, entries: [...] }` (0x51)
- `RECORD_ORDERS_RESPONSE { req_id, count, entries: [...] }` (0x52)
- `RECORD_FILLS_RESPONSE { req_id, count, entries: [...] }` (0x53)
- `RECORD_FUNDING_RESPONSE { req_id, count, entries: [...] }` (0x54)

Response records may exceed CMP fixed-size; use `entries`
array up to a cap, paginate via subsequent request if
needed, OR stream via multi-record response.

## Tasks

### 1. CMP query/response record design
Write `specs/2/18-messages.md` section documenting 5 new
request types + 5 response types. Fixed-size layout for
headers + variable-size entry arrays (or paginated).

Files: `specs/2/18-messages.md`, `rsx-dxs/src/records.rs`
(add 10 new Record structs), `rsx-gateway/src/protocol.rs`
(if gateway encodes these too).

### 2. rsx-risk query handler
In `rsx-risk/src/shard.rs`, add handlers for the 5 query
types. Each handler reads in-memory state, builds
response CMP record(s), sends back to gateway.

Files: `rsx-risk/src/shard.rs` (+ new `rsx-risk/src/query.rs` if
the logic gets bulky).

### 3. Gateway pending query map
Similar to pending order map. Keyed by req_id. Stores
the HTTP response channel (or future) so when the CMP
response arrives, gateway finishes the HTTP write.

Files: `rsx-gateway/src/pending_query.rs` (new),
`rsx-gateway/src/main.rs` (wire CMP response handler).

### 4. JWT middleware
Extract Bearer token, validate HS256 with shared secret.
Reject 401. Same JWT as WS so no new issuance logic.

Files: `rsx-gateway/src/rest.rs` + reuse `rsx-gateway/src/jwt.rs`.

### 5. REST endpoint handlers
For each of 5 endpoints: parse path, parse query params,
call shared `send_cmp_query()` helper, await response
(with timeout), serialize response entries to JSON.

Files: `rsx-gateway/src/rest.rs`.

### 6. Rate limits per user
Reuse pattern from `rsx-gateway/src/rate_limit.rs`. Per-user
token bucket. 100 req/min sustained, 10 req/s burst. Return
429 + Retry-After header.

Files: `rsx-gateway/src/rate_limit.rs` (extend for HTTP),
`rsx-gateway/src/rest.rs` (apply on protected routes).

### 7. CORS
Permissive in dev (`Access-Control-Allow-Origin: *`).
Configurable allowlist via env var for prod. Preflight
OPTIONS returns 204.

Files: `rsx-gateway/src/rest.rs`.

### 8. Error response envelope
Consistent shape: `{"error": "...", "code": "...",
"request_id": "...}`. All error paths return JSON with
envelope + set X-Request-Id header. Request ID via
`uuid::Uuid::new_v4()`.

Files: `rsx-gateway/src/rest.rs`.

### 9. Unit tests (Rust)
Per handler: happy path (mock risk response), 401
(missing/invalid JWT), 429 (rate limit), 404 (unknown
user). Mock CMP transport.

Files: `rsx-gateway/tests/rest_handlers_test.rs`.

### 10. Integration tests (testcontainers-rs)
Real Postgres, real risk, mock gateway TCP. Seed user,
issue JWT, HTTP call, assert response. ≥2 per endpoint.

Files: `rsx-gateway/tests/rest_integration_test.rs`.

### 11. Playwright e2e
Trade UI: Positions tab, Orders history, Funding tab hit
gateway REST (not playground). Assert real data flows.

Files: `rsx-playground/tests/play_trade.spec.ts` (update
to point at gateway REST endpoint or nginx-proxied path).

### 12. Remove /v1/ squatting from playground
playground has /v1/positions etc. — move under /api/ or
delete; playground's catch-all /v1/{path:path} proxy
stays to forward to gateway.

Files: `rsx-playground/server.py`.

### 13. Spec update
`specs/2/26-rest.md`: status partial → shipped. Document
the 5 endpoints, auth, rate limits, error envelope. Add
the CMP query architecture note.

## Acceptance

- All 5 endpoints return valid JSON for seeded users
- 401 on missing/invalid JWT
- 429 under load test
- CMP query latency <5ms (gateway→risk→gateway in same host)
- Integration tests pass: ≥10 cases
- Playwright e2e: trade UI renders positions/orders/funding
  from gateway REST
- `specs/2/26-rest.md` status = shipped

## Dependencies

- 11-OAUTH must be at least partially shipped (JWT issuance)
  before end-to-end testing — OR use dev-issued JWTs during
  08 development.

## Out of scope / follow-up

- WS push for account state changes (v2)
- GraphQL
- Admin endpoints (stay in playground)
- Pagination beyond basic limit/offset (v2: cursor-based)
