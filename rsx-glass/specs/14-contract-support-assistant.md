---
status: draft
---

# 14 — Contract: REST reads, identity, assistant, configuration

This file owns the UI's request/response contracts outside the two
WebSockets: the trader's own state reads, the identity and role API of
`rsx-auth`, the gateway's support API the USERS screen reads and writes, the
assistant's chat stream and its local-runner/MCP target, and the complete
configuration surface. WebSocket contracts are
[12-contract-gateway.md](12-contract-gateway.md) and
[13-contract-marketdata.md](13-contract-marketdata.md).

Source pin: citations are at `5d175b6ff04755e02fcca5ebbb918b01b929c23b`.

## The trader's own state (gateway REST)

| Endpoint | Returns | Status at the pin | Source |
|---|---|---|---|
| `GET /health` | liveness | served | [doc specs/2/26-rest.md] |
| `GET /v1/symbols` | per-symbol tick, lot, decimals | served (boot-time values; see MD-22) | [13-contract-marketdata.md](13-contract-marketdata.md) |
| `GET /v1/account` | collateral, equity, uPnL, margins, available | specified, not served | [doc specs/2/26-rest.md] |
| `GET /v1/positions` | non-zero positions | specified, not served | [doc specs/2/26-rest.md] |
| `GET /v1/orders` | open orders | specified, not served | [doc specs/2/26-rest.md] |
| `GET /v1/fills?symbol=&limit=&before=` | fill history | specified, not served | [doc specs/2/26-rest.md] |

The position, uPnL and open orders on screen are derived client-side from this
session's own fills and accepts until these reads ship
[doc specs/2/55-terminal.md "Transport & data sources"]. SR-01: glass restores
from them at startup and after every order-link reconnect, and marks any
figure built only from the session fold with `~` (GW-28). SR-02: account
equity, available margin, liquidation price, ROE%, funding and leverage show
`—` until a source exists (the honesty table in spec 55).

## Identity and roles (`rsx-auth`, real)

Bearer: an already-issued `rsx-auth` session JWT (from the GitHub OAuth login),
never self-minted (US-07). 3 s timeout per call; a non-2xx status is an error
carrying the response body, never swallowed [code rsx-term/support/http.go:14-56].

| Call | Request | Response | Source |
|---|---|---|---|
| me | `GET /roles/me` | `{"role": string|null}` | [code rsx-term/support/http.go Me], [code rsx-auth/src/rsx_auth/app.py:252] |
| list | `GET /roles/users?query=&limit=&cursor=` | `{"users":[{"user_id","login","email","role"|null,"created_at","last_login_at"}]}` | [code rsx-term/support/http.go ListUsers], [code rsx-auth/src/rsx_auth/app.py:260] |
| detail | `GET /roles/users/{id}` | one user object | [code rsx-auth/src/rsx_auth/app.py:271] |
| set role | `POST /roles/users/{id}/role` `{"role","reason","ticket_id"}` + `Idempotency-Key` | `{"role"}` | [code rsx-term/support/http.go SetRole], [code rsx-auth/src/rsx_auth/app.py:288] |
| audit | `GET /roles/audit` | role audit rows | [code rsx-auth/src/rsx_auth/app.py:317] (not used by the terminal) |

Roles: `viewer`, `operator`, `admin`, `auditor`; no role = deny by default.
RBAC [doc specs/2/7-dashboard.md §4]:

| Action | viewer | operator | admin | auditor |
|---|---|---|---|---|
| read user state | Y | Y | Y | Y |
| read audit | N | N | Y | Y |
| write note | N | Y | Y | N |
| adjust collateral | N | N | Y | N |
| freeze upsert / delete | N | N | Y | N |

## Support API (gateway, specified, not served)

Base `/v1/api/support` (spec 7, draft). The terminal's client is written
against this contract and fails cleanly (404) until the gateway ships it; an
in-memory fixture stands in for demos and tests. Founder: the API for accounts
is the gateway [founder F-SCR-08 "the api for this shiuld be the gateway right?"].
The contract as the terminal codes it (verbatim from its doc comment):

```
// rsx-term/support/http.go (exact, the HTTPAccountClient doc comment)
//	GET  /v1/api/support/users/{user_id}/account
//	     -> {"collateral":i64,"equity":i64,"unrealized_pnl":i64,
//	         "initial_margin":i64,"maint_margin":i64,"available":i64}
//	GET  /v1/api/support/users/{user_id}/positions
//	     -> {"positions":[{"symbol_id":u32,"side":i32,"qty":i64,
//	         "entry_px":i64,"mark_px":i64,"unrealized_pnl":i64,"liq_px":i64}]}
//	GET  /v1/api/support/users/{user_id}/fills?symbol_id=&limit=&before=
//	     -> {"fills":[{"oid":u64,"symbol_id":u32,"px":i64,"qty":i64,
//	         "side":i32,"fee":i64,"is_maker":bool,"ts_ns":u64}]}
//	GET  /v1/api/support/users/{user_id}/funding?symbol_id=&limit=&before=
//	     -> {"funding":[{"symbol_id":u32,"amount":i64,"rate_bps":i32,"ts_ns":u64}]}
//	GET  /v1/api/support/users/{user_id}/freezes
//	     -> {"freezes":[{"user_id":u32,"order_id":u64,"reason":string,
//	         "ticket_id":string,"created_at":string}]}
//	POST /v1/api/support/accounts/{user_id}/adjust-collateral
//	     {"delta":i64,"reason":string,"ticket_id":string}
//	     + X-Request-Id, Idempotency-Key headers -> the Account shape above
//	POST /v1/api/support/freezes/upsert
//	     {"user_id":u32,"order_id":u64,"reason":string,"ticket_id":string}
//	     -> one freezes row
//	POST /v1/api/support/freezes/delete
//	     {"user_id":u32,"order_id":u64,"reason":string,"ticket_id":string}
//	     -> {"ok":true}
//	POST /v1/api/support/notes
//	     {"user_id":u32,"note":string,"reason":string,"ticket_id":string}
//	     -> {"ok":true}
```

Spec 7 also lists `GET /users` (directory), `GET /users/{id}/reconcile/frozen-margin`
and `GET /audit`, requires `expected_version` on writes where applicable,
bounds `|delta|` by `ADMIN_MAX_COLLATERAL_DELTA`, and makes every write atomic
with its audit row [doc specs/2/7-dashboard.md §3, §5, §6]. SR-10: glass sends
`X-Request-Id` and a per-action `Idempotency-Key` on every write, and the
`expected_version` it read (US-05). SR-11: the fixture data is labelled as
fixture data on screen whenever it is in use (the terminal shows fixture
accounts with no label on the offline path) — a requirement inferred from H-1/H-2;
the founder never ruled on demo labels (OD-15).

Fixture (deterministic; users 1 and 2) [code rsx-term/support/fixture.go]:
user 1 collateral 100,000,000, equity 104,500,000, uPnL +4,500,000, IM
12,000,000, MM 6,000,000, available 88,000,000, one long PENGU position;
user 2 collateral 25,000,000, equity 21,200,000, uPnL −3,800,000, one short SOL.
An unknown user is honestly flat, not an error; writes mutate the in-memory
fixture; notes are accepted and not stored. Glass's own fixture rows match
[code rsx-glass/accounts.go FixtureAccountRows].

## Assistant: the chat stream (arizuko `/chat/{token}`, shipped)

| Item | Contract | Source |
|---|---|---|
| gate | one URL (`RSX_TERM_ASSIST`, the full chat URL including the minted route token); unset = no client, zero dials; the token IS the credential | [code rsx-term/main.go:311-322], [doc rsx-term/notes/assistant.md] |
| request | `POST <url>`, `Content-Type: application/json`, `Accept: text/event-stream`, body `{"content": "<prompt>", "topic": "<thread id>"}` | [code rsx-term/assistant/client.go:95-127, 174-180] |
| stream | SSE: `event:` / `data:` lines, blank-line terminated frames; `data` is JSON `{role, content, …}` | [code rsx-term/assistant/client.go:129-169] |
| frame → event | `event: round_done` ends the turn; `role: assistant` with content → `Reply`; `event: status` or content starting `⏳` → `Status` (progress, never content); `role: user` is the echoed prompt, ignored; empty content ignored | [code rsx-term/assistant/client.go:189-215] |
| keep-alive | SSE comments (`: ping`, every ~15 s) do NOT reset the idle timer (a wedged turn still sends them) | [code rsx-term/assistant/client.go:151-155] |
| cutoff | 180 s of inactivity (reset by each real frame) cancels the turn → `Failed("timed out")`; no total deadline (a cold container can take tens of seconds) | [code rsx-term/assistant/client.go:14-19, 217-224] |
| failures | non-200 → `Failed("HTTP <code>")`; stream ends without a reply → `Failed("assistant closed the stream without a reply")` / `"assistant finished without a reply"` | [code rsx-term/assistant/client.go:124-168] |
| threads | the client mints the topic (arizuko never returns its auto-topic) | [code rsx-term/ui/news_view.go:805-810] |

Events carry the topic; the UI drops events for other topics (stale threads).
The client's event channel is unbuffered beyond 64 and the send blocks
[code rsx-term/assistant/client.go:21-23, 171-172].

## Assistant: the local runner and MCP (designed, not built)

Owner of the design: `specs/2/60-terminal-assistant.md` (draft; it survives
the deletion of `rsx-term` and names `rsx-term` paths that are recoverable at
the pin). Its requirements are carried by AS-01 … AS-08 in
[07-screen-assistant.md](07-screen-assistant.md). Contract points the glass
implementation must honour, from spec 60:

| Point | Contract |
|---|---|
| runner | arizuko's `container.Run(cfg, folders, Input)` imported as a Go library, unchanged; one blocking call = one isolated Claude Code turn in Docker; its return is the end-of-turn signal only |
| reply wire | the reply and the resume session id arrive as newline-framed JSON-RPC `submit_turn {turn_id, session_id, status, result?, error?, timed_out?}` on a unix socket the UI owns; mid-turn progress as `submit_status {turn_id, text}`; the UI demuxes these before handing every other line to its MCP server; acks `{"result":{"ok":true}}` |
| correlation | `Input.MessageID` = a fresh turn id per ask; `turn_id` echoes it; `thread → session_id` map for multi-turn resume |
| MCP | an in-process server on the same socket; read tools served from an immutable state snapshot published after each fold; control tools as typed semantic messages through the UI's own update loop |
| folder | a tier-3 agent folder (`rsx/term/desk`) mounted as `$HOME` with the standing RSX instructions (CLAUDE.md, PERSONA.md) |
| config | `Image arizuko-ant`, `Timeout ~5m`, `IdleTimeout ~2m` (zero stops the container instantly), `HostAppDir` a real arizuko checkout, an explicit model and `QueryTimeoutMs` just under the timeout |
| prerequisites | docker, the built image, an arizuko checkout, a Go version ≥ arizuko's |
| open | egress (OD-12), control ceiling (OD-11), folder lifecycle, absorbing the runner's prompt annotations |

[founder F-AST-04 "inly reusing its claude code runner", "yes this means ideally just run runner unchanged.."]

## Configuration surface

Env-only configuration (repo rule: config via env vars). Every variable the
terminal read, its default, and what glass does with it:

| Variable | Default | Meaning | Source |
|---|---|---|---|
| `RSX_GW_URL` | `ws://127.0.0.1:8088`; `mock` = offline | order gateway WS | [code rsx-term/main.go:34-38] |
| `RSX_MD_URL` | `ws://127.0.0.1:8180` | market-data WS | [code rsx-term/main.go:39-40] |
| `RSX_TUI_USER` | `0` | trader user id (u32; a malformed value is fatal) | [code rsx-term/main.go:41-42, 743-757] |
| `RSX_TUI_SYMBOL` | `10` (PENGU-PERP) | primary symbol id | [code rsx-term/main.go:43-44] |
| `RSX_GW_JWT_SECRET` | the dev secret | HS256 secret for self-minted dev tokens | [code rsx-term/main.go:45-46] |
| `RSX_TUI_THEME` | default | `colorblind` swaps bid/ask | [code rsx-term/main.go:47-49] |
| `RSX_SESSION_OPENS` | `day=00:00,Asia=01:00,EU=08:00,US=14:30` | session-open markers; malformed = fatal | [code rsx-term/main.go:50-53, 716-726] |
| `RSX_TERM_STREAM` | off | streaming screens (else the order-entry view) | [code rsx-term/main.go:54-56] |
| `RSX_TERM_VENUE` | `rsx` | venue selection (13 MD-03) | [code rsx-term/main.go:57-63] |
| `RSX_TERM_HL_COINS` / `RSX_TERM_PHX_SYMBOLS` | curated lists; `all` | watch sets | [code rsx-term/main.go:142-200] |
| `RSX_TERM_WATCH` | — | `id:name[:code]` names and codes for RSX symbols | [code rsx-term/main.go:497-518] |
| `RSX_TUI_PRICE_DECIMALS` / `RSX_TUI_QTY_DECIMALS` / `RSX_TUI_TICK` | from `/v1/symbols`, else 6 / 4 / 1 | display overrides | [code rsx-term/main.go:520-540] |
| `RSX_TERM_MAX_NOTIONAL` | 10,000 | per-order notional ceiling, human quote units | [code rsx-term/main.go:360], [code rsx-term/ui/update.go:1161-1163] |
| `RSX_TERM_KEYMAP` | — | JSON rebind file | [code rsx-term/main.go:236-255] |
| `RSX_TERM_KEYCAST` | off | keycast overlay | [code rsx-term/main.go:304-309] |
| `RSX_TERM_NEWS` | off | Tree of Alpha reader | [code rsx-term/main.go:257-267] |
| `RSX_TERM_ASSIST` | — | assistant chat URL | [code rsx-term/main.go:311-322] |
| `RSX_TERM_AUTH_URL` + `RSX_TERM_SUPPORT_TOKEN` | — | identity client | [code rsx-term/main.go:269-282] |
| `RSX_TERM_GATEWAY_SUPPORT_URL` (+ token) | — | support client; `RSX_TERM_SUPPORT_FIXTURES=1` or the mock path uses the fixture | [code rsx-term/main.go:284-302] |
| `RSX_TERM_COLOR` | auto | terminal render tier (terminal-only) | [code rsx-term/ui/stream.go:166-193] |
| `RSX_TERM_SMOKE_HL` | off | opt-in live HL smoke test | [code rsx-term/hl_smoke_test.go] |

Glass already reads `RSX_MD_URL`, `RSX_SYMBOL_ID`, `RSX_SYMBOL`,
`RSX_TUI_THEME`, `RSX_SESSION_OPENS` [code rsx-glass/cmd/glass/main.go:1-20].
SR-20: glass uses one prefix for its own variables (the `RSX_TERM_*` /
`RSX_TUI_*` names die with the terminal; the names glass adopts are OD-16) and
keeps every behaviour in this table. SR-21: no default dials a hosted
deployment (GW-08); the terminal's `make term` target is disabled on purpose
[founder F-ACC-03].
