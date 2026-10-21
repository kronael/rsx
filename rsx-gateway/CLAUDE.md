# CLAUDE.md — rsx-gateway

Local to `rsx-gateway/`; the root `../CLAUDE.md` applies. The gateway is the
production client edge: WebSocket JSON order entry, JWT auth, rate limits,
client-id ↔ order-id mapping, casting to risk and the fill/update push back.
The playground only demonstrates it. Specs: `specs/2/11-gateway.md`,
`20-network.md`, `49-webproto.md`, `29-rpc.md`, `18-messages.md`,
`26-rest.md`, `7-dashboard.md` (support reads), `59-latency-observability.md`;
tests `37-testing-gateway.md`.

## Rules

- Owns JWT (HS256, `aud`/`iss`/`exp`/`nbf`), frame parsing and structural
  validation, per-user / per-IP / per-instance rate limits, the circuit
  breaker, cid and oid. Does NOT own margin, symbol status or position
  permission; risk and matching do. NEVER duplicate their checks here.
- TLS terminates in a proxy in front of the gateway; the gateway listens on
  plain WS. A JWT is reusable until `exp`; NEVER a single-use `jti`.
- No resting-order cap. The DoS answer is the minimum order size in the
  symbol config, enforced by risk.
- Order forwarding is at-most-once over casting plus idempotent retry: the
  gateway sends an order once, and the pending sweep re-sends nothing: it
  answers the user `U` status 3 reason TIMEOUT for an order with no first
  response (a resting order is kept). Dedup on cid inside the 1 hour window
  maps a retry to the same order, answered `U` status 3 reason
  DUPLICATE_ORDER_ID; the map is memory-only, so a restart empties it. The
  target design is `specs/2/61-cid-idempotency.md` (draft). The manual-seq
  send trio (`send_raw`, `next_seq`, `advance_seq`) stays caller-owned.
- Runtime: a monoio (io_uring) reactor for the many client fds plus one
  dedicated `gw-cast-recv` thread, pinned on `RSX_GW_CAST_CORE_ID`, that
  drains risk's casting stream into an SPSC ring and rings a doorbell. The
  reactor parks on readiness; it NEVER yield-spins or polls. Make the reactor
  correct before splitting it into tiles.
- One housekeeping path owns the periodic heartbeat and the idle reap; the
  handler queues only the greeting. A reap closes the socket. Per-connection
  outbound is bounded by `RSX_GW_MAX_OUTBOUND`; overflow closes the
  connection.
- One error code per condition class: `E[1006]` is rate limit only; pending
  full, open circuit and the instance limit (`RSX_GW_RL_INSTANCE`) answer
  `E[5]`; a failed forward answers `E[8]`. NEVER reuse a code for another
  condition.
- Per-hop timestamps ride inside the normal records (`gw_in_ns` and the other
  stamps on the fill), clock-synced in production; a latency leg is derived
  from one frame's stamps by any consumer. NEVER a separate delta frame.
- Account, position and fill reads are gateway REST (`/v1/api/support/*`,
  spec 7) over the Postgres that risk writes; `rsx-auth` is identity and
  roles only.
- The gateway and `rsx-marketdata` are one shape mirrored (fan-in here,
  fan-out there); a structural change is made in both or recorded in
  `BUGS.md` as the alignment gap.
- Tests: the connection handler runs over real sockets in `tests/`; a
  `known_defect_*` test pins an open `BUGS.md` entry;
  `tests/gateway_process_test.rs` checks the real binary (pinning,
  `/metrics`). `benches/ws_order_latency.rs` is the round-trip number.

## Docs (which file answers which question)

| File | Question |
|---|---|
| `README.md` | what it does, `Running` (env), quirks |
| `ARCHITECTURE.md` | how it is built: reactor + cast-recv thread, handler, routing, pending, limits, JWT, REST |
| `notes/io-model.md` | why monoio/io_uring here while casting busy-spins std UDP, and what zero-copy would take |
| `compare/README.md` | vs named alternatives |

## Keeper sections — do not regress

- The io-model note: readiness vs completion is most of the latency story
  for a many-connection edge.
- The at-most-once + idempotent-retry statement; collapsing it into
  "reliable delivery" re-opens a settled design.
- The one-code-per-condition table in `README.md`.
