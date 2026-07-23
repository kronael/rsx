"""rsx-auth FastAPI app: GitHub OAuth → RSX JWT."""
import logging
import secrets
from contextlib import asynccontextmanager

from fastapi import Depends, FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse, RedirectResponse
from pydantic import BaseModel
import jwt as pyjwt

from . import db, github, jwt_util, rbac
from .config import Config

logger = logging.getLogger("rsx-auth")
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s %(levelname)s %(name)s: %(message)s",
)

_config: Config | None = None


@asynccontextmanager
async def lifespan(app: FastAPI):
    global _config
    _config = Config.from_env()
    missing = _config.validate()
    if missing:
        logger.warning(
            "missing config (service will 500 on oauth flows): %s",
            ", ".join(missing))
    if _config.database_url:
        await db.init_pool(_config.database_url)
        logger.info("postgres pool ready")
    yield
    await db.close_pool()


app = FastAPI(
    title="rsx-auth",
    version="0.1.0",
    lifespan=lifespan,
)


def cfg() -> Config:
    assert _config is not None
    return _config


@app.get("/health")
async def health():
    return {"status": "ok"}


@app.get("/oauth/github/login")
async def github_login(redirect: str | None = None):
    """Start OAuth flow: redirect to GitHub authorize URL.

    Optional ?redirect=<trade-ui-url> lets the frontend
    tell us where to bounce the user back to after login.
    """
    c = cfg()
    if not c.github_client_id:
        raise HTTPException(500, "github client id not configured")
    # State carries optional post-login redirect
    state_token = secrets.token_urlsafe(32)
    # For a stateless design, encode redirect in state itself
    # by prefixing: `<token>:<b64(redirect)>`. Simpler: just
    # rely on the token. Store-backed sessions are future.
    url = github.authorize_url(
        c.github_client_id, c.redirect_uri, state_token)
    resp = RedirectResponse(url, status_code=302)
    resp.set_cookie(
        "rsx_oauth_state", state_token,
        max_age=600, httponly=True, samesite="lax",
    )
    if redirect:
        resp.set_cookie(
            "rsx_oauth_redirect", redirect,
            max_age=600, httponly=False, samesite="lax",
        )
    return resp


@app.get("/oauth/github/callback")
async def github_callback(
    request: Request,
    code: str | None = None,
    state: str | None = None,
    error: str | None = None,
):
    """Handle GitHub redirect: exchange code, issue JWT."""
    c = cfg()
    if error:
        return JSONResponse(
            {"error": f"github: {error}"}, status_code=400)
    if not code or not state:
        return JSONResponse(
            {"error": "missing code or state"}, status_code=400)

    # CSRF check: state must match cookie
    expected_state = request.cookies.get("rsx_oauth_state")
    if not expected_state or state != expected_state:
        return JSONResponse(
            {"error": "state mismatch"}, status_code=400)

    # Exchange code for GitHub token
    try:
        gh_token = await github.exchange_code(
            c.github_client_id,
            c.github_client_secret,
            code,
            c.redirect_uri,
        )
    except Exception as e:
        logger.warning("code exchange failed: %s", e)
        return JSONResponse(
            {"error": "code exchange failed"}, status_code=400)

    # Fetch GitHub user
    try:
        gh_user = await github.fetch_user(gh_token)
    except Exception as e:
        logger.warning("user fetch failed: %s", e)
        return JSONResponse(
            {"error": "user fetch failed"}, status_code=502)

    # Upsert user + seed account
    user_id = await db.upsert_user(
        provider="github",
        provider_sub=gh_user["sub"],
        email=gh_user.get("email"),
        login=gh_user.get("login"),
    )
    if c.starter_collateral > 0:
        await db.seed_account_if_missing(
            user_id, c.starter_collateral)

    # Bootstrap the trusted-admin-login list into a management role,
    # then embed the fresh role in the token (rbac.ROLES, or None for a
    # regular trader).
    role = await db.bootstrap_role_from_admin_login(
        user_id, gh_user.get("login"), c.admin_logins)

    # Issue RSX JWT
    token = jwt_util.issue(
        user_id=user_id,
        provider="github",
        provider_sub=gh_user["sub"],
        email=gh_user.get("email"),
        secret=c.jwt_secret,
        ttl_s=c.jwt_ttl_s,
        role=role,
    )

    # Redirect to trade UI with token in fragment (so it
    # isn't sent to server logs) or cookie. Use cookie
    # for simplicity: HttpOnly=false so JS can read +
    # put in Authorization header on subsequent calls.
    target = request.cookies.get("rsx_oauth_redirect") or c.trade_ui_url
    resp = RedirectResponse(target, status_code=302)
    resp.set_cookie(
        "rsx_token", token,
        max_age=c.jwt_ttl_s,
        httponly=False, samesite="lax",
    )
    resp.delete_cookie("rsx_oauth_state")
    resp.delete_cookie("rsx_oauth_redirect")
    return resp


def verify_bearer(request: Request) -> dict:
    """Extract + validate Bearer JWT. Returns claims dict."""
    auth = request.headers.get("authorization", "")
    if not auth.startswith("Bearer "):
        raise HTTPException(401, "missing bearer token")
    token = auth[7:].strip()
    try:
        return jwt_util.verify(token, cfg().jwt_secret)
    except pyjwt.InvalidTokenError as e:
        raise HTTPException(401, f"invalid token: {e}")


@app.get("/auth/me")
async def auth_me(claims: dict = Depends(verify_bearer)):
    """Return current user info. JWT required."""
    uid = int(claims["user_id"])
    user = await db.get_user(uid)
    if not user:
        raise HTTPException(404, "user not found")
    # Don't leak provider_sub in response body beyond sub
    return {
        "user_id": user["user_id"],
        "provider": user["provider"],
        "email": user.get("email"),
        "login": user.get("login"),
        "role": await db.get_role(uid),
        "created_at": user["created_at"].isoformat()
            if user.get("created_at") else None,
    }


@app.post("/auth/logout")
async def logout():
    """Client-side: token cookie cleared. Server is stateless."""
    resp = JSONResponse({"ok": True})
    resp.delete_cookie("rsx_token")
    return resp


# ---------------------------------------------------------------------
# RBAC / user-management (spec 7 dashboard, §4). rsx-auth owns identity
# + role assignment only; accounts/positions/fills/funding/freezes and
# their support-write actions live on the gateway (out of scope here —
# see .ship/48-USER-WALLET-MGMT/PLAN.md).
# ---------------------------------------------------------------------

async def caller_role(claims: dict = Depends(verify_bearer)) -> str | None:
    """Fresh DB role lookup for the calling user. Never trust the JWT's
    own embedded `role` claim for rsx-auth's OWN gating — a long-lived
    token can outlive a role change; the claim exists for a party with
    no DB access of its own (the gateway)."""
    return await db.get_role(int(claims["user_id"]))


def require(action: str):
    """Dependency factory: 403s unless the caller's live role can
    perform `action` (rbac.can — deny by default)."""
    async def dep(role: str | None = Depends(caller_role)) -> str | None:
        if not rbac.can(role, action):
            raise HTTPException(
                403, f"role {role!r} cannot perform {action!r}")
        return role
    return dep


def _serialize_user_row(row: dict) -> dict:
    return {
        "user_id": row["user_id"],
        "provider": row.get("provider"),
        "email": row.get("email"),
        "login": row.get("login"),
        "role": row.get("role"),
        "created_at": row["created_at"].isoformat()
            if row.get("created_at") else None,
        "last_login_at": row["last_login_at"].isoformat()
            if row.get("last_login_at") else None,
    }


@app.get("/roles/me")
async def roles_me(claims: dict = Depends(verify_bearer)):
    """Caller's own identity + role — no permission gate (self-service);
    frontends use this to decide which actions to even offer."""
    uid = int(claims["user_id"])
    return {"user_id": uid, "role": await db.get_role(uid)}


@app.get("/roles/users")
async def roles_users(
    query: str | None = None,
    limit: int = 50,
    cursor: int = 0,
    _role: str | None = Depends(require("read_user_state")),
):
    users = await db.list_users(query=query, limit=limit, cursor=cursor)
    return {"users": [_serialize_user_row(u) for u in users]}


@app.get("/roles/users/{user_id}")
async def roles_user_detail(
    user_id: int,
    _role: str | None = Depends(require("read_user_state")),
):
    user = await db.get_user(user_id)
    if not user:
        raise HTTPException(404, "user not found")
    return _serialize_user_row({**user, "role": await db.get_role(user_id)})


class SetRoleReq(BaseModel):
    role: str
    reason: str
    ticket_id: str


@app.post("/roles/users/{user_id}/role")
async def roles_set_role(
    user_id: int,
    req: SetRoleReq,
    request: Request,
    claims: dict = Depends(verify_bearer),
    _role: str | None = Depends(require("manage_roles")),
):
    if req.role not in rbac.ROLES:
        raise HTTPException(
            400, f"invalid role {req.role!r}; must be one of {rbac.ROLES}")
    if not req.reason.strip() or not req.ticket_id.strip():
        raise HTTPException(400, "reason and ticket_id are required")
    idempotency_key = request.headers.get("idempotency-key")
    if not idempotency_key:
        raise HTTPException(400, "Idempotency-Key header is required")
    request_id = request.headers.get("x-request-id") or secrets.token_hex(8)
    after = await db.set_role_audited(
        actor=int(claims["user_id"]),
        target_user_id=user_id,
        role=req.role,
        reason=req.reason,
        ticket_id=req.ticket_id,
        request_id=request_id,
        idempotency_key=idempotency_key,
    )
    return {"user_id": user_id, "role": after}


@app.get("/roles/audit")
async def roles_audit(
    actor: int | None = None,
    target: str | None = None,
    limit: int = 50,
    cursor: int = 0,
    _role: str | None = Depends(require("read_audit")),
):
    rows = await db.list_audit(
        actor=actor, target=target, limit=limit, cursor=cursor)
    return {
        "audit": [
            {**r, "ts": r["ts"].isoformat() if r.get("ts") else None}
            for r in rows
        ]
    }


def main():
    import uvicorn
    c = Config.from_env()
    host, port = c.listen.rsplit(":", 1)
    uvicorn.run(
        "rsx_auth.app:app",
        host=host, port=int(port),
        log_level="info",
    )


if __name__ == "__main__":
    main()
