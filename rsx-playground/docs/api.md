# API Reference

REST and HTMX partial endpoints exposed by the RSX playground server.

## REST API

### Orders

| Method | Path | Description |
|--------|------|-------------|
| POST | /api/order | Submit a new order |
| DELETE | /api/order/{oid} | Cancel an order |
| GET | /api/orders | List recent orders |
| POST | /api/orders/test | Submit an order as `user_id` (admin harness) through `rsx-cli order place`. Form fields include `tif` (`GTC`, `IOC`, `FOK`), `post_only`, `reduce_only`; price `0` is a market order, a marketable-limit IOC sweep priced from the book. The answer is `order <cid> <status>`: `filled`, `partial`, `resting`, `cancelled`, `rejected: <reason>` or `timeout` (504, the order may still rest) |
| POST | /api/orders/quick | One-click order as `user_id` (admin harness): `side`, `qty`, `price_offset_pct` (0 = market sweep), `symbol_id`, `randomize`, `rand_side` |
| POST | /api/orders/batch | `count` (default 10, at most 100) passive minimum-size orders around the PENGU mid, alternating sides, through `rsx-cli order batch` (admin harness) |
| POST | /api/orders/random | Five minimum-size orders near the mid of running symbols that have a book (admin harness) |
| POST | /api/orders/invalid | One lot-misaligned order the gateway rejects (admin harness) |
| POST | /api/orders/{cid}/cancel | Cancel a resting order through `rsx-cli order cancel` as the user it was placed by (admin harness); the order turns `cancelled` only when the gateway answers with a cancelled frame |
| POST | /api/gateway/duplicate-demo | Send one IOC order twice with one client order id as `user_id` (admin harness); the repeat is answered `DUPLICATE_ORDER_ID` |

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
| GET | /users | Admin user/role management page, with each user's collateral from `rsx-cli risk overview` (403 without `role=admin`) |

### Config (admin, `role=admin`)

| Method | Path | Description |
|--------|------|-------------|
| GET | /config | The row risk runs on for each symbol (`rsx-cli config show`: the highest version already effective), with the minimum order in USD (403 without `role=admin`) |
| POST | /api/config/schedule | Insert a new config version for a symbol with `rsx-cli config schedule` (403 without `role=admin`). Form fields: `symbol_id`, `initial_margin_rate_bps`, `maintenance_margin_rate_bps`, `max_leverage`, `effective_in_sec`, and optional `min_notional_usd`: the minimum order in USD, converted to the raw `price x qty` integer with the symbol's decimals (`round(usd x 10^(price_decimals + qty_decimals))`); blank keeps the current value. 400 with the reason for a non-numeric, zero or negative amount, one that rounds to zero raw, or one above the BIGINT range; 400 also carries the CLI's refusal (maintenance margin not below initial, leverage below 1); 404 for an unknown symbol; 502 with the CLI's error when it cannot reach Postgres |

### Symbols

| Method | Path | Description |
|--------|------|-------------|
| GET | /v1/symbols | Symbol catalog (`id`, `symbol`, `tick_size`, `lot_size`, `price_decimals`, `qty_decimals`) plus `M` tuples. From `rsx-cli config show`; a symbol is listed with its row in force (highest version with `effective_at_ms` at or before now, as risk applies it), which also carries `min_notional` (raw integer) and `min_notional_usd` (exact decimal string); cached 1s. 502 with the CLI's error when it cannot read Postgres |

### Book

| Method | Path | Description |
|--------|------|-------------|
| GET | /api/book/{depth} | Orderbook snapshot |
| GET | /api/bbo | Best bid/offer |

### Risk

| Method | Path | Description |
|--------|------|-------------|
| GET | /api/risk/users/{uid} | `rsx-cli account show`: `user_id`, `collateral`, `positions` (`symbol_id`, `net_qty`, `avg_px`, `upnl` at the BBO mid) and `frozen` orders; 404 for a user without an account (admin gate) |
| GET | /api/risk/overview | Every account from `rsx-cli risk overview`, margined at the active config's rates and marked at the BBO mid, with the system totals |
| GET | /api/risk/liquidations | Risk's `liquidations` table, else the WAL's liquidation records |
| GET | /api/risk/insurance | Insurance fund balances |

### Gateway

| Method | Path | Description |
|--------|------|-------------|
| GET | /api/gateway/metrics | Per-gateway state read from each gateway's `/metrics`, cached 1s: `live`, `ready`, `state`, `pending` (`used`, `cap`), `circuit` (`closed`, `open`, `half-open`), `rejects` (per spec 18 `FailureReason`: `code`, `name`, `count`) and the raw `counters` (`connections`, `orders_processed`, `rejects_*`, `rejects_total`, `circuit_state`). An unreachable gateway carries its reason in `error`; 503 when none is reachable |
| POST | /api/gateway/duplicate-demo | Send one IOC buy twice with the same client order id over the real gateway as `user_id` (admin; form fields `user_id`, `symbol_id`, `price`, `qty`). 200 with both frames when the second is answered `DUPLICATE_ORDER_ID`; 502 when the gateway does not answer or answers anything else, 503 when the gateway or the symbol's engine is down |

### Matching engine

| Method | Path | Description |
|--------|------|-------------|
| GET | /api/matching/counters | Per-ME counters read from each ME's `/metrics` (`orders_processed`, `publishes`, `drops`, `dedup_entries`, `rejects_*`, `wal_last_seq`, `wal_flushed_seq`, `checkpoint_wal_seq`, `config_version`), cached 1s. An unreachable ME carries its reason in `error`; 503 when none is reachable |
| GET | /api/matching/wal | Decoded ME WAL records, newest first. Query: `type` (`order_failed`, `config_applied`, `order_cancelled`, ...; 400 if unknown), `symbol_id`, `stream` (404 if unknown), `limit` (1-1000, default 100). `order_cancelled` carries `reason` (spec 18 wire code) and `reason_text`; `order_failed` carries `reason` (spec 18 `FailureReason` wire code) and `reason_text` |
| GET | /api/matching/config | Per symbol: the `symbol_config_applied` row and the `symbol_config_schedule` rows, each tagged `applied`, `superseded`, `due` or `pending`. 503 without Postgres, 500 with the query error |
| POST | /api/matching/crash/{name} | `rsx-cli fault kill` one ME (admin), wait for the watcher to bring it back ready, and return its WAL tip, flushed seq, checkpoint seq and config version from before the crash and after the recovery. 404 for an unknown ME; a failure to come back within 30s is reported in the body |
| POST | /api/matching/duplicate-demo | Cast the same order twice to a scratch ME (admin) through `rsx-cli fault dup-order`: the real `rsx-matching` binary on its own ports, symbol and WAL under `tmp/me-demo`. Returns that ME's records for the order id, ending in ORDER_FAILED `duplicate order id`. The gateway mints a fresh order id per order, so the live engine never sees a repeat |

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
| /x/gateway-counters | Per-gateway state, connections, orders forwarded, rejects, pending and circuit from each gateway's `/metrics` |
| /x/gateway-rejects | Rejects by spec 18 reason, summed over the reachable gateways |
| /x/matching-counters | Per-ME counters from each ME's `/metrics` |
| /x/matching-recovery | Per-ME WAL tip, flushed seq, checkpoint seq, records to replay, config version |
| /x/matching-rejects | Latest ORDER_FAILED records and engine-made cancels (post-only, reduce-only, expiry) from the ME WAL |
| /x/live-fills | Recent trade fills |
| /x/processes | Process health grid |
| /x/health | Health summary |
| /x/wal-status | WAL writer status |
| /x/logs | Recent log lines |
