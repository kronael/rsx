# 53.3 — rsx-gateway completion plan

Read-only audit of the crate at `3b798420`. Every status below was checked
against the code, not BUGS.md or the crate docs. Line numbers are per file
at that commit. Layout: `main.rs` (env, health, monoio reactor loop) →
`handler.rs` (one task per WS connection) → `route.rs` (risk responses to
client frames), with `cast_recv.rs` (dedicated risk→gateway recv thread +
SPSC ring + doorbell), `records.rs` (JSON frames), `ws.rs` (RFC 6455 +
JWT handshake), `state.rs`, `pending.rs`, `rate_limit.rs`, `circuit.rs`,
`rest.rs`, `replay.rs`, `jwt.rs`, `config.rs`, `convert.rs`, `order_id.rs`,
`egress.rs`.

Legend: **T** implemented + tested · **U** implemented, untested ·
**M** missing (or contradicted by the code) · **S** spec stale or
self-contradicting (doc fix, no code) · **N/A** not a gateway concern.

"Tested" counts a test anywhere in the workspace that pins the behaviour.
**Nothing tests the connection handler.** No test calls `handle_connection`
(`handler.rs:35`): the 141 tests in `rsx-gateway/tests` and the 12 in
`src/*_test.rs` drive the structs (`PendingOrders`, `RateLimiter`,
`CircuitBreaker`, `GatewayState`), the parser, the `route_*` functions,
`handle_rest`, and the test-only `ws::ws_handshake`. So every rate-limit,
circuit, validation, pending and cancel decision the handler makes
(`handler.rs:213-458`) and every E code it sends is untested where it is
made. Rows below that say "T (struct)" mean the building block is tested
and the handler wiring is not.

## 1. Spec vs code

### 11-gateway.md (the gateway service spec)

| # | Requirement | St | Evidence |
|---|---|---|---|
| 1 | JWT in the upgrade header; invalid or missing → HTTP 401, no handshake | T | `ws.rs:107-118`; `jwt_ws_e2e_test::{test_ws_handshake_missing_auth, test_ws_handshake_with_expired_jwt}`, `rest_contract_test::ws_handshake_401_without_auth` |
| 2 | HS256; refuse to start on an empty or < 32-byte secret | U | `config.rs:493-507` (`exit(2)`); no test |
| 3 | Claims `exp`, `nbf`, signature | T | `jwt.rs:231-255`; `jwt_test` (expired, wrong secret, nbf in future, bad user id) |
| 4 | `aud == rsx-gateway`, `iss == rsx-auth` | U | `jwt.rs:235-236`; every test token carries the right values, none the wrong ones |
| 5 | `jti` optional and "not wired through the WS handshake in v1" (11:100-105) | S | code requires `jti` and rejects replays (`ws.rs:169-182`); tested `jwt_ws_e2e_test::{test_ws_handshake_rejects_missing_jti, test_ws_handshake_rejects_jti_replay}` |
| 6 | Initial `H` on connect, `H` every 5 s, close after 10 s without a reply | U | `handler.rs:97-125` + `main.rs:387-407`; `heartbeat_test` covers the `GatewayState` predicates only. Two send paths (GW-HEARTBEAT-DUAL-PATH) and two timeout paths; the main-loop reap leaves the handler parked (GW-REAP-LEAKS-HANDLER, section 2) |
| 7 | 10/s per user, 100/s per IP → `E[1006]` | T (struct) | `handler.rs:224-246`; `rate_limit_test`, `rate_limit_e2e_test` (drive the limiter maps directly) |
| 8 | 1000/s per gateway instance | M | `rate_limit::per_instance` (`rate_limit.rs:62`) is never called; `rate_limit_test::rate_limit_1000_per_sec_per_instance` tests the constructor |
| 9 | Per-IP limiter map bounded at 10 000, FIFO eviction | T | `state.rs:248-263`; `state_test::ip_limiter_map_is_bounded` |
| 10 | Circuit breaker: consecutive failures → Open → cooldown → one half-open probe | T (struct) | `circuit.rs:32-88`; `circuit_test` (6). The only failure the handler records is a UDP send error (`handler.rs:360-364`) |
| 11 | Internal queue over limit → reject with `OVERLOADED` | M | pending full sends `E[1003,"pending queue full"]` (`handler.rs:329-338`); circuit open sends `E[5,"overloaded"]` (`handler.rs:252`). Same condition class, two codes |
| 12 | Never block on internal congestion | U | `send_raw` is non-blocking (`handler.rs:347`) |
| 13 | Max frame 4 KB, text only, fatal protocol violation closes | U | `ws.rs:305-326, 364-369`, `handler.rs:179-183`; no test sends an oversize, unmasked or FIN=0 frame |
| 14 | Max 64 subscriptions per connection | S | the gateway has no subscriptions; marketdata owns them |
| 15 | Max 5 WS connections per user | T | `state.rs:121-129`; `heartbeat_test::connection_limit_rejects_sixth`. The 6th is closed after the 101 with no E frame (`handler.rs:68-73`) |
| 16 | REST `/health` + `/v1/*` reads on the same port, same JWT | M | only `/health` and `/v1/symbols`, neither reads a JWT (`rest.rs:153-183`); the reads are deferred by 26's own status |
| 17 | REST rate limits 5/s user, 50/s IP | M | `rest.rs` has no limiter |
| 18 | Cancel by cid needs a cid → oid map; cancel by oid is stateless | M | cancel by oid also needs a pending hit (`handler.rs:408-418`) because `CancelRequest` needs `symbol_id` and the C frame has none; the 10 s sweep (`main.rs:381-385`) then makes any order older than 10 s uncancellable by either key (GW-PENDING-TIMEOUT-STRANDS-RESTING-ORDERS) |
| 19 | Runtime model: sharded reactors, bounded per-connection outboxes, egress off the reactor (11:32-69, "not yet implemented") | M | egress recv is off the reactor (`cast_recv.rs:178-254`) but routing and writes share one reactor (`main.rs:349-373`); outboxes unbounded (`state.rs:150-165`, GW-OUTBOUND-UNBOUNDED) |
| 20 | Lifecycle step 7: query open orders via `{O:[]}` or `GET /v1/orders` | S | both are Post-MVP in the same spec (11:166-172) and 26:7 |
| 21 | Rate limit exceeded → `E[1006]` or `ORDER_FAILED(RATE_LIMIT)` by stage | U | only `E[1006]`; `1006` also means "order forward failed" (`handler.rs:366`) and "cancel forward failed" (`:413, :430`) |

### 20-network.md (gateway rows)

| # | Requirement | St | Evidence |
|---|---|---|---|
| 1 | monoio / io_uring, one process | U | `main.rs:241-245`; structural |
| 2 | JWT in headers | T | as 11 #1 |
| 3 | Rate limit per user / IP | T (struct) | as 11 #7 |
| 4 | Ingress cap 10k orders → `OVERLOADED` | M | cap T (`pending_test::backpressure_rejects_at_10k_overloaded`); code is 1003, as 11 #11 |
| 5 | TLS; "termination at load balancer OR gateway (config-dependent)" (20:181, 313-321) | M | no TLS in the crate: plain `TcpListener` (`ws.rs:31`), `rustls` only in `[dev-dependencies]` (`Cargo.toml`) |
| 6 | UUIDv7 at entry, pending VecDeque | T | `order_id_test`, `pending_test` |
| 7 | Risk forwards each fill to the user's gateway, gateway forwards it | T | `route.rs:35-100`; `order_lifecycle_test::order_lifecycle_fill_done_routes_to_user`. A self-trade sends the same `F` twice to one user (`route.rs:87-88`) |
| 8 | Reject with `OVERLOADED` until the risk stream is up and CaughtUp (20:449-452) | M | no readiness gate; `/ready` reports pending saturation only (`main.rs:127`) |
| 9 | Gateway↔Risk partition: reject at ingress when the buffer is full | U | `handler.rs:329-338` |
| 10 | User-hash sharding, several gateways into risk (20:126-148, 385-397) | M | risk sends every response to one `RSX_GW_CAST_ADDR` (`rsx-risk/src/config.rs:376`, `cast_loop.rs:116`); the playground `stress` and `M3S` presets start two gateways (`rsx-playground/runtime.py:64-65`), and orders on `gw-1` get no response (inference from the code, not run) |
| 11 | Native clients over casting/UDP from the internet (20:310-311) | S | casting is unauthenticated by design (4-cast §10.4, repo CLAUDE.md "Trust boundaries") |
| 12 | Env names `RSX_GATEWAY_ADDR`, `RSX_RISK_ADDR` (20:416-423) | S | code reads `RSX_GW_LISTEN` (`config.rs:510`), `RSX_RISK_CAST_ADDR` (`main.rs:159`) |
| 13 | "Retries Risk connection with backoff" | S | UDP; there is no connection |

### 29-rpc.md

| # | Requirement | St | Evidence |
|---|---|---|---|
| 1 | UUIDv7 at gateway entry, carried in every record | T | `handler.rs:284`; `order_id_test` (8) |
| 2 | Pending `VecDeque` **per user**, LIFO with scan fallback (29:88-95) | M | one process-wide `VecDeque` (`state.rs:35`, `pending.rs:275-278`), linear scans over up to `max_pending` = 10 000 entries per find and remove (`pending.rs:296-332`), lookups not scoped to the user (GW-CANCEL-NOT-USER-SCOPED). `OrderContext` does not exist (S part) |
| 3 | Out-of-order completions correlated by order id | T | `pending.rs:296-308`; `pending_test::pending_linear_scan_on_mismatch`, `stream_ordering_test` |
| 4 | Per-order timeout 10 s: client gets an error, pending removed (29:202-226) | M | the sweep drops the result (`main.rs:383`, `let _ = …remove_stale`): no frame, no counter; it also sweeps orders that already rested (`route_order_inserted`, `route.rs:102-120`, never marks them). `pending_test::pending_timeout_removes_stale_order` covers the removal only |
| 5 | Link failure → every pending order `ORDER_FAILED(NETWORK_ERROR)` (29:177-194) | M | no link-failure detection; rsx-cast consumes risk heartbeats inside `CastReceiver` and exposes no liveness signal (`rsx-cast/src/cast.rs` public API: `try_recv_with`, `reset_after_replay`, `expected_seq`) |
| 6 | No automatic retry | U | structural |
| 7 | Rate limit → `ORDER_FAILED(RATE_LIMIT)` | U | sends `E[1006]` (11 allows either) |
| 8 | Buffer full → `ORDER_FAILED(OVERLOADED)` | M | as 11 #11 |
| 9 | 1000/s per instance | M | as 11 #8 |
| 10 | Circuit "10 failures/min", "tracks ME stream health" | S | code counts consecutive cast send errors (`circuit.rs:73-88`), as 11 says; the gateway cannot see the ME |
| 11 | Gateway on Tokio, one task per session, thread pool | S | monoio single reactor + one cast-recv thread (`main.rs:241`, `cast_recv.rs:165-167`) |
| 12 | Dedup window 5 min, ME map pruned every 10 s | S | ME window is 1 h (`rsx-matching/src/dedup.rs`); SPEC-DEDUP-WINDOW-5MIN-VS-1H |
| 13 | ORDER_DONE "fully filled OR resting" (29:119, 149) | S | a resting order gets ORDER_INSERTED → `U` status 1 (`route.rs:102-120`); ORDER_DONE is terminal only |
| 14 | "Stale order policy 5 minutes" inside the duplicate-order code fence (29:233-235) | S | garbled section |

### 18-messages.md (gateway rows)

| # | Requirement | St | Evidence |
|---|---|---|---|
| 1 | Fills precede ORDER_DONE; DONE / FAILED remove the pending entry | T | `route.rs:122-153, 201-226`; `stream_ordering_test::fills_precede_order_done_in_stream`, `order_lifecycle_test` |
| 2 | Exactly one completion per order reaches the client (18:459-465) | M | a user cancel makes the ME emit ORDER_CANCELLED and ORDER_DONE(CANCELLED) (`rsx-matching/src/cancel.rs:74, 82`); risk forwards both (`rsx-risk/src/cast_loop.rs:257, 267`); the gateway turns each into a status-2 `U` (`route.rs:155-183`, `122-153`). `stream_ordering_test::exactly_one_completion_per_order` covers fill + done only |
| 3 | No completion within 10 s → error to the user (18:466-469) | M | as 29 #4 |
| 4 | `OrderCancelledRecord.reason` carries `CANCEL_REASON_*` to the client | M | `route.rs:175` sends `reason: 0` (GW-CANCEL-REASON-DROPPED) |
| 5 | `OrderFailedRecord.reason` (`FailureReason` 0-13) forwarded | T | `route.rs:218`; `stream_ordering_test::order_done_or_failed_never_both` asserts reason 1; parser accepts 0-13 (`records.rs:247`) |
| 6 | `gw_in_ns` / `gw_out_ns` stamps (18:163-180) | U | `F` carries all five (`route.rs:40-56`), done `U` too (`route.rs:146-150`); inserted, cancelled and failed `U` send zeros (`route.rs:113-117, 176-180, 219-223`). 18:195-197 says OrderDone carries none — stale (S part) |
| 7 | Gateway validates margin (18:41, 62, 102, 601-627), updates positions on fill (18:629-648), converts float input (18:671) | S | contradicts the trust boundary (risk owns margin) and the wire: clients send tick/lot integers (`records.rs:181-182`); `convert::price_to_fixed`/`qty_to_fixed` (`convert.rs:5-36`) are used by tests and one bench only |
| 8 | Record 9 `RECORD_ORDER_REQUEST` "reserved"; domain wire records live in `rsx-messages` (18:116-121, 135) | S | record 9 is the live GW→risk order; its struct is `rsx_risk::types::OrderRequest` (`rsx-risk/src/types.rs:21`), so `rsx-gateway` links all of `rsx-risk` (`Cargo.toml:20`, used once at `handler.rs:26`) |
| 9 | ORDER_DONE(RESTING) on insert (18:265-300) | S | ME emits ORDER_INSERTED; see 29 #13 |

### 49-webproto.md

49:13-15 says "when they disagree, the code wins and this spec is the bug".
The live order channel disagrees with almost all of 49.

| # | Requirement | St | Evidence |
|---|---|---|---|
| 1 | Protocol is protobuf, `[u32 BE len][body]`, binary WS frames (49:43-67, 379-383) | S | gateway speaks JSON text frames `{"N":[…]}` (`records.rs:139-168, 338-439`) and closes on any non-text data opcode (`handler.rs:179-183`); rsx-term speaks the same JSON (`rsx-core/wire/order.go:32-60`) |
| 2 | "Removed: JSON single-letter frames" (49:432-438) | S | they are the live channel; 49:6-8 itself says JSON. No spec documents the N/C/U/F/E/H/Q fields or the E codes the gateway sends |
| 3 | In-band `WireHello` auth (49:146-156, 388-392 path b) | S | header auth only (`ws.rs:152-187`) |
| 4 | Side / TIF / status enums | T | `records.rs:177-194, 240-243`; `records_test::enum_*` |
| 5 | Failure reason table (49:121-134) | S | 8 and 13 differ from 18 (SPEC-FAILURE-REASON-TABLE-DRIFT) |
| 6 | Parse error → error frame, not a disconnect | U | `handler.rs:185-211` sends `E[1001]` / `E[1002]` and continues |
| 7 | Post-MVP queries O, P, A, FL, FN, M | M | deferred (11 §Post-MVP) |
| 8 | QUIC / HTTP-3 listener (49:401-417, 58) | M | roadmap |
| 9 | Retry with the same `cid` → exactly-once ME acceptance (49:421-425; GUARANTEES §1.0 rows 1-2, :45-53, :94-97) | M | the gateway mints a fresh UUIDv7 per `N` (`handler.rs:284`) and ME dedup is keyed by `(user_id, order_id)`, so a retried cid is a new order. GW-DEDUP-WINDOW-DOCUMENTED-NOT-IMPLEMENTED is this gap |
| 10 | No order-accepted ACK; first response is an update or fill | U | `handler.rs:340-385` sends nothing on success |
| 11 | Liquidation `Q` routed to the user | T (struct) | `route.rs:185-199`; `cast_recv_test::decode_liquidation_dispatches_correctly`, `records_test::serialize_q_frame_liquidation` |
| — | Cites `rsx-term/wire/order.go`, `rsx-tui/src/wire.rs` | S | the terminal wire layer is `rsx-core/wire/order.go` |

### 26-rest.md

| # | Requirement | St | Evidence |
|---|---|---|---|
| 1 | `GET /health`, no auth | T | `rest.rs:154-157`; `rest_contract_test::health_*` (6) |
| 2 | `/health` body has `version`, `uptime_sec`, `ts` (26:26-29) | M | body is `{"status":"ok"}`; `health_body_is_status_ok` pins the short body |
| 3 | `/v1/symbols` as `{M:[[sym,tick,lot,name]…]}` with status | M | `rest.rs:158-179` writes `{"symbols":[{"id",…}]}`, no name or status; the values are env defaults (`config.rs:462-485`, decimals hard-coded 8 at :478-479); `symbols_body_contains_symbols_key` pins the contradiction. rsx-term reads this shape (`rsx-term/conn/symbols.go:42-58`) |
| 4 | `/v1/account`, `positions`, `orders`, `fills`, `funding` with JWT → 401 | M | deferred by 26:7; the playground serves its own local `/v1/*` (`rsx-playground/server.py:9524-9900`) |
| 5 | Errors as `{E:[code,msg]}`, 400/429/500 | M | 404 body is `{"error":"not found"}` (`rest.rs:119-132`) |
| 6 | 5/s user, 50/s IP, 429 + `Retry-After` | M | none |
| 7 | No write endpoints | U | `parse_path` drops the method (`rest.rs:98-103`): `POST /health` answers 200 |
| 8 | WS upgrade on `/ws` (26:10-11) | S | any path with `Sec-WebSocket-Key` upgrades (`ws.rs:69-73`); the clients use `/ws/private` and `/` |

### 37-testing-gateway.md (G1–G31)

| St | Rows |
|---|---|
| T (16) | G1 (JSON frames, `records_test`), G3 (JWT header), G4, G5, G6 (struct), G8 (state predicates), G11, G12 (struct), G14, G18, G21, G22, G24, G29, G30 (i64 on the wire), G31 |
| U (5) | G2 casting link, G10 no ACK, G15 single link, G17 tick/lot pre-validation (`prevalidation_test` checks `symbol_configs.len()` only), G25 no permessage-deflate (`ws.rs:125-131` negotiates no extension) |
| M (6) | G7 OVERLOADED (code 1003), G9 order timeout, G16 CONFIG_APPLIED cache (below), G20 per-instance cap, G23 fee (always `0`, `route.rs:51`; `FillRecord` has no fee), G28 exactly one completion (cancel sends two) |
| S (2) | G19 stale-order policy (a client rule, nothing in the gateway), G27 dedup 5 min (ME 1 h) |
| N/A (2) | G13 S/X subscribe (marketdata), G26 load-balancer sharding |

Also S: the unit/E2E file list (`protocol_test.rs`, `gateway_e2e_test.rs`)
names files that do not exist; G3 "A fallback" and G21 "FailureReason
0-12" (13 exists). Benchmark section: see section 1 "Other specs" (22).

Tests that pin nothing the gateway does (counted as no coverage):
`rate_limit_test::{rate_limit_10_per_sec_per_user, rate_limit_100_per_sec_per_ip,
rate_limit_1000_per_sec_per_instance}` (constructors), `config_test::config_defaults`
and `heartbeat_test::{heartbeat_config_interval_5s, heartbeat_config_timeout_10s}`
(env defaults), `state_test::ws_new_order_accepted_and_filled` (a pending
push/remove, despite the name), `state_test::concurrent_sessions_isolated`
(asserts its own setup), `state_test::circuit_breaker_opens_on_gateway_overload`
(duplicate of `circuit_test::circuit_open_after_10_failures`),
`records_test::fill_fee_*` (4; serialize fees the gateway never produces),
`convert_test` float cases (4; dead functions),
`prevalidation_test::config_cache_updated_on_config_applied` and
`state_test::config_applied_reloads_symbol_overrides` (pass only because they
`set_var` the env the process read at boot).

### 47-validation-edge-cases.md (gateway rows)

| # | Requirement | St | Evidence |
|---|---|---|---|
| 1 | §1.1 / §1.2 px ≤ 0, qty ≤ 0 → reject | U | `handler.rs:267-276` via `convert.rs:40-47`; `convert_test::validate_*` test the predicates |
| 2 | §1.3 cid 1-20 bytes; empty or longer rejected | U | `records.rs:183-188`; every test uses a 20-char cid (`records_test.rs:3`). The handler's own `> 20` check (`handler.rs:279-282`, `E[1010]`) is unreachable |
| 3 | §1.7 unknown symbol → `E[1007]` | U | `handler.rs:259-265`; `prevalidation_test::symbol_not_found_rejects_early` checks the vector length |
| 4 | §1.8 side > 1, tif > 2 rejected at parse | T | `records.rs:178-194`; `records_test::{parse_n_frame_invalid_side_rejected, parse_n_frame_invalid_tif_rejected}` |
| 5 | §3.4 cancel by cid or oid through the pending map | M | the C parser takes a cid only if it is exactly 20 chars (`records.rs:220-228`) while `N` accepts 1-20 (`records.rs:184`): a 1-19-byte cid can be placed and never cancelled by cid (the playground pads to 20 since `68e4554a`; diary 2026-10-21 "15-char cids broke gateway cancel") |
| 6 | §3.4 wrong user → reject | M | lookup is not scoped to the requester (`pending.rs:323-332`, `handler.rs:408, 425`) (GW-CANCEL-NOT-USER-SCOPED) |
| 7 | §4.1 gateway updates its tick/lot cache on CONFIG_APPLIED | M | `ConfigAppliedRecord` carries no tick/lot; `reload_symbol_overrides` re-reads the process env (`state.rs:90-119`), which a running process cannot change, so the cache never moves; the gateway's tick/lot come from env at boot (`config.rs:462-485`) |
| 8 | §4.3 gateway timeout → "order timeout" to the user | M | as 29 #4 |
| 9 | §5 step 1 `E[1001]` utf8, `E[1002]` parse / missing field | U | `handler.rs:185-211` |
| 10 | §5 step 2 per-instance 1000/s → `E[5,"overloaded"]` | M | as 11 #8 |
| 11 | §5 step 3 `E[1007]`, `E[1008]`, `E[1009]` | U | `handler.rs:257-277` |
| 12 | §7 rejection rate by reason; "gateway accepts, ME rejects" counts | M | `rejects` and `orders_processed` (`main.rs:143-149`) are never incremented anywhere (GW-HEALTH-ORDERS-PROCESSED-NEVER-INCREMENTED) |
| — | §1.1 / §1.2 / §4.4 float conversion and precision rows | S | the wire carries i64 tick/lot units (`records.rs:181-182`) |
| — | §2.1 "the gateway is the appropriate place" for a `price * qty` overflow check (47:253-255) | S | risk saturates and owns margin (47:241-251); a second owner contradicts the trust boundary |
| — | §1.3 cites `protocol.rs`, §1.7 cites `handler.rs:361-371` | S | `records.rs`, `handler.rs:259-265` |

### Other specs and docs that name gateway behaviour

A read-only sub-agent audited these against the code. I re-checked the
load-bearing rows by hand: risk has no `ReplicationService`
(`grep ReplicationService rsx-risk/src` → nothing), risk never sends
`RECORD_LIQUIDATION` (grep → only the persist-local struct), the
`deploy/systemd/rsx-gateway.service` `CPUAffinity=1`, no gateway entry in
`bench-baseline.json`, and 9-deploy:36-41 "Gateway Tier (2+ hosts)". Only
the rows that matter for the plan are listed. The counts table below
covers every row.

| Spec | Requirement | St | Evidence |
|---|---|---|---|
| GUARANTEES | §1.0, §1.3, §3.1 step 3: a retry with the same cid is accepted once; §2.3, §7: gateway dedup, 5 min, keyed by `order_id` | M | 49 #9 above; the client never sends an `order_id` |
| GUARANTEES | §5.1 partition detected in 1 s, users get an error; §6.1/§6.2 gateway rejects when Risk or the ME is slow | M | no liveness signal (29 #5); the only reject signals are pending-full and an open circuit (`handler.rs:250-254, 329-338`) |
| GUARANTEES | §1.3 "Gateway → Risk: SPSC ring", "gateway returns ack"; §6.3 "ME → Gateway 4096-entry ring"; §3.1 "fills never touch Gateway" | S | casting/UDP (`handler.rs:347`); no ack (GUARANTEES:45 itself); the one ring is cast-recv → reactor, 8192 (`main.rs:60`); `route_fill` delivers fills |
| 4-cast / 10-replication | FAULTED or RECONNECT → replay from the producer, then `reset_after_replay` | M | nothing serves the risk → gateway stream: risk embeds no `ReplicationService`. With `RSX_RISK_REPLICATION_ADDR` unset the gap is skipped (`cast_recv.rs:94-103`); set, the connect fails and the drain panics (`cast_recv.rs:126`), and the panic hook exits. A successful drain discards the records (`cast_recv.rs:117-124`). `handle_replay` has no test; `replay_drain_test` covers `drain_replay` only |
| 10-replication | §10.11 a duplicate ORDER_DONE ack is idempotent via `client_order_id` dedup | M | no dedup; `route_order_done` re-emits through the `rec.user_id` fallback (`route.rs:132-136`) |
| 9-deploy, 30-scenarios | gateway tier of 2+ hosts; `stress` preset load-balances two gateways (30:223, :247) | M | 20 #10 |
| 9-deploy, 45-tiles, 11:48-49 | each order-path busy-spin thread owns a pinned core | M | the `gw-cast-recv` busy-spin thread is spawned (`main.rs:216`) before the pin (`main.rs:231-239`), which pins the reactor thread only; with `CPUAffinity=1` both share core 1 |
| 9-deploy | rolling upgrade: stop accepting, drain, close | M | shutdown does one `tick`, joins the recv thread and drops the runtime (`main.rs:333-342`); no close frames, `live` never cleared |
| 9-deploy | env table complete and correct | S | missing `RSX_GW_MAX_PENDING`, `ORDER_TIMEOUT_MS`, `HEARTBEAT_INTERVAL_S`, `IDLE_TIMEOUT_S`, `CB_*`, `RSX_MAX_SYMBOLS`, `RSX_DEFAULT_*`, `RSX_RISK_REPLICATION_ADDR`; port tables disagree with `deploy/env/gateway.env` |
| 22-perf, repo rule | gateway latency tables carry p50 and p99; gateway benches gated | M | the five Criterion benches report means and are absent from `bench-baseline.json` (`scripts/bench-gate.sh:105` prints NEW, never fails); `ws_order_latency` prints p50/p99 but needs a live cluster and is a `[[bench]]` target, so `cargo bench --workspace` runs it although its header says hand-run (`ws_order_latency.rs:47-49`); `reports/20260723_gateway-egress.md` has a p50-only table |
| 33-telemetry, 12-health | `rate_limit_pct`, `circuit_breaker_state`, `orders_per_sec`, error rate | M | `/metrics` exports `pending`, `connections` and two counters nobody increments (`main.rs:128-152`) |
| 33-telemetry | structured JSON logs | M | `tracing_subscriber::fmt()` without `.json()` (`main.rs:95-98`); workspace-wide, out of this pass |
| 38-liquidator, 37 G14 | `Q` liquidation notice reaches the user | M | gateway half exists (`route.rs:185-199`); `rsx-risk` never sends `RECORD_LIQUIDATION`, so it has no producer |
| 24 §9.1, 57, 19, 21 | gateway tick/lot cache follows CONFIG_APPLIED | M | 47 #7. 57:42, :234-236 "handler only bumps a counter" is S: it calls the env re-read (`state.rs:77-88`) |
| 56 | SQPOLL gated on `RSX_GW_CORE_ID` | M | `main.rs:241-245` builds no `uring_builder`; 56 marks it future |
| 54, 58 | QUIC listener, `WireHello` first-frame auth | M | roadmap; 54:30-33, :143 describe it as current (S) |
| 59 | `gw_in_ns` at ingress, `gw_out_ns` on F and done U, upstream stamps forwarded | T | `route.rs:41, 52-56, 146-150`; `order_lifecycle_test.rs:114-121, 153-165`, `records_test::serialize_f_frame_fill` |
| 59 | aggregate latency histograms via `latency_sample!` | M | the macro compiles away without the `latency-trace` feature (`rsx-log/src/lib.rs:131-137`); nothing enables it |
| 6-consistency | gateway acts on Fill / Inserted / Cancelled / Done, ignores BBO; fills precede DONE | T | `cast_recv_test::decode_*` (9), `stream_ordering_test` |
| 28-risk | N shards by user hash, gateway reads a vshard map | M | one `CastSender` (`main.rs:172`); 28:432 says v1 is single-shard; GW-SINGLE-SHARD-NO-ROUTING (by design) |
| 45-tiles, 25-process, 1-architecture, 5-codepaths, 44-testing, 6 §5, 57, 22 | "no SPSC ring in the gateway", "0 pinned / 1 monoio / 0 rings", "Gateway tile", "Core Tile (busy-spin)", "SIGTERM = crash", ME → Gateway arrow, `protocol_test.rs`, `main.rs::route_fill`, 44:84 "ORDER_DONE precedes all fills", stale `server.py` and `main.rs` line citations | S | one ring + doorbell (`cast_recv.rs:157-173`); threads: reactor, `gw-cast-recv`, `rsx-log-drain`, health, plus a tokio runtime inside `drain_replay`; shutdown flushes (`main.rs:333-342`); `records_test.rs`; `route.rs:35`; invariant 1 |
| crate README | env names `RSX_GW_LISTEN_ADDR`, `RSX_GW_HEARTBEAT_INTERVAL_MS`, `_TIMEOUT_MS`, `RSX_GW_CIRCUIT_*`; "clients query order status"; "circuit trips on sustained Risk failures"; 16 test files incl. handler/protocol | M/S | code reads `RSX_GW_LISTEN`, `_HEARTBEAT_INTERVAL_S`, `RSX_GW_IDLE_TIMEOUT_S`, `RSX_GW_CB_*` (`config.rs:510-519`); no query API; the circuit trips on local send errors only (`handler.rs:363`); 17 + 2 files, no handler test (GW/MD-README-ENV-DRIFT) |
| crate ARCHITECTURE | runtime model "main loop polls the casting receiver", module table (`protocol.rs`, maps in `pending.rs`, breaker in `state.rs`), "readable 10 ms timeout", "6th connection: handshake rejected", "pending held until a terminal record", "reaper closes", "no internal queueing" | S/M | recv thread + ring (`cast_recv.rs`); `records.rs`, `circuit.rs`; `handler.rs:127-148`; the 6th is dropped after the 101 (`handler.rs:60-74`); 29 #4; GW-REAP-LEAKS-HANDLER; GW-OUTBOUND-UNBOUNDED |
| notes/io-model.md, compare/README | recv path on its own unsaturated core; `SO_REUSEPORT` shards; lead number "11.5 ms p50 … egress-starvation bug" | M/S | thread unpinned; `TcpListener::bind` without reuseport (`ws.rs:31`); the number predates `cast_recv.rs` |

### Counts

| Spec | T | U | M | S |
|---|---|---|---|---|
| 11-gateway | 6 | 6 | 6 | 3 |
| 20-network | 4 | 2 | 4 | 3 |
| 29-rpc | 2 | 2 | 5 | 5 |
| 18-messages (gateway rows) | 2 | 1 | 3 | 3 |
| 49-webproto | 2 | 2 | 3 | 5 |
| 26-rest | 1 | 1 | 5 | 1 |
| 37-testing-gateway (G1–G31, +2 N/A) | 16 | 5 | 6 | 2 |
| 47-validation (gateway rows) | 1 | 5 | 6 | 3 |
| **Core specs** | **34** | **24** | **38** | **25** |
| GUARANTEES | 0 | 3 | 5 | 4 |
| 22 / 33 / 12 (perf, telemetry, health) | 1 | 3 | 7 | 1 |
| 4 / 10 / 6 / 1 (transport, replication, consistency, architecture) | 3 | 6 | 3 | 3 |
| 9 / 45 / 25 / 28 / 30 (deploy, tiles, process, risk, scenarios) | 2 | 8 | 7 | 9 |
| 24 / 19 / 21 / 57 / 59 / 38 / 41 / 55 / 42-44 / 5 | 13 | 12 | 7 | 6 |
| 54 / 56 / 58 (roadmap) | 1 | 0 | 4 | 1 |
| **Other specs** | **20** | **32** | **33** | **24** |
| crate README / ARCHITECTURE / notes / compare | 3 | 2 | 9 | 7 |

M rows overlap heavily: the timeout appears in 18, 29, 37, 47; OVERLOADED in
11, 20, 29, 37; per-instance limit in 11, 29, 37, 47; dedup in 29, 37, 49,
GUARANTEES, 10; replay in 4, 10, GUARANTEES; multi-gateway in 9, 20, 30.
Distinct items after de-duplication: 18 fix-now (the spec decides), 13
proposals, 3 founder decisions (section 2).

## 2. Open gateway bugs (BUGS.md)

| Entry | Verdict | Why |
|---|---|---|
| GW-PENDING-TIMEOUT-STRANDS-RESTING-ORDERS (`BUGS.md:732`) | **fix now** (reclassify from "needs sign-off") | still real (`main.rs:381-385`, `route.rs:102-120`). 18:425-469 and 29:202-226 decide it: the timeout is "no completion from the ME", and a resting order's completion is ORDER_INSERTED (18 calls it ORDER_DONE(RESTING)). So the sweep skips an entry once ORDER_INSERTED arrived, and an entry it does drop gets `U` status 3 reason TIMEOUT (10). The capacity policy for resting entries is the one open part → GW-RESTING-CAPACITY |
| GW-CANCEL-NOT-USER-SCOPED (`:1846`) | **fix now** | still real (`pending.rs:323-332`, `handler.rs:408, 425`; the entry cites `:580, 618`). 47 §3.4 and 29:90 ("per user") decide |
| GW-CANCEL-REASON-DROPPED (`:243`) | **fix now** | still real (`route.rs:175`). The entry asks who owns the `U` reason slot; the status decides it: status 3 → `FailureReason`, status 2 → `CANCEL_REASON_*` (18:184-195), else 0. rsx-term and the playground ignore the slot for status 2 (`rsx-core/wire/order.go:312`), so nothing breaks; the 49 rewrite (step 11) documents the slot |
| GW-HEALTH-ORDERS-PROCESSED-NEVER-INCREMENTED (`:894`) | **fix now** | still real (`main.rs:147`; the entry cites `:215`); `rejects` (`main.rs:144`) has no writer either. 47 §7 wants rejects by reason |
| GW-OUTBOUND-UNBOUNDED (`:1388`) | **fix now** | still real (`state.rs:150-165`; entry cites `:144, :152`). 11:59-61 decides "outboxes are bounded". Order updates cannot be coalesced or dropped (GUARANTEES §1.0 "dropping a fill is never sound"), so overflow closes the connection; the client reconnects (11:88-90). `RSX_GW_MAX_OUTBOUND` |
| GW-HEARTBEAT-DUAL-PATH (`:1417`) | **fix now** | still real (`handler.rs:97-125` and `main.rs:387-392`; the broadcast never stamps `last_heartbeat_sent_ns`). The broadcast is also what wakes an idle handler, so the fix keeps the main-loop path, stamps the send, and drops the handler's |
| GW-CANCEL-NO-RATELIMIT (`:1839`) | keep, **proposal** | still real (`handler.rs:386-438`; entry cites `:547-649`). The spec limits "orders" (11:107-115) and gates "new orders" on the circuit (11:122-128); whether a cancel spends a token is unstated. Recommend: cancels spend the per-user and per-IP tokens, the circuit never gates a cancel |
| GW-DEDUP-WINDOW-DOCUMENTED-NOT-IMPLEMENTED (`:438`) | **rewrite → GW-CID-RETRY-NOT-IDEMPOTENT, decision owed** | the entry says ME dedup carries correctness. It does not for client retries: GUARANTEES §1.0 and 49:421-425 promise that a retry with the same cid is accepted once, but every `N` gets a fresh UUIDv7 (`handler.rs:284`) and the ME key is `(user_id, order_id)`. GUARANTEES §7 names the key `order_id`, which the client never sends. **Options:** (a) the gateway keeps `(user_id, cid) → order_id` for 5 min and re-sends a retried cid with the same order id, so ME dedup returns DUPLICATE_ORDER_ID (3) or accepts the first copy once; memory is bounded by the rate limit (10/s × 300 s = 3 000 entries per user); needs rsx-term to keep cids unique per user across reconnects (`Cid(counter)` restarts per process, `rsx-core/wire/order.go:14-18`); (b) GUARANTEES §1.0/§2.3/§3.1/§7 and 49 say a retry is a new order. **Default if nothing is decided:** (b) |
| GW-STAMP-LATENCY-LIVE (`:1208`) | **stale — rewrite** | done for `F` and the done `U` (`route.rs:40-56, 146-150`); the entry cites `handler.rs:301`. Remainder: inserted, cancelled and failed `U` frames send zeros because their records are 64 B with no room (18:195-197) → proposal under 59; consumer work is rsx-term's |
| ME-ENGINE-LATENCY-NOT-REPORTED (`:1213`) | **re-home to rsx-term** | the gateway forwards both stamps; the rest is `rsx-term/ui/update.go:773-779` |
| GATEWAY-LATENCY (`:1593`, `:1648`) and the 2026-07-23 "Plan: gateway egress fix" block (`:797-867`) | **prune to one measurement entry** | the tile split they defer is shipped (`cast_recv.rs`); what remains is a quiet-box re-measurement (as MATCHING-BENCH-SHARED-HOST-VARIANCE) |
| WS-FORK-DRIFT (`:1394`) | **re-home to marketdata** | the gateway holds the hardened reader; its dead looser copy `ws::ws_read_frame` (`ws.rs:209-271`) goes in step 8; the marketdata half is pass step 5; the shared-module hoist stays a proposal |
| GW/MD-README-ENV-DRIFT (`:1412`) | fix now (docs, step 12) | see section 1 "Other specs" |
| DRAIN-REPLAY-4X-DUP (`:1422`) | keep, record only | needs a new shared crate (rsx-cast frozen) |
| ME-GW-DIRECT-SPEC-GAPS (`:1226`) | keep, **proposal** | spec rewrite owed |
| GW-SINGLE-SHARD-NO-ROUTING (`:1620`, by design) | keep | one risk sender per gateway; GW-MULTI-GATEWAY below is the related open half |
| PLAYGROUND-MARKET-ORDER-REJECTED-BY-GATEWAY (`:2021`) | **re-home to playground** | 47 §1.1 rejects px 0 at the gateway and no frame has a market type: the gateway is right; the playground sends a crossing limit. The "unaligned limit filled" half is GW-SYMBOL-CONFIG-FROM-ENV |
| GATEWAY-CRASH-UNDER-WS-CHURN (`:2054`, RESOLVED), RETURN-PATH-INTERMITTENT-DROP (`:1899`, RESOLVED), IOC-NOT-HONORED (`:1626`, FIXED) | **prune** | resolved in their own text; the Playwright `@long` tag note under the churn entry moves to the playground |

New entries found in this audit (recorded by step 0):

| New entry | Verdict | Evidence |
|---|---|---|
| GW-REPLAY-DISCARDS-RECORDS (MED) | **fix now** | after a FAULTED or RECONNECT the recv thread drains the risk replay, but the apply closure only logs (`cast_recv.rs:117-124`): every fill and completion in the gap is fetched and dropped, and `replay.rs:1-7` claims they are re-emitted. GUARANTEES §1.0 "dropping a fill is never sound". Fix: decode each replayed record with `decode_cast_event` and push it on the same ring. The playground never sets `RSX_RISK_REPLICATION_ADDR` for the gateway (`rsx-playground/runtime.py:312-323`), so there the gap is skipped (`cast_recv.rs:94-103`) |
| GW-REAP-LEAKS-HANDLER (MED) | **fix now** | the main loop removes an idle connection's state (`main.rs:394-407`) but its handler task stays parked in `select!{readable, egress.wait()}` (`handler.rs:136-148`) with nothing to wake it: the socket stays open, and a later frame on it is processed for a connection that no longer exists (orders forwarded, responses dropped by `push_to_user`). Read from the code, not reproduced |
| GW-CANCEL-CID-LENGTH-ASYMMETRY (LOW) | **fix now** | 47 §1.3 / §3.4; `records.rs:184` vs `:220-228`. Key = 32 hex chars → oid, 1-20 bytes → cid |
| GW-TIMEOUT-SILENT (MED) | **fix now** | 29 #4, 18 #3, 47 §4.3; `main.rs:383` |
| GW-SELF-TRADE-FILL-SENT-TWICE (LOW) | **fix now** | `route.rs:87-88` pushes the same `F` to taker and maker; when they are one user, two frames arrive for one fill (18:392-400 "one FILL per match"); rsx-term folds each `F` into position (`rsx-core/wire/order.go:329-370`), so a self-trade double-counts (inference) |
| GW-OVERLOADED-CODE (LOW) | **fix now** | pending full → `E[5,"overloaded"]` as 11:26-27, 20:206-208, 29:279 and the circuit path (`handler.rs:252`) |
| GW-INSTANCE-RATE-LIMIT-MISSING (LOW) | **fix now** | 11:112, 29:293, 47 §5 step 2 → `E[5,"overloaded"]`; `RSX_GW_RL_INSTANCE` |
| GW-WIRE-ORDERREQUEST-IN-RISK (LOW) | **fix now** | 18:116-121; move `OrderRequest` to `rsx-messages` beside `CancelRequest` (`rsx-messages/src/lib.rs:375`), drop the `rsx-risk` dependency |
| GW-REST-HEALTH-BODY (LOW) | **fix now** | 26 #2, #5, #7: `/health` fields, `{E:[…]}` error bodies, GET only |
| GW-CANCEL-DOUBLE-TERMINAL (MED) | **proposal** | 18 #2. Which record is terminal for a user cancel is an ME/gateway contract (post-only cancels emit ORDER_CANCELLED alone, `rsx-book/src/matching.rs:103-111`). **Options:** (a) ME stops emitting ORDER_CANCELLED before ORDER_DONE on a user cancel; (b) the gateway sends a status-2 `U` for ORDER_CANCELLED only when its reason is not USER_CANCEL. **Default:** (a), decided in the matching follow-up |
| GW-RESTING-CAPACITY (MED) | **proposal** | after the strand fix, resting orders keep their pending entry, so one user with many resting orders can fill the shared `max_pending` cap and make every other user's order OVERLOADED. The spec has a 10k ingress cap (29:277) and no resting-order cap. **Options:** per-user cap; separate resting index with its own cap. **Default:** separate index, cap = `max_pending` |
| GW-SYMBOL-CONFIG-FROM-ENV (MED) | **proposal** | 47 #7, 26 #3: tick/lot come from env at boot, decimals are hard-coded 8, the CONFIG_APPLIED reload re-reads an env that cannot change, and `/v1/symbols` serves it to rsx-term. **Options:** (a) carry tick/lot in `ConfigAppliedRecord` (16 spare bytes; ME-REPLAY-IGNORES-CONFIG-CHANGES needs the same change); (b) read `symbol_config_applied` from Postgres at boot and on CONFIG_APPLIED, on a sidecar thread (risk's shape). **Default:** (b) for boot and names, (a) for live changes |
| GW-RISK-READINESS-GATE (MED) | **proposal** | 20 #8, 29 #5, GUARANTEES §5.1 (detect a partition in 1 s): needs a risk-liveness signal; `CastReceiver` consumes heartbeats internally and exposes none (rsx-cast frozen). **Options:** rsx-cast API (sign-off) or a gateway-level probe record |
| GW-TLS-TERMINATION | **decision owed** | 20 #5. **Options:** (a) spec: TLS terminates at the load balancer only; (b) TLS listener in the gateway (`monoio-rustls`). **Default:** (a) |
| GW-MULTI-GATEWAY (MED) | **proposal** | 20 #10: risk has one gateway peer; needs per-gateway senders in risk and a user → gateway map |
| GW-REST-READ-ENDPOINTS | **proposal** | 26 #3-#6, 49 #7: a read surface over risk state (spec 53 read-service); decide gateway vs playground ownership of `/v1/*` |
| GW-FILL-FEE-ALWAYS-ZERO (LOW) | **proposal** | 37 G23; `FillRecord` has no fee; risk computes it |
| GW-SHARDED-REACTORS | **proposal** | 11:32-69, "not yet implemented" in the spec itself |
| GW-CAST-RECV-THREAD-UNPINNED (MED) | **fix now** | 11:48-49 "dedicated pinned busy-spin thread"; `main.rs:216` vs `:231-239`. `RSX_GW_CAST_CORE_ID` pins it; unset leaves it unpinned with a warning |
| GW-BENCH-NO-P99 (LOW) | **fix now** | repo rule + 22: no gateway bench result carries p99, none is gated |
| GW-RISK-STREAM-NOT-SERVED (MED) | **proposal** (risk side) | 4-cast / 10-replication row above: nothing serves the risk → gateway stream, and pointing `RSX_RISK_REPLICATION_ADDR` anywhere makes the gateway exit on the first gap. **Options:** (a) risk embeds a `ReplicationService` over its gateway-stream WAL and the gateway routes the replay (GW-REPLAY-DISCARDS-RECORDS is the gateway half); (b) drop the gateway drain and the variable, count skipped gaps, and say in GUARANTEES that a client can miss a notification after a risk → gateway gap. **Default:** (a). Either way the drain failure becomes a warning + skip, not a panic |
| GW-JTI-SINGLE-USE | **decision owed** | 11:100-105 says `jti` is optional and unwired; the code makes every token single-use per handshake (`ws.rs:169-182`). rsx-auth mints one `jti` per login (`rsx-auth/src/rsx_auth/jwt_util.py:29-36`), and the playground opens a new WS per order with the same cookie token (`rsx-playground/server.py:6212-6217`), so a signed-in user's second request is a 401 (inference, not run); the Trade spec's test token has no `jti` at all (`rsx-playground/tests/test_helpers.ts:30-36`), which may be PLAYGROUND-TRADE-SPEC-EMPTY-RESULT. **Options:** (a) keep single-use, as 11:88-90 "re-opens with fresh JWT"; the playground keeps one connection per session or fetches a fresh token per connection; (b) the tracker rejects a `jti` only across different peers. **Default:** (a), spec 11 updated |
| GW-SHUTDOWN-NO-DRAIN (LOW) | **proposal** | 9-deploy rolling-upgrade section (a STUB in the spec): stop accepting, flip `/ready`, close frames |
| GW-LIQUIDATION-NO-PRODUCER (LOW) | **re-home to risk** | 38 row above: the `Q` path has no producer |

Split: **18 fix-now items** (6 existing entries: GW-PENDING-TIMEOUT-STRANDS-RESTING-ORDERS,
GW-CANCEL-NOT-USER-SCOPED, GW-CANCEL-REASON-DROPPED,
GW-HEALTH-ORDERS-PROCESSED-NEVER-INCREMENTED, GW-OUTBOUND-UNBOUNDED,
GW-HEARTBEAT-DUAL-PATH, plus the docs half of GW/MD-README-ENV-DRIFT; 11 new:
GW-REPLAY-DISCARDS-RECORDS, GW-REAP-LEAKS-HANDLER,
GW-CANCEL-CID-LENGTH-ASYMMETRY, GW-TIMEOUT-SILENT,
GW-SELF-TRADE-FILL-SENT-TWICE, GW-OVERLOADED-CODE,
GW-INSTANCE-RATE-LIMIT-MISSING, GW-WIRE-ORDERREQUEST-IN-RISK,
GW-REST-HEALTH-BODY, GW-CAST-RECV-THREAD-UNPINNED, GW-BENCH-NO-P99),
**13 proposals** (GW-CANCEL-NO-RATELIMIT, GW-CANCEL-DOUBLE-TERMINAL,
GW-RESTING-CAPACITY, GW-SYMBOL-CONFIG-FROM-ENV, GW-RISK-READINESS-GATE,
GW-MULTI-GATEWAY, GW-REST-READ-ENDPOINTS, GW-FILL-FEE-ALWAYS-ZERO,
GW-SHARDED-REACTORS, GW-RISK-STREAM-NOT-SERVED, GW-SHUTDOWN-NO-DRAIN, plus
the kept ME-GW-DIRECT-SPEC-GAPS and DRAIN-REPLAY-4X-DUP), **3 decisions**
(GW-CID-RETRY-NOT-IDEMPOTENT, GW-TLS-TERMINATION, GW-JTI-SINGLE-USE),
**10 stale entries to prune, re-home or rewrite** (GW-STAMP-LATENCY-LIVE,
ME-ENGINE-LATENCY-NOT-REPORTED, the two GATEWAY-LATENCY entries and the
2026-07-23 plan block, WS-FORK-DRIFT, PLAYGROUND-MARKET-ORDER-REJECTED-BY-GATEWAY,
GATEWAY-CRASH-UNDER-WS-CHURN, RETURN-PATH-INTERMITTENT-DROP, IOC-NOT-HONORED,
GW-LIQUIDATION-NO-PRODUCER to risk).

## 3. LOC — what to cut in `rsx-gateway/src`

3 692 lines at `3b798420`, 299 of them `src/*_test.rs`, so 3 393 of code.

| Cut | Evidence | Approx. |
|---|---|---|
| Float conversion, banned by the repo rule and unused by the binary | `convert.rs:1-37` (`price_to_fixed`, `qty_to_fixed`); used by `convert_test` and `benches/gateway_bench.rs:205-206` only | −37 |
| Server-side parsers for frames the gateway never accepts | `records.rs:161-165, 232-336` (`U`, `F`, `E`, `Q` parsers; the handler answers them `E[1004]`, `handler.rs:449-457`) and the client-side `N`/`C` encoders `records.rs:340-363`; tests decode through a `tests/common` helper | −130 |
| Dead and test-only WS code | `ws_read_frame` (`ws.rs:209-271`, unused, accepts unmasked frames), `ws_handshake` (`:84-94`, tests only), the one-line wrapper `ws_handshake_from_request` (`:75-82`), and the `Option` mask after the mask is already required (`:371-376, 384-388`) | −80 |
| Six inline copies of "serialize E, write, warn" next to `send_error` | `handler.rs:188-195, 202-210, 330-337, 365-371, 394-405, 450-456` vs `:515-523` | −35 |
| Order id split by hand twice, joined once | `handler.rs:291-296`, `:462-492` (`build_cancel`) vs `route.rs:16-21`; one `order_id::split` / `join` pair | −25 |
| Unreachable `E[1010]` cid check, `let _unused = (oid, qty)` with a "SAFETY" comment | `handler.rs:279-282`, `:380-384` | −9 |
| Three completion routes repeat remove-pending + owner fallback + five zero stamps | `route.rs:122-226`; one helper | −20 |
| Legacy `gw_in_ns` fallback expression ×4 | `route.rs:69-73, 79-83, 94-98`, `cast_recv.rs:204-208` | −15 |
| Symbol env parsing twice | `state.rs:90-119` re-implements `config.rs:462-485` (and is the dead env re-read, GW-SYMBOL-CONFIG-FROM-ENV) | −20 |
| `main.rs` boilerplate | `log_effective_gateway_config` called once (`:62-88`), eight `// SAFETY: fail-fast at startup` markers on ordinary `expect`s, env addr parsing outside `config.rs` (`:159-169`, `:231`), twelve locals copied out of `config` (`:221-229, 254-256`), and the dead `GatewayConfig::risk_addr` read from `RSX_GW_RISK_ADDR` and only logged (`config.rs:5, 92`) | −45 |
| Test-only constructors and helpers in `src` | `rate_limit::{per_user, per_ip, per_instance, advance_time_by}` (`rate_limit.rs:45-64`); `jwt::validate_jwt` (`jwt.rs:34-36`) | −25 |
| Stale doc | `replay.rs:1-7` ("round-1 leaves this as a debug log — see `main.rs::handle_replay`") | 0 (truth fix) |

About −440 lines in `src`; the fixes in steps 2-7 add roughly 150 back. Tests
(outside `src`): the tautologies listed under 37 (≈ −150), replaced by the
handler tests of step 1. Not cut: `order_id_timestamp_ms` (`order_id.rs:37-41`,
pins the time-sortable property 29 requires) and `JtiTracker::{len, is_empty}`
(clippy `len_without_is_empty`).

## 4. Playground and rsx-term

Surveyed by a read-only sub-agent; I re-checked by hand the E-code drop
(`server.py:5794-5798`), the two-gateway presets (`runtime.py:64-65`), the
cookie-token reuse (`server.py:6212-6217`), the jti-less test token
(`tests/test_helpers.ts:28-36`) and the only gateway counter read
(`server.py:4074-4077`). Playground paths are under `rsx-playground/`.

**Playground — shown today**
- Login through rsx-auth (GitHub OAuth, `server.py:9949-9957`), which mints a
  gateway JWT with `aud`, `iss`, one `jti` per token; `/api/orders/test`
  mints a JWT for any `user_id` (`_mint_user_token`, `server.py:5685`).
- Order entry `{"N":[…]}` with GTC/IOC/FOK, reduce-only, post-only on the
  Orders page (`pages.py:3211-3262`); the Trade page has GTC/IOC and
  reduce-only only (`pages.py:1404-1426`). Cids are minted 20 chars
  (`_new_cid`, `server.py:5721`).
- `U` status → filled / resting / cancelled / rejected; rejected `U` reason
  0-13 decoded by `REJECT_REASONS` (`server.py:5743-5758`, includes 13).
- Real cancel by cid for resting rows (`server.py:6751, 6784`) and on the
  Trade page (`server.py:6222`).
- Gateway process up/down, pid, restarts (`scan_processes`, topology node
  `_topo_gateway`, `server.py:2607`); TCP probe `/x/gateway-mode`
  (`server.py:4777`).
- One gateway metric: `orders_processed` summed over `gw-*` for the
  Cast Connections "Gateway → Risk" row (`server.py:4074-4077`), which is
  always 0 because nothing increments it.
- Recovery feed greps logs for "faulted" / "replay" (`server.py:2479-2545`).
- `stress_client.py:283-331` is the only consumer of the five latency
  stamps and of rejects by reason; `stress_report_page` renders neither.

**Playground — missing (steps 14-15)**
- Any read of gateway `/health`, `/ready`, `/metrics` beyond that one
  counter: pending used/cap, connections, rejects by code, circuit state,
  risk-stream state, heartbeat. Needs the counters of step 6 first.
- E codes: the playground keeps only the message text (`server.py:5798`);
  no code → name table, and 1006 has three meanings today (step 6 splits
  them).
- Rate-limit and circuit-breaker demos: the playground raises the limits to
  5000/user and 10000/IP (`runtime.py:318-319`), and a Playwright spec
  asserts the topology does not mention the circuit breaker
  (`tests/play_topology.spec.ts:156-171`).
- Order timeout and stranded-order visibility (`/x/stale-orders`,
  `server.py:4830`, counts playground rows, not the gateway's pending).
- Cancel by order id; a client-chosen cid and "resend same cid" (the only
  duplicate demo is the playground's own `x-idempotency-key`,
  `server.py:1603-1621`).
- FOK and post-only on the Trade page.
- A persistent private WS: `_send_ws_frame` opens one connection per order
  and returns the first non-heartbeat frame within 2 s (`server.py:5608-5650`),
  so later fills, cancels and `Q` frames never reach the dashboard.
- Latency stamps on recent orders and the order trace.
- Multiple gateways: every UI path uses one `GATEWAY_URL`
  (`server.py:77`, `:5819`) while the `stress` / `M3S` presets start two
  (GW-MULTI-GATEWAY).
- REST: the playground serves its own `/v1/*` (`server.py:9524-9900`); the
  gateway's `/v1/symbols` is shadowed (GW-REST-READ-ENDPOINTS decides owner).
- Hazards for the playground plan, not gateway code: the cookie-token reuse
  and the jti-less test token (GW-JTI-SINGLE-USE); stale comments that a
  resting GTC gets no ack (`server.py:5772-5775`); `tests/test_order_api.py`
  mocks a `"price_band"` reason that no spec defines; no
  `RSX_RISK_REPLICATION_ADDR` in the gateway spawn env
  (`runtime.py:312-323`) — correct today, since setting it crashes the
  gateway (GW-RISK-STREAM-NOT-SERVED).

**rsx-term — shown today** (`rsx-term/`, wire layer `rsx-core/wire/`)
- Mints its own JWT per dial with a fresh `jti` (`rsx-term/conn/jwt.go:39-67`),
  Bearer header (`conn/live.go:154-158`), reconnect with backoff
  (`live.go:376`); echoes `H` (`live.go:353-355`); link shown live/offline
  (`ui/view.go:92-95`).
- `N` with GTC/IOC/FOK, reduce-only (`r`), post-only (`p`)
  (`ui/keymap.go:109-110`); cancel by cid, paired to the order by qty FIFO
  (`rsx-core/wire/order.go:155-179`).
- `U` 1 → open order, 0/2 → "done", 3 → `rejected: failure_reason=<n>`;
  `E` → `rejected: <code>: <msg>`; `F` → position and fill count.
- RTT measured client side; `GET /v1/symbols` at start for tick/lot
  (`conn/symbols.go:42-58`).

**rsx-term — missing** (hand-off to pass step 4, rsx-term; not built here)
- Reason and E-code name tables (TERM-FAILED-REASON-NOT-DECODED).
- Filled vs cancelled: `Done` drops the status (`order.go:312`).
- `Q` frames ignored (`Fold` handles U/F/E/H only, `order.go:243-256`).
- Internal and engine latency legs from the frame stamps
  (ME-ENGINE-LATENCY-NOT-REPORTED; `ui/view.go:726-729`).
- Heartbeat age; auth-failure reason (a 401 is hidden in reconnect backoff).
- Cids unique per user across restarts if GW-CID-RETRY-NOT-IDEMPOTENT takes
  option (a) (`Cid(counter)`, `order.go:14-18`).

**Tests touching the gateway today.** Real gateway behaviour is asserted only
by `tests/play_matching.spec.ts:41, 78-114` (resting, FOK reject, post-only
cancel, cancel through the gateway), `tests/api_matching_test.py:700-720,
808-845, 974-1000`, and `tests/stress_integration_test.py:290-614` (JWT
101/401, heartbeat echo, `E[1007]`, `E[1002]`, `E[1005]`, `E[1006]`,
`E[1003]`) — the last one is in no make target. `api_orders_test.py` asserts
recorded flags or HTTP 200; `play_trade.spec.ts:95, 105` accepts "error" as a
pass; `acceptance_test.py:302-376` (real fill and cancel) is in no make
target. Go: `rsx-core/wire/order_test.go`, `rsx-term/conn/live_test.go:114-240`.

**Gate registration.** A new `tests/api_gateway_test.py` goes in the
`_gate-api` list after `api_matching_test.py` (`Makefile:56-69`), hermetic
like `api_matching_test.py` (stub `_fetch_daemon_metrics` and `send_*`;
`_gate-startup` does not build the gateway). A new `/x/gateway-*` partial
goes in `HTMX_PARTIALS` (`tests/test_htmx_partials.py:9`). A new
`tests/play_gateway.spec.ts` goes in the `process-control` project's
`testMatch` (`tests/playwright.config.ts:134-141`). Latency stays out of
play specs.

## 5. Steps

One `sonnet` agent and one commit per step, sequential on the shared tree;
the parent reads each diff before starting the next. rsx-cast is not edited
in any step: every fix uses the `CastSender` / `CastReceiver` /
`drain_replay` APIs that exist. No step moves an auth or wire-parsing
check out of the gateway, or a margin, position or book check into it.
"Counts" = `cargo test -p rsx-gateway` pass/ignored totals (153/0 at
`3b798420`), recorded before each step.

| # | Step (commit) | Done when | Verify |
|---|---|---|---|
| 0 | `docs(bugs)`: triage per section 2 — prune the 3 resolved entries, collapse the GATEWAY-LATENCY entries and the 2026-07-23 plan block into one quiet-box measurement entry, re-home ME-ENGINE-LATENCY-NOT-REPORTED (rsx-term), WS-FORK-DRIFT (marketdata), PLAYGROUND-MARKET-ORDER-REJECTED-BY-GATEWAY and the churn `@long` note (playground), GW-LIQUIDATION-NO-PRODUCER (risk); rewrite GW-DEDUP-WINDOW-… as GW-CID-RETRY-NOT-IDEMPOTENT and GW-STAMP-LATENCY-LIVE to its remainder; reclassify GW-PENDING-TIMEOUT-STRANDS-RESTING-ORDERS as fix-now; re-cite every live GW-* entry to current lines; add the new entries | every GW-* entry cites a file:line that exists at HEAD | `grep -n 'handler.rs:547\|handler.rs:580\|main.rs:477\|main.rs:215\|state.rs:144\|handler.rs:301' BUGS.md` → nothing; `git diff --stat` touches BUGS.md only |
| 1 | `test(gateway)`: connection-handler harness — `tests/common/mod.rs` runs `handle_connection` on a monoio loopback with a `GatewayState`, a `CastSender` aimed at a test UDP socket standing in for risk, and a masked-frame WS client; `tests/handler_test.rs` pins today's behaviour: `N` → `RECORD_ORDER_REQUEST` with the frame's fields + a pending entry; `E[1001/1002/1004/1005/1006/1007/1008/1009]` and `E[5]` (circuit forced open) each leave the connection open; oversize, unmasked, FIN=0 and binary frames close it; cancel by oid and by cid forward a `CancelRequest`; a `route_*` push reaches the socket; wrong `aud` / `iss` → 401. Delete the tautologies listed under 37 | the 11 / 47 / 20 rows that are handler code move U → T | `cargo test -p rsx-gateway --test handler_test`; `cargo test -p rsx-gateway`; `make lint` |
| 2 | `fix(gateway)`: pending per user (29:88-95) — `PendingOrders` keyed by `user_id` with one global count for the `max_pending` cap; cancel lookup scoped to the requester (GW-CANCEL-NOT-USER-SCOPED, 47 §3.4); `C` key = 32 hex chars → oid, 1-20 bytes → cid (GW-CANCEL-CID-LENGTH-ASYMMETRY) | user A cannot reach user B's same-cid order; a 15-byte cid cancels | new `handler_test` cases; `pending_test` updated; `cargo test -p rsx-gateway` |
| 3 | `fix(gateway)`: order timeout per 18:425-469 and 29:202-226 — an entry is marked resting on ORDER_INSERTED and the sweep skips it; a swept entry sends `U[oid,3,0,0,10,…]` (TIMEOUT) to its user and counts as a reject (GW-PENDING-TIMEOUT-STRANDS-RESTING-ORDERS, GW-TIMEOUT-SILENT); the cap stays `max_pending` (GW-RESTING-CAPACITY holds the policy question) | a resting order older than the timeout still cancels; an unanswered order gets a TIMEOUT `U` | `handler_test` with a short `order_timeout_ms`; `cargo test -p rsx-gateway` |
| 4 | `fix(gateway)`: egress truth — replayed records go through `decode_cast_event` onto the same ring, and a failed drain warns and skips instead of panicking (GW-REPLAY-DISCARDS-RECORDS); one `F` per self-trade (GW-SELF-TRADE-FILL-SENT-TWICE); a cancelled `U` carries `rec.reason` (GW-CANCEL-REASON-DROPPED); an unknown `final_status` warns instead of becoming "filled" (`route.rs:127`) | a fill inside a replayed gap reaches the client once | `cast_recv_test` case using the `replay_drain_test` WAL server fixture; `order_lifecycle_test` cases; `cargo test -p rsx-gateway` |
| 5 | `fix(gateway)`: connection lifecycle — the main loop owns the heartbeat (broadcast stamps `last_heartbeat_sent_ns`) and the idle reap; the handler's own heartbeat and timeout go; a reap signals the connection's egress and the handler closes the socket when its state is gone (GW-HEARTBEAT-DUAL-PATH, GW-REAP-LEAKS-HANDLER); `outbound` bounded by `RSX_GW_MAX_OUTBOUND`, overflow closes the connection (GW-OUTBOUND-UNBOUNDED, 11:59-61) | one `H` per interval; an idle client's socket is closed at the timeout; a client that never reads is closed at the cap | `handler_test` cases; `cargo test -p rsx-gateway` |
| 6 | `fix(gateway)`: overload codes and counters — pending full → `E[5,"overloaded"]` (GW-OVERLOADED-CODE); per-instance limiter `RSX_GW_RL_INSTANCE` (default 1000) → `E[5]` (GW-INSTANCE-RATE-LIMIT-MISSING); a forward failure → `E[8,"network error"]` (`FailureReason::NETWORK_ERROR`, 29:189-194) so `1006` means rate limited only; `orders_processed` on each forward, `rejects` by E code and by `U` failure reason, `circuit_state` in `/metrics` (GW-HEALTH-ORDERS-PROCESSED-NEVER-INCREMENTED, 47 §7, 33) | `curl $RSX_GW_HEALTH_ADDR/metrics` shows non-zero `orders_processed` after one order and a per-code reject | `handler_test` cases; unit test on the snapshot closure; `cargo test -p rsx-gateway` |
| 7 | `fix(gateway)`: pin the `gw-cast-recv` thread on `RSX_GW_CAST_CORE_ID` (GW-CAST-RECV-THREAD-UNPINNED) | the recv thread's `Cpus_allowed_list` is the configured core | process test modelled on `rsx-matching/tests/me_process_test.rs` (`/proc/<pid>/task/*/status`); `cargo test -p rsx-gateway` |
| 8 | `refactor(messages)`: `OrderRequest` moves from `rsx-risk/src/types.rs:21` to `rsx-messages` beside `CancelRequest`; gateway and risk import it from there; `rsx-risk` leaves the gateway's dependencies (GW-WIRE-ORDERREQUEST-IN-RISK). Same layout, same bytes | gateway no longer depends on risk | `cargo tree -p rsx-gateway -e normal \| grep rsx-risk` → nothing; `cargo test -p rsx-messages -p rsx-risk -p rsx-gateway` |
| 9 | `fix(gateway)`: REST truth per 26 — `/health` adds `version`, `uptime_sec`, `ts`; errors as `{"E":[code,msg]}`; non-GET → 400 (GW-REST-HEALTH-BODY). `/v1/symbols` keeps its shape until GW-SYMBOL-CONFIG-FROM-ENV is decided (rsx-term parses it) | the 26 #2, #5, #7 rows are T | `rest_contract_test` updated; `cargo test -p rsx-gateway` |
| 10 | `refactor(gateway)`: the section 3 cuts not already taken — float conversion, server-side `U/F/E/Q` parsers and `N/C` encoders (tests decode through `tests/common`), dead WS code, `send_error` everywhere, one order-id split/join, one completion helper in `route.rs`, one latency-anchor fn, one symbol-env parser, `main.rs` boilerplate into `config.rs`, test-only constructors; fix the `replay.rs` doc. No behaviour change | `wc -l rsx-gateway/src/*.rs` below the step-9 total; counts unchanged | `cargo test -p rsx-gateway` (same counts); `make lint`; `make fmt-check` |
| 11 | `perf(gateway)`: in-process tail harness `benches/handler_tail.rs` (frame in → `RECORD_ORDER_REQUEST` out; risk record in → client frame written), p50 + p99 from one run (rsx-book `tail_bench.rs` is the model); the five Criterion benches into `bench-baseline.json`; `ws_order_latency` marked `bench = false` so `cargo bench` stops running it; `reports/YYYYMMDD_gateway-tail.md`; crate README points at it (GW-BENCH-NO-P99) | every row of the report has p50 and p99; `make perf-gate` lists the gateway benches as gated, not NEW | `cargo bench -p rsx-gateway --bench handler_tail`; `make perf-gate` |
| 12 | `docs(specs)`: reconcile the S rows — 49 rewritten around the JSON channel the gateway and rsx-term speak (frames, fields, E-code table, the `U` reason slot by status), protobuf kept as the target section; 11 (jti per GW-JTI-SINGLE-USE, heartbeat, REST, runtime model as built, subscriptions), 18 (trust-boundary rows, record 9, RESTING, stamps on OrderDone), 20 (env names, native UDP, TLS per GW-TLS-TERMINATION), 26 (current shapes), 29 (per-user pending, monoio, DONE semantics, garbled section), 37 (file list, G3, G21, G27), 47 (float rows, `protocol.rs`, §2.1), GUARANTEES (§1.3, §2.3, §3.1, §6.3, §7 per GW-CID-RETRY-NOT-IDEMPOTENT), 5 / 9 / 22 / 44 / 45 / 57 / 59 / 1 / 25 / 55 stale rows | no spec cites `protocol.rs`, `gateway_e2e_test.rs`, `rsx-term/wire/` or `RSX_GATEWAY_ADDR` | `grep -rn 'protocol\.rs\|gateway_e2e_test\|rsx-term/wire/\|RSX_GATEWAY_ADDR' specs/2 GUARANTEES.md` → nothing |
| 13 | `docs(gateway)`: crate `CLAUDE.md` (doc conventions + keeper sections: trust boundary, reactor + recv thread + ring, pending lifetime rule, where the E-code table lives, no floats, rsx-cast frozen); README env table generated from `config.rs` / `main.rs`, test inventory; ARCHITECTURE runtime model, module table, flow; `notes/io-model.md` current state; `compare/README.md` lead number (gateway half of GW/MD-README-ENV-DRIFT) | every env var the README names is read by the code and vice versa | `grep -o 'RSX_[A-Z0-9_]*' rsx-gateway/README.md \| sort -u` vs the same over `rsx-gateway/src` → equal sets |
| 14 | `feat(playground)`: gateway backend — `_collect_gateway_metrics` per `gw-*` (live, ready, state, pending used/cap, connections, `orders_processed`, rejects by code and reason, circuit state); `/api/gateway/counters` + a `/x/gateway-*` partial; E-code name table beside `REJECT_REASONS` and keep the code (`server.py:5798`); new `tests/api_gateway_test.py` in `_gate-api`, partial in `HTMX_PARTIALS` | partials render live numbers | `cd rsx-playground && uv run pytest tests/api_gateway_test.py tests/test_htmx_partials.py` |
| 15 | `feat(playground)`: gateway features in the UI — Gateway tab: burst → `E[1006]` count, overload → `E[5]`, circuit state; cancel by order id; cid field + "resend same cid" (shows today's outcome until GW-CID-RETRY-NOT-IDEMPOTENT is decided); FOK and post-only on the Trade page; heartbeat age; risk-stream state; signed-in path stops reusing one token per request (per GW-JTI-SINGLE-USE); `tests/play_gateway.spec.ts` in `process-control` (functional only, no latency asserts) | each feature visible and asserted | `cd rsx-playground/tests && bunx playwright test play_gateway.spec.ts` |
| 16 | `docs(ship)` + diary: gates and ledger | `make test`, `make integration`, `make lint`, `make fmt-check`, `make gate` green (gate 4 needs a quiet box — record load if it fails, as for risk and matching); BUGS.md pruned; PLAN.md ledger row; diary entry | the gate output itself |

Step 1 comes first because every later fix is asserted through its harness.
Step 3 depends on step 2's pending structure; step 10 waits for 2-9; step 11
measures after 10; steps 14-15 need step 6's counters. Steps 4-9 can swap
order (distinct files except `handler.rs`, which 2, 3, 5 and 6 all touch —
run those in order). Step 12 waits for the three decisions; everything
else is unblocked.

**Founder calls this plan needs (not built):** GW-CID-RETRY-NOT-IDEMPOTENT
(gateway cid window vs documenting that retries are new orders);
GW-TLS-TERMINATION (load balancer only vs a gateway listener);
GW-JTI-SINGLE-USE (single-use tokens vs per-peer); GW-RISK-STREAM-NOT-SERVED
(risk serves its gateway stream vs documented gap loss);
GW-CANCEL-NO-RATELIMIT; GW-CANCEL-DOUBLE-TERMINAL (with the matching
follow-up); GW-RESTING-CAPACITY; GW-SYMBOL-CONFIG-FROM-ENV (with
ME-REPLAY-IGNORES-CONFIG-CHANGES); GW-RISK-READINESS-GATE (needs an rsx-cast
liveness API); GW-MULTI-GATEWAY; GW-REST-READ-ENDPOINTS; GW-SHARDED-REACTORS.
