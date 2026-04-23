# rsx-auth

OAuth identity service for RSX. Delegates auth to GitHub
(and future providers); issues RSX JWTs validated by
rsx-gateway. No passwords, no email verification — GitHub
handles identity.

## Endpoints

| Route | Purpose |
|-------|---------|
| `GET /health` | liveness |
| `GET /oauth/github/login?redirect=<url>` | 302 to GitHub authorize URL |
| `GET /oauth/github/callback?code=&state=` | OAuth callback; issues JWT; redirects to trade UI |
| `GET /auth/me` | current user info (Bearer required) |
| `POST /auth/logout` | clears `rsx_token` cookie |

## JWT claims

```json
{
  "sub":     "github:<id>",
  "user_id": <int>,
  "email":   <str or null>,
  "iat":     <epoch>,
  "exp":     <epoch>
}
```

Signed with `RSX_GW_JWT_SECRET` (HS256). Same secret as
rsx-gateway WS auth.

## Users table

`rsx-risk/migrations/003_users.sql`:

```sql
users (user_id, provider, provider_sub, email, login,
       created_at, last_login_at)
```

`user_id` is the numeric ID used throughout RSX
(`accounts.user_id` references it).

## Config (env)

| Var | Default | Notes |
|-----|---------|-------|
| `RSX_AUTH_LISTEN` | `0.0.0.0:8082` | listen addr |
| `RSX_GW_JWT_SECRET` | (required) | shared with gateway |
| `RSX_AUTH_JWT_TTL_S` | `604800` | 7 days |
| `RSX_AUTH_GITHUB_CLIENT_ID` | (required) | from github.com/settings/developers |
| `RSX_AUTH_GITHUB_CLIENT_SECRET` | (required) | same |
| `RSX_AUTH_REDIRECT_URI` | `http://localhost:8082/oauth/github/callback` | register on GitHub app |
| `RSX_AUTH_STARTER_COLLATERAL` | `0` | seed accounts row on first login |
| `RSX_AUTH_TRADE_UI_URL` | `http://localhost:5173/trade` | redirect target after callback |
| `DATABASE_URL` | (required) | Postgres conn string |

## Run

```bash
uv sync
uv run python -m rsx_auth.app
# or: uvicorn rsx_auth.app:app --host 0.0.0.0 --port 8082
```

## Tests

```bash
uv run --extra dev pytest -v
```

Unit tests for JWT + OAuth endpoints (GitHub HTTP mocked).

## GitHub OAuth app setup

1. https://github.com/settings/developers → New OAuth App
2. Homepage URL: your trade UI URL
3. Authorization callback URL: `<rsx-auth-host>/oauth/github/callback`
4. Copy Client ID + Client Secret to env vars

## Extending to more providers

Add provider module (e.g. `google.py`) and endpoint handlers.
`users.provider` column already supports multiple providers
via `(provider, provider_sub)` unique constraint.
