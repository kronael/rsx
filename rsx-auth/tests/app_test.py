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
from rsx_auth import db, jwt_util, rbac  # noqa: E402


def _bearer(user_id: int = 1) -> dict:
    token = jwt_util.issue(
        user_id=user_id,
        provider="github",
        provider_sub="sub",
        email=None,
        secret=os.environ["RSX_GW_JWT_SECRET"],
        ttl_s=3600,
    )
    return {"authorization": f"Bearer {token}"}


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


def test_jwt_issued_carries_role_claim():
    token = jwt_util.issue(
        user_id=99, provider="github", provider_sub="xyz", email=None,
        secret=os.environ["RSX_GW_JWT_SECRET"], ttl_s=3600,
        role=rbac.ADMIN,
    )
    claims = jwt_util.verify(token, os.environ["RSX_GW_JWT_SECRET"])
    assert claims["role"] == "admin"


# ------------------------------ /roles/* ------------------------------
# rsx-auth's own role-lookup is re-checked from the DB on every request
# (see app.caller_role) rather than trusting the bearer token's `role`
# claim — so these tests monkeypatch db.get_role per case instead of
# minting tokens with different embedded roles.

def test_roles_me_requires_bearer(client):
    r = client.get("/roles/me")
    assert r.status_code == 401


def test_roles_me_returns_live_role(client, monkeypatch):
    async def fake_get_role(user_id):
        assert user_id == 1
        return rbac.ADMIN
    monkeypatch.setattr(db, "get_role", fake_get_role)
    r = client.get("/roles/me", headers=_bearer(1))
    assert r.status_code == 200
    assert r.json() == {"user_id": 1, "role": "admin"}


def test_roles_users_denies_no_role(client, monkeypatch):
    async def fake_get_role(user_id):
        return None
    monkeypatch.setattr(db, "get_role", fake_get_role)
    r = client.get("/roles/users", headers=_bearer(1))
    assert r.status_code == 403


def test_roles_users_allows_viewer(client, monkeypatch):
    async def fake_get_role(user_id):
        return rbac.VIEWER

    async def fake_list_users(query=None, limit=50, cursor=0):
        return [{
            "user_id": 7, "provider": "github", "email": "a@b.com",
            "login": "alice", "created_at": None, "last_login_at": None,
            "role": None,
        }]
    monkeypatch.setattr(db, "get_role", fake_get_role)
    monkeypatch.setattr(db, "list_users", fake_list_users)
    r = client.get("/roles/users", headers=_bearer(1))
    assert r.status_code == 200
    assert r.json()["users"][0]["login"] == "alice"


def test_roles_set_role_denies_viewer(client, monkeypatch):
    async def fake_get_role(user_id):
        return rbac.VIEWER
    monkeypatch.setattr(db, "get_role", fake_get_role)
    r = client.post(
        "/roles/users/7/role",
        json={"role": "operator", "reason": "promo", "ticket_id": "T-1"},
        headers={**_bearer(1), "idempotency-key": "k1"},
    )
    assert r.status_code == 403


def test_roles_set_role_requires_idempotency_key(client, monkeypatch):
    async def fake_get_role(user_id):
        return rbac.ADMIN
    monkeypatch.setattr(db, "get_role", fake_get_role)
    r = client.post(
        "/roles/users/7/role",
        json={"role": "operator", "reason": "promo", "ticket_id": "T-1"},
        headers=_bearer(1),
    )
    assert r.status_code == 400


def test_roles_set_role_requires_reason_and_ticket(client, monkeypatch):
    async def fake_get_role(user_id):
        return rbac.ADMIN
    monkeypatch.setattr(db, "get_role", fake_get_role)
    r = client.post(
        "/roles/users/7/role",
        json={"role": "operator", "reason": "  ", "ticket_id": "T-1"},
        headers={**_bearer(1), "idempotency-key": "k1"},
    )
    assert r.status_code == 400


def test_roles_set_role_rejects_invalid_role(client, monkeypatch):
    async def fake_get_role(user_id):
        return rbac.ADMIN
    monkeypatch.setattr(db, "get_role", fake_get_role)
    r = client.post(
        "/roles/users/7/role",
        json={"role": "superuser", "reason": "x", "ticket_id": "T-1"},
        headers={**_bearer(1), "idempotency-key": "k1"},
    )
    assert r.status_code == 400


def test_roles_set_role_success(client, monkeypatch):
    async def fake_get_role(user_id):
        return rbac.ADMIN
    calls = {}

    async def fake_set_role_audited(**kwargs):
        calls.update(kwargs)
        return kwargs["role"]
    monkeypatch.setattr(db, "get_role", fake_get_role)
    monkeypatch.setattr(db, "set_role_audited", fake_set_role_audited)
    r = client.post(
        "/roles/users/7/role",
        json={"role": "operator", "reason": "promo", "ticket_id": "T-1"},
        headers={**_bearer(1), "idempotency-key": "k1",
                 "x-request-id": "req-1"},
    )
    assert r.status_code == 200
    assert r.json() == {"user_id": 7, "role": "operator"}
    assert calls["actor"] == 1
    assert calls["target_user_id"] == 7
    assert calls["role"] == "operator"
    assert calls["reason"] == "promo"
    assert calls["ticket_id"] == "T-1"
    assert calls["idempotency_key"] == "k1"
    assert calls["request_id"] == "req-1"


def test_roles_audit_denies_operator(client, monkeypatch):
    async def fake_get_role(user_id):
        return rbac.OPERATOR
    monkeypatch.setattr(db, "get_role", fake_get_role)
    r = client.get("/roles/audit", headers=_bearer(1))
    assert r.status_code == 403


def test_roles_audit_allows_auditor(client, monkeypatch):
    async def fake_get_role(user_id):
        return rbac.AUDITOR

    async def fake_list_audit(actor=None, target=None, limit=50, cursor=0):
        return []
    monkeypatch.setattr(db, "get_role", fake_get_role)
    monkeypatch.setattr(db, "list_audit", fake_list_audit)
    r = client.get("/roles/audit", headers=_bearer(1))
    assert r.status_code == 200
    assert r.json() == {"audit": []}
