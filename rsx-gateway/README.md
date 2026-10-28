# rsx-gateway

The client edge of the exchange: one process that takes WebSocket order entry
from clients and pushes order updates and fills back. The design is in
[ARCHITECTURE.md](ARCHITECTURE.md).

## What It Does

Accepts WebSocket connections with a JWT, rate-limits and validates orders,
forwards them to Risk over casting/UDP and routes Risk's responses back to the
owning user's connections. The same port serves `GET /health` and
`GET /v1/symbols`.

## Running

```
RSX_GW_LISTEN=0.0.0.0:8080 \
RSX_GW_CAST_ADDR=127.0.0.1:9102 \
RSX_RISK_CAST_ADDR=127.0.0.1:9101 \
RSX_GW_WAL_DIR=./tmp/wal \
RSX_GW_JWT_SECRET=a-secret-of-at-least-32-bytes-long \
RSX_GW_DATABASE_URL=postgres://rsx:rsx@127.0.0.1:5432/rsx \
cargo run -p rsx-gateway
```

## Environment Variables

Every name below is read by `src/config.rs`, `src/main.rs`,
`src/cast_recv.rs` or `src/state.rs`. A numeric value that does not parse
falls back to its default. An address that does not parse in
`RSX_GW_CAST_ADDR` or `RSX_RISK_CAST_ADDR` stops startup.

| Env Var | Default | Purpose |
|---------|---------|---------|
| `RSX_GW_JWT_SECRET` | none | HS256 secret shared with `rsx-auth`; startup exits 2 when unset or shorter than 32 bytes |
| `RSX_GW_LISTEN` | `0.0.0.0:8080` | WebSocket and REST listen address (plain TCP, no TLS) |
| `RSX_GW_CAST_ADDR` | `127.0.0.1:9102` | casting bind address for Risk's responses |
| `RSX_RISK_CAST_ADDR` | `127.0.0.1:9101` | Risk's casting address, where orders are sent |
| `RSX_GW_WAL_DIR` | `./tmp/wal` | WAL directory of the casting sender and the replay tip file |
| `RSX_GW_HEALTH_ADDR` | unset | address of `/health`, `/ready`, `/metrics`; no server when unset |
| `RSX_GW_MAX_PENDING` | `10000` | in-flight (`Submitted`) orders across all users; a full set answers `E[5]`. Resting orders are tracked for cancel and do not count |
| `RSX_GW_MAX_OUTBOUND` | `1024` | queued frames per connection; overflow closes the connection |
| `RSX_GW_ORDER_TIMEOUT_MS` | `10000` | age at which an order with no first response is answered TIMEOUT |
| `RSX_GW_HEARTBEAT_INTERVAL_S` | `5` | seconds between server `H` frames |
| `RSX_GW_IDLE_TIMEOUT_S` | `10` | seconds without an inbound frame before the connection is closed |
| `RSX_GW_RL_USER` | `10` | orders per second per user |
| `RSX_GW_RL_IP` | `100` | orders per second per source address |
| `RSX_GW_RL_INSTANCE` | `1000` | orders per second across the instance |
| `RSX_GW_CB_THRESHOLD` | `10` | consecutive failed forwards that open the circuit |
| `RSX_GW_CB_COOLDOWN_MS` | `30000` | milliseconds an open circuit stays open before one probe |
| `RSX_GW_CORE_ID` | unset | pin the monoio reactor thread |
| `RSX_GW_CAST_CORE_ID` | unset | pin the `gw-cast-recv` thread; a warning at startup when unset |
| `RSX_GW_DATABASE_URL` | required (exit 2) | Postgres of risk's config; the symbol table is read from `symbol_config_schedule` |
| `RSX_GW_SYMBOL_POLL_S` | `10` | seconds between re-reads of the symbol table, besides the re-read on a newer `CONFIG_APPLIED` |
| `RSX_RISK_REPLICATION_ADDR` | unset | `host:port` of risk's replication service (`RSX_RISK_REPLICATION_BIND_ADDR`) for a gap in the response stream, read once at boot (a malformed value exits 2); the gap is skipped when unset or when the replay TLS files that `rsx-cast` names are missing |

The gateway serves exactly the symbols risk's Postgres lists: per symbol the
row with the highest `config_version` whose `effective_at_ms` has passed. Boot
stops when Postgres is unreachable, the table is missing or empty after 30 s
(risk creates and fills it), or a row is malformed (non-positive tick, lot or
`min_notional`, decimals outside 0..=255, a status that is not a lowercase
word). A later failed re-read keeps the table in force and is logged.

`RUST_LOG` sets the log filter. The core ids are indexes into the cpus the
process may run on, not cpu numbers.

## Error Codes

A refused order answers `E[code,"message"]` and the connection stays open.
Each code names one condition class and no code serves two.

| Code | Message | Condition |
|------|---------|-----------|
| 5 | `overloaded` | instance rate limit, in-flight set full, or circuit open |
| 8 | `network error` | the casting send of an order or cancel to Risk failed |
| 1001 | `invalid utf8` | the frame payload is not UTF-8 |
| 1002 | the parse error | the frame does not parse, including a `cid` outside 1 to 20 bytes |
| 1004 | `unsupported` | a frame kind a client may not send (`U`, `F`, `E`, `Q`) |
| 1005 | `invalid order id`, `order not found` | a cancel key that names no pending order of the sender |
| 1006 | `rate limited` | the per-user or per-IP limit, and nothing else |
| 1007 | `unknown symbol` | the symbol id is not configured |
| 1008 | `price not tick aligned` | price is not a positive multiple of the tick |
| 1009 | `qty not lot aligned` | quantity is not a positive multiple of the lot |

Outcomes from Risk and the matching engine arrive as `U` and `F` frames, not
as `E`. A repeated `cid` and an order timeout are `U` status 3 with reason
3 (`DUPLICATE_ORDER_ID`) and reason 10 (`TIMEOUT`). The frame formats are in
`specs/2/49-webproto.md`.

## Deployment

- No durable state. The `cid` map and the pending set live in memory, so a
  restart empties both.
- One gateway per Risk: Risk sends every response to one `RSX_GW_CAST_ADDR`,
  so a second gateway receives none.
- `RSX_GW_JWT_SECRET` is shared with the auth service.
- TLS terminates in a proxy in front of the gateway.
- monoio needs Linux with io_uring, kernel 5.1 or newer.

## Auth

- HS256 JWT in the `Authorization: Bearer` header of the upgrade request. A
  missing or invalid token gets HTTP 401 and no handshake.
- Validates `exp`, `nbf` when present, `aud == "rsx-gateway"` and
  `iss == "rsx-auth"`.
- A token is reusable until `exp`: no `jti` is required or tracked, and one
  token opens any number of connections, up to 5 per user.
- The per-IP limiter map holds at most `IP_LIMITER_MAX = 10_000` addresses
  and evicts the oldest first.

## Testing

```
cargo test -p rsx-gateway
```

Unit tests sit in `src/*_test.rs`. `tests/` runs the connection handler over
real sockets (`handler_*_test.rs`) and the real binary
(`gateway_process_test.rs`: pinning and `/metrics`). A `known_defect_*` test
pins an open entry in `BUGS.md`. `benches/` holds the Criterion benches and
`ws_order_latency.rs`, which needs a live cluster.

## Dependencies

- `rsx-types` for `SymbolConfig`, `FailureReason` and CPU pinning
- `rsx-cast` for the casting sender and receiver
- `rsx-messages` for the wire records and the client JSON frames (`WsFrame`, `parse`, `serialize`)
- `rsx-health` for `/health`, `/ready` and `/metrics`
- `rsx-log` for off-hot-path logging

## Gotchas

- The 6th connection of a user completes the upgrade and is then closed with
  no error frame.
- A resend with a known `cid` inside one hour is answered, not forwarded. It
  does not resubmit an order that never reached Risk.
- A crash loses the in-flight orders: the pending set is memory-only. The
  gateway has no order-status query.
- Rate limits are per instance.
- An open circuit rejects every new order with `E[5]`. Orders are not queued.
- The circuit counts failed casting sends, not slow or silent responses from
  Risk.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md): runtime model, module table, flows,
  limits
- [notes/io-model.md](notes/io-model.md): why monoio here
- `specs/2/11-gateway.md`, `specs/2/49-webproto.md`
