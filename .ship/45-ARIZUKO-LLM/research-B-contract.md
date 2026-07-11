# arizuko: programmatic ingress/egress contract for a non-browser client

Research target: `/home/onvos/app/arizuko` (read-only). Goal: the simplest
concrete HTTP contract for a Rust trading-terminal client (not a browser) to
POST a message to an agent and get the reply back, across multiple turns of
the same conversation.

**Bottom line**: use webd's **route-token surface** (`/chat/<token>/...`).
It requires no bearer/JWT on the hot path — the token in the URL *is* the
credential — and is explicitly designed for exactly this (spec 5/W, "any URL
+ any valid token" policy). This is far simpler than the authed `/mcp` or
`/me/*` surfaces, which need a full user JWT + `X-User-*` proxy-stamped
headers.

---

## 1. INGRESS — POST a user message

**Primary (recommended): webd route-token surface**

```
POST /chat/{token}
POST /chat/{token}/          (trailing-slash variant, same handler)
```
Registered at `webd/server.go:103-104`:
```go
withCORS("POST /chat/{token}", s.handleChatTokenPost)
withCORS("POST /chat/{token}/", s.handleChatTokenPost)
```
Handler: `webd/route_token.go:143` (`handleChatTokenPost`).

- **Headers**: `Content-Type: application/json` (or
  `application/x-www-form-urlencoded`); `Accept` controls response shape
  (see §2). No `Authorization` header required — the path segment `{token}`
  is the sole credential, looked up via
  `s.stRoutd.LookupRouteToken(token)` (`webd/route_token.go:57`). CORS is
  permissive (`Access-Control-Allow-Origin: *`,
  `webd/route_token.go:453-465`), so this also works from non-browser HTTP
  clients trivially (no CORS preflight issue to work around in Rust anyway).
- **Body schema** (`webd/route_token.go:301-327`, `parseChatBody`):
  ```json
  { "content": "string, required, trimmed non-empty", "topic": "string, optional" }
  ```
  Form-encoded equivalent: `content=...&topic=...`.
- **Rate limit**: per-token, `WEBHOOK_RATE_WEB` (default 20/min)
  (`webd/route_token.go:150-155`, `webd/README.md`).
- Every message is stored via `s.rc.SendMessage` → routd's
  `POST /v1/messages` (`webd/route_token.go:347-359`) — webd is a thin
  adapter, routd is the sole message appender.

**Alternative: routd's own `POST /v1/messages` directly (skip webd)**

routd exposes `POST /v1/messages` as the generic channel-protocol inbound
endpoint (`ARCHITECTURE.md:318`, "Channel Protocol" — `Inbound: POST
/v1/messages → store → route`). This is what *every* adapter (webd, teled,
discd, …) calls. A client could hit it directly, **but**:
- it requires registering as a channel first (`POST /v1/channels/register`
  with `CHANNEL_SECRET`, per `ARCHITECTURE.md:320-325`) or presenting a
  valid channel/service bearer (`chanlib.Auth`, ES256 `service:<name>`
  token when `AUTHD_URL` is set),
- and the reply comes back over a channel egress callback
  (`POST <adapter_url>/send`) that *your* client would need to expose as an
  HTTP server — i.e. you'd have to implement the receiving half of the
  channel protocol yourself.

**Verdict**: `/chat/{token}` via webd is simpler and more direct for a
non-browser client — no channel registration, no bearer, no need to run a
server to receive `/send` callbacks; SSE/poll instead.

---

## 2. EGRESS — how the reply comes back

Response shape is driven by the request's `Accept` header
(`webd/route_token.go:181-184`):

| `Accept` | Behavior |
|---|---|
| `text/event-stream` | SSE stream of the reply, `Content-Type: text/event-stream` |
| `text/html` | HTMX bubble (`<div class="msg user" ...>`) — browser-widget only, not useful here |
| anything else (default) | JSON: `{"user": {...}, "turn_id": "<msg id>", "status": "pending"}`; optionally blocks and returns `"assistant": {...}` if `?wait=<seconds>` (0-120) is given |

**SSE on the POST itself** (`wantSSE` branch, `webd/route_token.go:194-215`):
subscribes to the hub *before* injecting the message
(`s.hub.subscribe(folder, topic)`, `webd/route_token.go:195`), then calls
`serveSSE(w, r, ch)` (`webd/hub.go:82`), which:
- sets `Content-Type: text/event-stream`, `Cache-Control: no-cache`,
  `Connection: keep-alive`, `X-Accel-Buffering: no` (`webd/hub.go:75-80`)
- writes `: ok\n\n` immediately, then relays hub-published frames, with a
  15s `: ping\n\n` keepalive (`webd/hub.go:98-121`)

**Separate SSE endpoints, keyed by turn** (`webd/turn.go`), for
poll-after-post or reconnect flows:

```
GET /chat/{token}/{id}       -> handleTurnSnapshot   (JSON: frames + status, non-streaming)
GET /chat/{token}/{id}/status -> handleTurnStatus     (JSON: status + counts only)
GET /chat/{token}/{id}/sse    -> handleTurnSSE        (SSE, scoped to one turn_id, closes on round_done)
```
(`webd/server.go:108-110`; handlers `webd/turn.go:61,93,121`.) `{id}` is the
`turn_id` returned from the POST response (== the inbound message's `m.ID`,
`webd/route_token.go:224` `"turn_id": m.ID`).

`handleTurnSSE` replays any frames already produced for that turn (via
`Last-Event-Id` header, `webd/turn.go:144-152`), then streams new ones from
the hub filtered to that `turn_id` (`webd/turn.go:175-183`), and emits a
terminal `event: round_done` frame when the turn finishes
(`webd/turn.go:159,187-189`).

**SSE frame format** — two encodings appear:
- Hub-published frames from `injectRouteMessage`/`channel.go` (e.g. the
  echoed user message, agent replies) are `event: message\ndata: {...json...}\n\n`
  (`webd/hub.go:64`, `webd/route_token.go:371`). Payload JSON:
  `{"id","role","content","sender","topic","folder","created_at"}`
  (`webd/route_token.go:361-369`) — `role` is `"user"` or `"assistant"`
  (`messageRole(m)`, referenced `webd/route_token.go:134`).
- `handleTurnSSE` frames additionally carry an `id:` line and event name
  from `frame.Kind` (`"message"` or `"status"`, detected by a `"⏳ "`
  content prefix — `webd/turn.go:27-30`), plus a final
  `event: round_done\ndata: {"turn_id","status"}\n\n` (`webd/turn.go:159,188`).

**No webhook callback exists on this surface** — this is pull (SSE/poll),
not push-to-your-server, unlike the channel-protocol path in §1's
alternative.

**Simplest for a Rust client**: `POST /chat/{token}` with
`Accept: text/event-stream` in one call — the reply streams back on the
same connection, no need to separately track/poll `turn_id`. Use
`GET /chat/{token}/{id}/sse` only if you need to disconnect and resume
watching a specific in-flight turn.

---

## 3. THREADS / SESSIONS — identifiers to track

Two identifiers, at two different granularities:

- **`folder`** (the agent/group) — baked into the token at mint time
  (`jid = "web:" + req.TargetFolder [+ "/" + req.JIDSuffix]`,
  `routd/tokens_http.go:43-50`). One token = one fixed folder; a client
  does not choose or send the folder per-request.
- **`topic`** (the thread) — the client-generated, client-tracked
  conversation identifier. This is the one you must generate and resend.

**Where topic comes from / goes:**
- Request: `topic` field in the POST JSON/form body
  (`webd/route_token.go:158-169`, `parseChatBody`). If omitted, webd
  auto-generates one: `fmt.Sprintf("t%d", time.Now().UnixMilli())`
  (`webd/route_token.go:164`) — **but that generated value is never
  returned in the response**, so a client that wants a stable thread
  **must generate and pass its own `topic` string** to reliably resume it
  (the widget's own JS does exactly this: `rnd()` + `topicFor(id)`,
  `webd/route_token.go:557,564`).
- Response: the JSON reply's `user` payload echoes `topic`
  (`webd/route_token.go:361-369`, field `"topic"`), and `turn_id` is the
  triggering message's ID (`m.ID`), scoped to that topic.
- History: `GET /chat/{token}/{topic}/messages` returns prior messages for
  that `(jid, topic)` pair, oldest→newest, capped at 100
  (`webd/route_token.go:107-138`, registered `webd/server.go:106`).
- SSE reconnect for a topic (not a single turn):
  `GET /chat/stream?token=<t>&group=<folder>&topic=<topic>`
  (`webd/route_token.go:379-427`, `webd/server.go:107`).

**DB schema** (`routd/migrations/0001-initial-schema.sql`):
```sql
CREATE TABLE messages (
  id, chat_jid, sender, content, timestamp, topic, ... , routed_to, turn_id, ...
);
CREATE TABLE sessions (
  group_folder, topic,      -- composite PK
  session_id, parent_topic, forked_at, observed_cursor
);
```
(columns per `ARCHITECTURE.md:392`: `sessions` PK is `group_folder + topic`).
`store.GetSession(folder, topic)` (referenced `ROUTING.md:298-301`) is how
routd maps `(folder, topic)` → the underlying Claude Code session id —
this is the actual multi-turn memory anchor; the client never sees or
sends `session_id` directly, only `topic`.

**So, concretely, to keep one coherent multi-turn thread across separate
HTTP requests, the client must:**
1. Generate a topic string once (e.g. a UUID) on the first message.
2. Send that same `topic` value in the JSON body of every subsequent
   `POST /chat/{token}` in the thread.
3. (Optional) Use `GET /chat/{token}/{topic}/messages` to rehydrate history
   after a restart, since the topic is the only client-held key — nothing
   else is returned to remember.

`turn_id` (from each POST response) is a *per-message* identifier, useful
only for polling/streaming that one turn's status (`/chat/{token}/{id}/sse`
etc.) — it is not the thread identifier and changes every message.

---

## 4. AUTH — minting the token, minimal local-single-user path

The `/chat/{token}` surface itself needs **no bearer token** — the URL
token is the whole credential (`webd/README.md` "Token contract": `/chat/*`
and `/hook/*` "stay unauthenticated; the URL route-token IS the
capability"). The only auth question is **how to mint that token** in the
first place, which is a routd-side, operator-gated call:

```
POST /v1/route_tokens/chat
```
(`routd/server.go:245`, handler `routd/tokens_http.go:22-59`)

- Requires scope `routes:write` or `routes:write:own_group`
  (`s.authz(w, r, "routes:write", "routes:write:own_group")`,
  `routd/tokens_http.go:23`).
- Body (`apiv1.RouteTokenRequest`): `{"owner_folder": "<folder>", "target_folder": "<folder-or-descendant, optional>", "jid_suffix": "<optional>"}`.
- Response 201: `{"token": "...", "url": "<webHost>/chat/<token>/", "jid": "web:<folder>", "owner_folder": "...", "created_at": "..."}`
  (`routd/tokens_http.go:56-58`).

**Local single-user shortcut — the auth gate is fully open by default.**
`routd/server.go:298-306` (`authz`):
```go
// verify==nil is open (single-tenant / local-dev): ok=true, empty sub/folder.
if s.verify == nil {
    return "", "", true
}
```
`s.verify` is nil whenever routd's `AUTHD_URL` is unset — which is the
default for a bare local/single-instance deployment (no `authd` JWKS
verifier wired up). In that mode `POST /v1/route_tokens/chat` needs **no
`Authorization` header at all** — any caller who can reach routd's HTTP
port can mint a token for any folder. This is the minimal path for a
local single-user operator: just call the endpoint directly, no authd
interaction needed.

**If `AUTHD_URL` is set** (multi-tenant / hardened deployment), the caller
needs a bearer with `routes:write` scope. The minimal non-OAuth path is a
**daemon service-key exchange**, not the full user OAuth dance:
```
POST /v1/service-token   (authd/http.go, per authd/README.md)
```
exchanging one of `AUTHD_SERVICE_KEYS` (`principal=secret` pairs configured
on authd) for a `service:<name>` ES256 access token
(`authd/README.md` "Entry points" + "Configuration"). Whether the derived
`service:*` scope set includes `routes:write` depends on `grants.go`'s
`DeriveRules` for that principal (tier-0/operator service keys get broad
scopes per `ARCHITECTURE.md:566-573` "tier-0 `*`"). Full interactive OAuth
login (`GET /auth/*` on authd, Google/GitHub/Discord/Telegram widget) is
the user-facing alternative but is unnecessary machinery for a scripted
local client.

---

## 5. Concrete curl example

Assume routd reachable at `http://localhost:8080` (local/open auth mode,
`AUTHD_URL` unset) and webd at `http://localhost:8081` (adjust to your
actual `ROUTER_URL`/`WEBD_URL`/compose port mapping — both daemons default
to `:8080` internally per `webd/main.go:42-45`, so a bare local run needs
distinct host-port mappings for the two).

### (a) Mint a token, establishing a thread later via a client-chosen topic

```bash
curl -sS -X POST http://localhost:8080/v1/route_tokens/chat \
  -H 'Content-Type: application/json' \
  -d '{"owner_folder":"atlas"}'
# => {"token":"ab12cd34...","url":"http://localhost:8081/chat/ab12cd34.../","jid":"web:atlas","owner_folder":"atlas","created_at":"..."}
```
Capture `token` from the response, e.g. `TOKEN=ab12cd34...`.
Generate the thread id client-side: `TOPIC=trade-desk-$(uuidgen)`.

First message, JSON with wait (simplest single-shot: block up to 30s for
the assistant reply in the same response):
```bash
curl -sS -X POST "http://localhost:8081/chat/$TOKEN?wait=30" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json' \
  -d "{\"content\":\"what's BTC funding right now?\",\"topic\":\"$TOPIC\"}"
# => {"user":{...,"topic":"trade-desk-...","turn_id":"msg-..."},"turn_id":"msg-...","status":"pending","assistant":{"role":"assistant","content":"...","id":"..."}}
```

### (b) Receive the reply via SSE instead (streaming, `curl -N`)

```bash
curl -N -X POST "http://localhost:8081/chat/$TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'Accept: text/event-stream' \
  -d "{\"content\":\"what's BTC funding right now?\",\"topic\":\"$TOPIC\"}"
# streams:
# : ok
#
# event: message
# data: {"id":"msg-...","role":"user","content":"what's BTC funding right now?",...}
#
# event: message
# data: {"id":"msg-...","role":"assistant","content":"...",...}
```
(No `wait` param needed/used for the SSE path — the stream itself carries
the reply as it's produced.)

### (c) Second message, same thread — just resend the same `topic`

```bash
curl -sS -X POST "http://localhost:8081/chat/$TOKEN?wait=30" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json' \
  -d "{\"content\":\"and open interest trend over 24h?\",\"topic\":\"$TOPIC\"}"
```
Because `topic` (`trade-desk-...`) matches the first message's topic,
routd resolves the same `sessions` row (`group_folder="atlas"`,
`topic="trade-desk-..."`) and the agent continues with full context of the
prior turn — no other identifier needs to be tracked or resent.

Rehydrate history at any time (e.g. after a client restart) with:
```bash
curl -sS "http://localhost:8081/chat/$TOKEN/$TOPIC/messages"
```

---

## Files referenced

- `webd/server.go` (route registration), `webd/route_token.go` (POST/GET
  handlers, token lookup, message injection, widget HTML), `webd/turn.go`
  (per-turn snapshot/status/SSE), `webd/hub.go` (SSE broker + frame
  writer), `webd/README.md`, `webd/api.go`, `webd/channel.go`
- `routd/tokens_http.go`, `routd/tokens.go` (mint/list/revoke/resolve),
  `routd/server.go` (route table + `authz`), `routd/migrations/0001-initial-schema.sql`
  (`route_tokens`, `messages`, `sessions`, `groups` schema)
- `authd/README.md` (`/v1/tokens`, `/v1/service-token`, OAuth surface)
- `ARCHITECTURE.md` (Channel Protocol, three-planes model, SQLite schema
  table), `ROUTING.md` (topic/session model, `sessions` PK)
