# API Reference

REST and HTMX partial endpoints exposed by the RSX playground server.

## REST API

### Orders

| Method | Path | Description |
|--------|------|-------------|
| POST | /api/order | Submit a new order |
| DELETE | /api/order/{oid} | Cancel an order |
| GET | /api/orders | List recent orders |
| POST | /api/orders/test | Submit an order as `user_id` (admin harness). Form fields include `tif` (`GTC`, `IOC`, `FOK`), `post_only`, `reduce_only`. The client order id is 20 chars, the gateway's cancel key |
| POST | /api/orders/{cid}/cancel | Cancel a resting order through the gateway as the user it was placed by (admin harness); the order turns `cancelled` only when the gateway answers with a cancelled frame. Bookkeeping-only `submitted` entries (batch, random) flip locally |

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

### Matching engine

| Method | Path | Description |
|--------|------|-------------|
| GET | /api/matching/counters | Per-ME counters read from each ME's `/metrics` (`orders_processed`, `publishes`, `drops`, `dedup_entries`, `rejects_*`, `wal_last_seq`, `wal_flushed_seq`, `checkpoint_wal_seq`, `config_version`), cached 1s. An unreachable ME carries its reason in `error`; 503 when none is reachable |
| GET | /api/matching/wal | Decoded ME WAL records, newest first. Query: `type` (`order_failed`, `config_applied`, `order_cancelled`, ...; 400 if unknown), `symbol_id`, `stream` (404 if unknown), `limit` (1-1000, default 100). `order_cancelled` carries `reason` (spec 18 wire code) and `reason_text`; `order_failed` carries `reason` (spec 18 `FailureReason` wire code) and `reason_text` |
| GET | /api/matching/config | Per symbol: the `symbol_config_applied` row and the `symbol_config_schedule` rows, each tagged `applied`, `superseded`, `due` or `pending`. 503 without Postgres, 500 with the query error |
| POST | /api/matching/crash/{name} | SIGKILL one ME (admin), wait for the watcher to bring it back ready, and return its WAL tip, flushed seq, checkpoint seq and config version from before the crash and after the recovery. 404 for an unknown ME; a failure to come back within 30s is reported in the body |
| POST | /api/matching/duplicate-demo | Cast the same order twice to a scratch ME (admin): the real `rsx-matching` binary on its own ports, symbol and WAL under `tmp/me-demo`. Returns that ME's records for the order id, ending in ORDER_FAILED `duplicate order id`. The gateway mints a fresh order id per order, so the live engine never sees a repeat |

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
| /x/matching-counters | Per-ME counters from each ME's `/metrics` |
| /x/matching-recovery | Per-ME WAL tip, flushed seq, checkpoint seq, records to replay, config version |
| /x/matching-rejects | Latest ORDER_FAILED records and engine-made cancels (post-only, reduce-only, expiry) from the ME WAL |
| /x/live-fills | Recent trade fills |
| /x/processes | Process health grid |
| /x/health | Health summary |
| /x/wal-status | WAL writer status |
| /x/logs | Recent log lines |
