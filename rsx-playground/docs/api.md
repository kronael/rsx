# API Reference

REST and HTMX partial endpoints exposed by the RSX playground server.

## REST API

### Orders

| Method | Path | Description |
|--------|------|-------------|
| POST | /api/order | Submit a new order |
| DELETE | /api/order/{oid} | Cancel an order |
| GET | /api/orders | List recent orders |

### Trade (authenticated, per-user)

| Method | Path | Description |
|--------|------|-------------|
| POST | /api/trade/orders | Submit an order as the caller's own session user (no `user_id` field) |
| POST | /api/trade/orders/{cid}/cancel | Cancel the caller's own order by client id |

### Auth / roles (proxied to rsx-auth)

| Method | Path | Description |
|--------|------|-------------|
| GET | /oauth/github/login | Start GitHub OAuth |
| GET | /oauth/github/callback | OAuth callback, issues session cookie |
| GET | /auth/me | Caller's own identity + role |
| POST | /auth/logout | Clear session cookie |
| GET | /roles/me | Caller's own role (self-service, no gate) |
| GET | /roles/users | List users + roles (`role=admin` or other `read_user_state` roles) |
| POST | /roles/users/{uid}/role | Change a user's role (`role=admin`; requires `Idempotency-Key` header + `reason`/`ticket_id` body) |

### Users (admin, `role=admin`)

| Method | Path | Description |
|--------|------|-------------|
| GET | /users | Admin user/role management page (403 without `role=admin`) |
| POST | /api/users/create | Create a user (loopback/`PLAYGROUND_ADMIN_TOKEN` ops harness) |
| POST | /api/users/{uid}/deposit | Deposit collateral (loopback/`PLAYGROUND_ADMIN_TOKEN` ops harness) |

### Book

| Method | Path | Description |
|--------|------|-------------|
| GET | /api/book/{depth} | Orderbook snapshot |
| GET | /api/bbo | Best bid/offer |

### Risk

| Method | Path | Description |
|--------|------|-------------|
| GET | /api/risk/users/{uid} | User risk state |
| POST | /api/risk/users/{uid}/freeze | Freeze user |
| POST | /api/risk/users/{uid}/unfreeze | Unfreeze user |
| POST | /api/risk/liquidate | Trigger liquidation |

### Status

| Method | Path | Description |
|--------|------|-------------|
| GET | /api/status | System health |
| GET | /api/maker/status | Market maker health |

## HTMX Partials

All `/x/*` endpoints return HTML fragments for HTMX polling.

| Path | Description |
|------|-------------|
| /x/position-heatmap | Net position per symbol |
| /x/margin-ladder | Recent fills and notional |
| /x/funding | BBO-derived funding rate |
| /x/risk-latency | Order latency percentiles |
| /x/book-stats | Live BBO table |
| /x/live-fills | Recent trade fills |
| /x/processes | Process health grid |
| /x/health | Health summary |
| /x/wal-status | WAL writer status |
| /x/logs | Recent log lines |
