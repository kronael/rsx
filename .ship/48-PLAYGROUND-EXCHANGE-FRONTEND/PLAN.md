# 48 — rsx-playground: dev dashboard to full exchange front-end

## Goal

Promote `rsx-playground` from an ops/demo dashboard into the complete usable
front-end to the RSX exchange: a user logs in (GitHub OAuth via `rsx-auth`),
manages users and authorization, and trades (live book/BBO, order entry,
cancel, positions, fills, per-symbol views) — driven by the founder, planned
by fable, implemented by sonnet subagents one chunk at a time.

**Domain lock:** `rsx-playground/` and `rsx-auth/` only. Never touch
`rsx-gateway`, `rsx-core`, `rsx-term`, `rsx-glass`, or Rust exchange crates —
those are owned by other agents/the founder. The dashboard is a CLIENT of
the exchange; it talks to Gateway/Marketdata over their existing WS/REST
APIs and never modifies the exchange.

## Direction conflict — flagged, not silently overridden

`TODO.md` "Direction (2026-07-03)" currently records: *"Trade surface = TUI,
not GUI... Playground keeps a web GUI — but read-only/less-interactive...
Demonstration surface, not a trading terminal. `/trade` no longer served."*
`rsx-playground/README.md` still says *"Not a production component.
Development/testing tool only."*

This plan directly reverses that decision at the founder's explicit,
detailed request today (2026-07-23). Proceeding on the instruction since
it is unambiguous and specific — but recording the reversal here and in
`TODO.md`/README rather than leaving the old decision silently
contradicted by new code. `rsx-term`/`rsx-tui` is unaffected and stays the
terminal-native surface; the playground becomes a second, browser-native
surface with its own login, not a replacement.

## What already exists (assessed 2026-07-23 — do not rebuild)

`rsx-playground/server.py` (9.1K lines) already carries most of the hard
plumbing for this:

- **Real JWT verification**, not a stub: `_extract_token_from_headers`
  (Bearer header or `rsx_token` cookie) → `_decode_user_token` (HS256,
  `aud=rsx-gateway`/`iss=rsx-auth`, same secret as gateway) →
  `_resolve_request_user` → `_require_private_user` (401 JSON if no valid
  user). Already gates `/v1/positions`, `/v1/fills`, `/v1/account`,
  `/v1/orders` (GET) and rejects a mismatched `user_id` query param with 403.
  Unit-tested in `tests/api_proxy_test.py`.
- **`/ws/private`** proxies the browser's real Bearer/cookie token straight
  to Gateway (Gateway does its own JWT verify + binds the WS to that user —
  defense stays at Gateway, per this repo's trust-boundary doctrine). A
  loopback-only, opt-in dev fallback (`PLAYGROUND_ALLOW_INSECURE_USER_ID`)
  mints a token for an `x-user-id` header — already logs a WARN, already
  gated to loopback, do not touch.
- **`/auth/*`, `/oauth/*`** already proxy to `rsx-auth` (`auth_proxy`).
  **`/v1/*`** already proxies unmatched paths to Gateway (`v1_proxy`,
  forwards `_request_auth_headers(request)` — reuse this, don't reinvent).
- **Order wire protocol is JSON, not protobuf.** Per
  `specs/2/49-webproto.md` header: the *order/event* channel is currently
  JSON text (`{"N":[sym,side,px,qty,cid,tif,ro,po]}` request,
  `{"U":[oid,status,filled,remaining,reason]}` /
  `{"F":[...]}` / `{"E":[...]}` response) — protobuf order channel is a
  roadmap item, only the public marketdata feed is protobuf today. This
  means order entry from the browser does **not** need a JS protobuf codec;
  it reuses the same JSON shape `send_order_to_gateway()` already speaks.
- **Two separate order-submission paths exist and must stay separate:**
  1. `/api/orders/test` + `/api/orders/{cid}/cancel` + friends — an
     **ops/admin harness**, gated by `_require_admin_request` (loopback or
     `PLAYGROUND_ADMIN_TOKEN`), `user_id` is a free-form field, and
     `send_order_to_gateway()` **mints its own JWT** for that `user_id`
     (fine — it's admin-only, impersonation is the point of a test harness).
  2. **Does not yet exist:** a per-session authenticated order path where
     the user_id comes only from the caller's own verified token. This is
     new work (Chunk 3 below) and must NOT mint a token for an arbitrary
     `user_id` — it must forward the caller's own resolved auth headers
     (`_request_auth_headers`, already written) so Gateway's own JWT check
     is the only source of truth for identity. Never let a logged-in
     browser session submit an order "as" a different user_id.
- **rsx-auth** (`rsx-auth/src/rsx_auth/`) is a real, tested GitHub OAuth →
  RSX-JWT service: `/oauth/github/login`, `/oauth/github/callback`,
  `/auth/me`, `/auth/logout`. Users table
  (`rsx-risk/migrations/003_users.sql`, **out of domain, do not touch**):
  `users(user_id, provider, provider_sub, email, login, created_at,
  last_login_at)`. **No roles/permissions concept exists today.**
- **No login UI, no Users tab, no Auth/roles tab, no Trade tab exist in
  `pages.py`.** `TABS` (pages.py:30) has 20 entries, none of them these.
  `/api/users/create` + `/api/users/{id}/deposit` exist (admin-gated) but
  have no listing page.
- The embedded **"Terminal" tab** (`terminal.py`/`terminal_page.py`) spawns
  a shared, loopback/admin-gated `rsx-term` PTY — not per-user, not part of
  this plan, do not touch (rsx-term is out of domain).

## Architecture decisions for the new work

1. **Roles live in `rsx-auth`, in a table `rsx-auth` owns.** The `users`
   table is owned by `rsx-risk`/exchange schema (out of domain — never
   `ALTER TABLE users`). Add a new table via a migration `rsx-auth` applies
   itself at pool-init (`rsx-auth/migrations/001_auth_roles.sql`, guarded
   by the same shared `migrations(id)` idempotency table the Rust side
   already uses, keyed distinctly e.g. `'auth_001_roles'`), e.g.
   `user_roles(user_id PRIMARY KEY, role TEXT NOT NULL DEFAULT 'trader')`.
   Bootstrap: `RSX_AUTH_ADMIN_LOGINS` env var (comma-separated GitHub
   logins) — on upsert, a login in that set gets `role='admin'`. Config via
   env only, per repo convention.
2. **Two distinct "admin" concepts stay distinct — do not merge them:**
   - `_require_admin_request` (loopback / `PLAYGROUND_ADMIN_TOKEN`) — infra
     control (start/stop processes, ops order harness). Unchanged.
   - New `role=admin` (rsx-auth JWT claim) — product-level authorization
     (who can see the Users tab, change another user's role). New, scoped
     to the new user-facing pages only.
3. **New authenticated order path reuses, not duplicates, the existing
   order-building code.** `/api/orders/test`'s form-parsing/validation/wire-
   building (symbol lookup, tick/lot alignment, market-sweep price, i64
   overflow check, `{"N":[...]}` construction) is ~150 lines that must be
   extracted into a shared helper called by both the admin harness (form
   `user_id`) and the new authenticated endpoint (session `user_id`) — per
   this repo's "amend the original, don't duplicate" rule.
4. **`send_order_to_gateway` gets a sibling, not a rewrite.** Keep the
   admin path's token-minting behavior; add a second function/parameter
   path that opens the upstream WS with headers from
   `_request_auth_headers(request)` instead of minting, so a real user's
   own token is what Gateway checks.
5. **Users-tab reads may use direct PG** (consistent with existing
   `/api/risk/overview`, `/api/users/create` precedent — playground already
   has PG access for dev-ops convenience) but **role writes go through a
   new `rsx-auth` admin endpoint** (`POST /admin/users/{id}/role`, itself
   gated by the new `role=admin` JWT claim), keeping the mutation on the
   service that owns the roles table.

## Steps

### Chunk 1 — rsx-auth: roles table + JWT/session claim
**Files:** `rsx-auth/migrations/001_auth_roles.sql` (new),
`rsx-auth/src/rsx_auth/db.py` (apply migration at pool-init, `get_role`,
`set_role`, `list_users`), `rsx-auth/src/rsx_auth/config.py`
(`RSX_AUTH_ADMIN_LOGINS`), `rsx-auth/src/rsx_auth/jwt_util.py` (`role`
claim), `rsx-auth/src/rsx_auth/app.py` (`/auth/me` returns role; new
`GET /admin/users`, `POST /admin/users/{id}/role`, both requiring
`role=admin` from the caller's own verified JWT), `rsx-auth/tests/*`.
**Gate:** `cd rsx-auth && uv run --extra dev pytest -v`

### Chunk 2 — rsx-playground: login/logout + session identity in the nav
**Files:** `server.py` (new `/x/session-chip` HTMX partial using
`_resolve_request_user` + a call to rsx-auth's `/auth/me`; wires
`/oauth/github/login?redirect=./` and `/auth/logout`), `pages.py` (nav
gains a login/logout control + identity chip; no schema/route churn to
existing tabs), `tests/api_*.py` + one `play_*.spec.ts` for the login
affordance rendering logged-out vs logged-in.
**Gate:** `make test` (rsx-playground unit) + the new Playwright spec.

### Chunk 3 — Trade tab: real per-user order entry/cancel + book/positions
**Files:** `server.py` (extract shared order-building helper per decision
#3; new `_require_private_user`-gated `POST /api/trade/orders`,
`POST /api/trade/orders/{cid}/cancel` per decision #4; new `/trade` route),
`pages.py` (`trade_page()`: book ladder via existing `/x/book`, order form,
positions/fills via existing `/v1/positions` `/v1/fills` `/v1/account`),
`TABS` (+"Trade"), tests: `tests/api_orders_test.py` additions (authenticated
path, rejects spoofed `user_id`), new `tests/play_trade.spec.ts`.
**Gate:** `make test && make lint`, then the new Playwright spec; full
`make gate` before calling this chunk done (highest-risk chunk — touches
the shared order-submission code path).

### Chunk 4 — Users & Authorization admin tab
**Files:** `server.py` (new `role=admin`-gated `/users` page route; proxies
list/role-change to the new rsx-auth `/admin/users*` endpoints; reuses
existing `/api/users/create` + `/api/users/{id}/deposit` for the create/
deposit actions already there), `pages.py` (`users_page()`: table of users +
role + balance, create-user form, role-change control), `TABS` (+"Users"),
`tests/api_*.py` + `play_users.spec.ts`.
**Gate:** rsx-auth pytest (if Chunk 1 needs any follow-on) + `make test &&
make e2e`.

### Chunk 5 — Docs reconciliation (small, may be folded into Chunk 2-4 by
whichever sonnet touches that area rather than run standalone)
Flip `TODO.md` "Direction (2026-07-03)" entry to record 2026-07-23's
reversal; update `rsx-playground/README.md` ("Not a production component"
language, Tabs list, Quick Start); `docs/tabs.md` + `docs/api.md` new
entries; `.diary/` entry per `/diary` convention.

## Acceptance

- `make gate` green after Chunk 3 and again after Chunk 4.
- A user can: click "log in", complete GitHub OAuth, land back on the
  dashboard authenticated; place and cancel an order under their own
  identity (never able to spoof another `user_id`); see their own
  positions/fills/account; an admin-role user can list users, create one,
  deposit, and change another user's role; a non-admin cannot reach
  `/users`.
- No new duplicate order-building/JWT-minting/proxy logic — everything
  reuses the helpers listed in "Architecture decisions."
- Every new route has an `api_*.py` or `play_*.spec.ts` test.

## Out of scope

- `rsx-gateway`, `rsx-term`, protobuf order channel (roadmap, spec says so).
- Fine-grained per-action permission scopes beyond `trader`/`admin`.
- Multi-provider OAuth (rsx-auth already designed for it, not needed now).
- Fixing `PLAYGROUND-MARKET-ORDER-REJECTED-BY-GATEWAY` (open bug, logged
  in `BUGS.md`, not this plan's job per Bug Triage Protocol).
