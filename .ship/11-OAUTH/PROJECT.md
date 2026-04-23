# PROJECT.md — OAuth (GitHub) + User Identity

## Goal

Production-ready user identity via GitHub OAuth. New Python
service `rsx-auth/` handles the OAuth dance, creates users,
and issues JWTs validated by rsx-gateway. No passwords to
store, no email verification, no MFA — GitHub handles all
of it.

## Non-goals

- Password-based auth (delegate to GitHub)
- Email verification flow (GitHub verifies)
- Password reset (GitHub's problem)
- MFA (GitHub's MFA suffices for v1)
- Multi-provider (Google, Twitter, etc.) — later, trivial to add

## IO Surfaces

- **rsx-auth** — new Python FastAPI service on :8082
- **users table** — new Postgres table, FK from accounts.user_id
- **rsx-gateway** — validates JWTs from rsx-auth (existing logic)
- **rsx-webui** — adds "Sign in with GitHub" button

## Architecture

```
  trade UI                rsx-auth              GitHub
    |                       |                     |
    |-- click "Sign in" --->|                     |
    |                       |-- redirect to GH -->|
    |                       |                     |-- user auths
    |                       |                     |
    |                       |<-- code + state ----|
    |                       |-- exchange code --->|
    |                       |<-- access_token ----|
    |                       |-- fetch user -------|
    |                       |<-- user info -------|
    |                       |-- upsert users row
    |                       |-- issue JWT
    |<-- redirect w/ JWT ---|
    |
    |-- Bearer JWT ------> rsx-gateway (REST + WS)
```

## Users table (new migration)

`rsx-risk/migrations/003_users.sql`:

```sql
CREATE TABLE users (
    user_id        SERIAL PRIMARY KEY,
    provider       TEXT NOT NULL,         -- 'github', 'google', etc.
    provider_sub   TEXT NOT NULL,         -- external stable ID
    email          TEXT,
    login          TEXT,                  -- external handle
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_login_at  TIMESTAMPTZ,
    UNIQUE (provider, provider_sub)
);
-- accounts.user_id already int; FK added via migration
ALTER TABLE accounts ADD CONSTRAINT fk_accounts_user
    FOREIGN KEY (user_id) REFERENCES users(user_id);
```

## Tasks

### 1. GitHub OAuth app registration
User-side: register app on github.com/settings/developers,
set redirect URI. **User will handle this** — we supply the
redirect URI pattern.

### 2. rsx-auth scaffolding
Create `rsx-auth/` dir: FastAPI app, pyproject.toml,
Dockerfile. ~200-300 lines total.

### 3. Users migration + rsx-risk integration
003_users.sql. Update `seed_accounts()` to also seed a
`users` row for dev fixtures [1..5, 99] with a dummy
github_sub.

### 4. OAuth endpoints
- `GET /oauth/github/login` → 302 to GH authorize URL (with state cookie)
- `GET /oauth/github/callback?code=...&state=...` → token exchange + user fetch + upsert + JWT + 302 to trade UI
- `GET /auth/me` → validate JWT, return user info (email, login, user_id)
- `POST /auth/logout` → client-side only (clear cookie/token); no server state

### 5. JWT issuance
pyjwt HS256 with `RSX_GW_JWT_SECRET`. Claims:
`{ "sub": provider_sub, "user_id": numeric, "email": ..., "exp": now + 7 days, "iat": now }`.

### 6. Frontend "Sign in with GitHub" button
- rsx-webui TopBar: show login button when no JWT in
  localStorage. Click → redirect to rsx-auth /oauth/github/login.
- Callback lands on trade UI with `?token=...` (or
  cookie); store, reload.
- Logout button clears token.

### 7. Starter collateral on first login
Configurable via `RSX_AUTH_STARTER_COLLATERAL`. For demo:
10 BTC-equivalent. For mainnet: 0 (require deposit).

### 8. Tests
- Unit: JWT issuance + verification
- Integration (testcontainers): OAuth callback mocks GitHub, verifies user/account creation
- Playwright: end-to-end "sign in" flow (mock GH in test mode)

### 9. Deployment config
- `rsx-auth` runs as separate process; add to start script
- Env vars: `RSX_AUTH_LISTEN`, `RSX_AUTH_GITHUB_CLIENT_ID`,
  `RSX_AUTH_GITHUB_CLIENT_SECRET`, `RSX_AUTH_REDIRECT_URI`,
  `RSX_AUTH_STARTER_COLLATERAL`, `DATABASE_URL`
- Secrets via env / secret manager; never committed

### 10. Docs
New `specs/2/50-auth.md` documenting the auth model,
provider flow, JWT claims, extension path to more providers.

## Acceptance

- User clicks "Sign in with GitHub" → lands back on trade
  UI authenticated, can place orders
- `users` table has a row per distinct GitHub user
- accounts.user_id for auth'd user is stable across
  logins
- JWT issued by rsx-auth is accepted by rsx-gateway
  (REST + WS) without modification
- Playwright e2e sign-in test passes (mocking GitHub in CI)

## Out of scope / follow-up

- Google / Twitter / wallet providers
- Admin UI for banning users
- Account deletion / GDPR export
- Rate-limiting OAuth endpoints (spam protection)
