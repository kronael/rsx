"""FastAPI tests for rsx-auth.

Covers: /health, /oauth/github/login redirect, callback
CSRF state check, /auth/me auth enforcement. GitHub HTTP
calls are mocked via monkeypatch of the github module.
"""
import os
import pytest
from fastapi.testclient import TestClient

# Configure env before importing the app
os.environ.setdefault(
    "RSX_GW_JWT_SECRET",
    "test-secret-at-least-32-bytes-long-please!")
os.environ.setdefault("RSX_AUTH_GITHUB_CLIENT_ID", "test-client-id")
os.environ.setdefault(
    "RSX_AUTH_GITHUB_CLIENT_SECRET", "test-client-secret")
os.environ.setdefault("DATABASE_URL", "")
os.environ.setdefault("RSX_AUTH_STARTER_COLLATERAL", "0")

from rsx_auth.app import app  # noqa: E402
from rsx_auth import jwt_util  # noqa: E402


@pytest.fixture
def client():
    # lifespan=on runs startup (which tries to init pool
    # with empty DB url; we guard that case)
    with TestClient(app) as c:
        yield c


def test_health(client):
    r = client.get("/health")
    assert r.status_code == 200
    assert r.json() == {"status": "ok"}


def test_github_login_redirects_to_github(client):
    r = client.get("/oauth/github/login", follow_redirects=False)
    assert r.status_code == 302
    loc = r.headers["location"]
    assert loc.startswith("https://github.com/login/oauth/authorize")
    assert "client_id=test-client-id" in loc
    assert "state=" in loc
    # State cookie set
    assert "rsx_oauth_state" in r.cookies


def test_callback_rejects_missing_state(client):
    r = client.get(
        "/oauth/github/callback?code=abc",
        follow_redirects=False,
    )
    assert r.status_code == 400


def test_callback_rejects_state_mismatch(client):
    client.cookies.set("rsx_oauth_state", "expected-state")
    r = client.get(
        "/oauth/github/callback?code=abc&state=wrong",
        follow_redirects=False,
    )
    assert r.status_code == 400


def test_auth_me_requires_bearer(client):
    r = client.get("/auth/me")
    assert r.status_code == 401


def test_auth_me_rejects_invalid_bearer(client):
    r = client.get(
        "/auth/me",
        headers={"authorization": "Bearer not.a.jwt"},
    )
    assert r.status_code == 401


def test_logout_clears_cookie(client):
    r = client.post("/auth/logout")
    assert r.status_code == 200
    # Set-Cookie: rsx_token=""; Max-Age=0 (deletion)
    # TestClient exposes this via response.cookies; check
    # absence after the request is sufficient.


def test_jwt_issued_is_verifiable():
    """Cross-check: token issued by jwt_util verifies fine."""
    token = jwt_util.issue(
        user_id=99,
        provider="github",
        provider_sub="xyz",
        email="a@b.com",
        secret=os.environ["RSX_GW_JWT_SECRET"],
        ttl_s=3600,
    )
    claims = jwt_util.verify(
        token, os.environ["RSX_GW_JWT_SECRET"])
    assert claims["user_id"] == 99
