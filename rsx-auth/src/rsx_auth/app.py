"""rsx-auth FastAPI app: GitHub OAuth → RSX JWT."""
import logging
import secrets
from contextlib import asynccontextmanager

from fastapi import Depends, FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse, RedirectResponse
import jwt as pyjwt

from . import db, github, jwt_util
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

    # Issue RSX JWT
    token = jwt_util.issue(
        user_id=user_id,
        provider="github",
        provider_sub=gh_user["sub"],
        email=gh_user.get("email"),
        secret=c.jwt_secret,
        ttl_s=c.jwt_ttl_s,
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
        "created_at": user["created_at"].isoformat()
            if user.get("created_at") else None,
    }


@app.post("/auth/logout")
async def logout():
    """Client-side: token cookie cleared. Server is stateless."""
    resp = JSONResponse({"ok": True})
    resp.delete_cookie("rsx_token")
    return resp


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
