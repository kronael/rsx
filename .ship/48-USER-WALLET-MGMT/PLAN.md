# User Management + Wallet Management — tui + glass

Native-only management surface (spec `specs/2/7-dashboard.md`). Never web.

## Scope correction (mid-task, from the founder)

Original brief assumed rsx-auth (or the playground) could serve
accounts/positions/fills/funding/freezes reads + support writes. Corrected:
that data lives in Postgres written by `rsx-risk/src/persist.rs`; the
exchange's one API surface for it is the **gateway** (`/v1/api/support/*`,
spec 7 §3). The gateway is mid-rewrite by another agent — **no edits here**.

Split, final:

- **rsx-auth** — identity + RBAC only. JWT carries a `role` claim.
  Finish the partial `config.py`/`db.py` work already on disk.
- **gateway** (not built here) — accounts/positions/fills/funding/freezes
  reads + adjust-collateral/freeze/note writes, per spec 7. This session
  designs the Go client against that contract and **stubs/fixtures** it;
  the exact endpoint list goes back to the founder in the report.
- **rsx-term / rsx-glass** — frontends, gateway-independent, built now.

## 1. rsx-auth: finish RBAC (real, tested end-to-end)

- Fix `VALID_ROLES`/`DEFAULT_ROLE` in `db.py`: the on-disk `("trader",
  "admin")` pair is the wrong axis (that's a product/trading role, not
  spec 7 §4's management RBAC). Replace with spec 7's 4 roles: `viewer`,
  `operator`, `admin`, `auditor`. No management role by default (a normal
  trader gets none of the four — deny-by-default) → `DEFAULT_ROLE = None`
  sentinel (`get_role` returns `None`, not a fabricated tier).
- New `rbac.py`: the §4 permission matrix as data
  (`ROLE_PERMISSIONS: dict[str, set[str]]`) + `can(role, action) -> bool`,
  deny-by-default for unknown role/action. Unit-testable with no DB.
- Wire `config.admin_logins` into `github_callback`: a login in the
  allow-list is upserted to `admin` on every login (self-healing bootstrap,
  matches "trusted admin list" semantics — a login removed from the env
  var stops being re-asserted, but an existing DB row isn't auto-revoked;
  that revocation is a manual `set_role` action, logged as a known
  boundary, not silently overridden by env-var absence which could be a
  deploy accident).
- `jwt_util.issue` gains a `role` param → embedded claim (what the
  correction calls "JWT role claim"); `github_callback` looks up the
  fresh role before minting.
- New endpoints, prefixed `/roles` (rsx-auth's own domain — deliberately
  NOT `/v1/api/support`, which is the gateway's future prefix):
  - `GET /roles/users?query=&limit=&cursor=` — identity+role directory
    (extends existing `list_users`)
  - `GET /roles/users/{user_id}` — one user + role
  - `POST /roles/users/{user_id}/role` `{role, reason, ticket_id}` —
    admin-only (`can(caller_role, "manage_roles")`), audited
  - `GET /roles/audit?actor=&target=&limit=&cursor=` — role-change trail
  - `GET /roles/me` — caller's own identity + role (frontends use this to
    decide which actions to show)
- New `audit_log` table (rsx-auth-owned, migration
  `002_roles_audit.sql`): actor, action, target, before/after json, reason,
  ticket_id, request_id, idempotency_key, ts. Role-write is audited in the
  same transaction (spec 7 §6 "atomic").
- Tests (`tests/rbac_test.py`, extend `tests/app_test.py`): matrix unit
  tests (no DB) + endpoint tests monkeypatching `db.*` (mirrors the
  existing `github` monkeypatch pattern — DATABASE_URL="" in tests, no
  Postgres needed, consistent with the current suite).

## 2. rsx-term: Users/Wallets screen

- Bolt onto the existing streaming multi-screen rotation (`screenBook →
  screenNews → screenLLM`, `model.go`/`stream.go`/`keymap.go`) as a 4th
  screen `screenUsers` — the only place in the terminal with a tab-cycled
  multi-screen architecture; classic DOM mode is unaffected (`cfg.Stream`
  off ⇒ byte-identical, per the crate's keeper invariant).
- `conn/support.go`: two clients behind small interfaces —
  `RolesClient` (real HTTP, hits rsx-auth's new `/roles/*`, JWT bearer —
  the token is an already-issued rsx-auth session token read from
  `RSX_TERM_SUPPORT_TOKEN`, never self-minted with an asserted role: the
  terminal must not invent its own privilege) and `AccountClient` (the
  gateway `/v1/api/support/*` contract — **stub/fixture only**, clearly
  TODO-marked, swapped for a real HTTP impl once the gateway ships it).
  `cfg.Support == nil` ⇒ offline placeholder screen, mirroring `news.Off`
  / `Assist == nil`.
- List → detail → action, k9s-flat-keys style: `j/k` select, `enter`
  drill into detail (account/positions/fills/funding/freezes/role), `esc`
  back, `/` search. Actions (role-gated, hidden/blocked with an honest
  reason when the caller's role can't): `o` set role, `a` adjust
  collateral (single-line `<delta> <ticket> <reason…>`), `z` freeze
  upsert/delete, `n` add note.
- Goldens: `dom_golden_test.go`/`stream_golden_test.go` pattern — new
  `TestUsersScreenGolden` fixture-driven (no network).

## 3. rsx-glass: read-only accounts overview

- One more `Grid`-producing panel (table, not heatmap cells) fed by the
  same stub/fixture `AccountClient` shape as rsx-term — mirrors, doesn't
  duplicate, the rsx-term visual language per crate CLAUDE.md. No CRUD.
- Golden PNG under `soft/testdata/`.

## Guardrails re-affirmed

No gateway edits. No rsx-risk/rsx-matching/rsx-book/rsx-cast edits. Not
web. Fail loud. `gofmt`/`go vet`/`go test ./...` + goldens on rsx-term/
rsx-glass; `uv run --extra dev pytest` on rsx-auth. Stubbed pieces are
flagged plainly in the report, not silently implied "done".
