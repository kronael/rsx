"""Users tab (/users) and the /roles/* proxy extension.

/users is gated on the CALLER's own live role claim (_caller_role,
re-checked against rsx-auth on every load), distinct from the
loopback/PLAYGROUND_ADMIN_TOKEN gate the ops harness uses. Role
reads/writes proxy through to rsx-auth's own RBAC-gated /roles/*
endpoints -- playground never decides who may change a role itself.

Run with: cd rsx-playground && uv run pytest tests/api_users_test.py -v
"""

from unittest.mock import patch

import server


def test_users_page_requires_auth(client):
    """No auth header/cookie -> 403, never renders the table."""
    resp = client.get("/users")
    assert resp.status_code == 403
    assert "admin access required" in resp.text.lower()


def test_users_page_rejects_non_admin_role(client, auth_headers, monkeypatch):
    """A caller with SOME role but not admin (e.g. viewer/operator)
    still cannot reach /users -- the plan's acceptance criterion is a
    strict role=='admin' gate, not "any management role"."""
    async def fake_role(request):
        return "viewer"

    monkeypatch.setattr(server, "_caller_role", fake_role)
    resp = client.get("/users", headers=auth_headers)
    assert resp.status_code == 403
    assert "admin access required" in resp.text.lower()


def test_users_page_admin_sees_table(client, auth_headers, monkeypatch):
    """role=='admin' -> 200, renders the fetched user list."""
    async def fake_role(request):
        return "admin"

    async def fake_fetch_users(request):
        return (
            [{
                "user_id": 7, "login": "octocat",
                "email": "octo@example.com", "role": "operator",
            }],
            None,
        )

    async def fake_fetch_collateral():
        return {7: 500_000_000}

    monkeypatch.setattr(server, "_caller_role", fake_role)
    monkeypatch.setattr(server, "_fetch_admin_users", fake_fetch_users)
    monkeypatch.setattr(
        server, "_fetch_user_collateral", fake_fetch_collateral)

    resp = client.get("/users", headers=auth_headers)
    assert resp.status_code == 200
    assert "octocat" in resp.text
    assert "operator" in resp.text
    assert "5.0" in resp.text  # 500_000_000 / 1e8


def test_users_page_never_grants_access_via_admin_request_gate(client):
    """The loopback-admin gate (_require_admin_request, used by the
    ops harness) must NOT double as the /users gate -- a loopback dev
    caller with no rsx-auth role is still refused."""
    resp = client.get("/users")  # TestClient requests are loopback
    assert resp.status_code == 403


def test_caller_role_returns_none_without_bearer_token(client):
    """_caller_role never trusts an x-user-id header -- role-gating
    always requires a real token."""
    import asyncio
    from starlette.requests import Request as StarletteRequest

    async def _check():
        scope = {
            "type": "http", "method": "GET", "path": "/users",
            "headers": [], "query_string": b"",
            "client": ("127.0.0.1", 12345),
        }
        request = StarletteRequest(scope)
        return await server._caller_role(request)

    role = asyncio.run(_check())
    assert role is None


def test_roles_proxy_translates_cookie_to_bearer(client):
    """rsx-auth's verify_bearer checks ONLY the Authorization header,
    never a cookie. A browser session (rsx_token cookie, no custom
    header -- exactly what this tab's own client-side fetch() calls
    send) must still reach /roles/* -- the proxy translates the
    cookie into a Bearer header rather than 401ing every logged-in
    browser call."""
    class FakeResp:
        status = 200
        headers = {}
        async def read(self):
            return b'{"user_id": 1, "role": "admin"}'
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    call = {}

    class FakeSession:
        def request(self, method, url, **kwargs):
            call["headers"] = kwargs.get("headers", {})
            return FakeResp()
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    client.cookies.set("rsx_token", "cookie-only-token")
    with patch("aiohttp.ClientSession", return_value=FakeSession()):
        resp = client.get("/roles/me")
    assert resp.status_code == 200
    assert (
        call["headers"].get("authorization")
        == "Bearer cookie-only-token"
    )
    # The raw cookie still forwards too (OAuth's own state/redirect
    # cookies depend on this on /oauth/* -- not exercised here, but
    # the translation must be additive, not a replacement).
    assert "rsx_token=cookie-only-token" in call["headers"].get(
        "cookie", "")


def test_roles_proxy_forwards_get(client):
    """GET /roles/{path} proxies to AUTH_HTTP/roles/{path}."""
    class FakeResp:
        status = 200
        headers = {}
        async def read(self):
            return b'{"user_id": 1, "role": "admin"}'
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    call = {}

    class FakeSession:
        def request(self, method, url, **kwargs):
            call["method"] = method
            call["url"] = url
            call["headers"] = kwargs.get("headers", {})
            return FakeResp()
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    with patch("aiohttp.ClientSession", return_value=FakeSession()):
        resp = client.get(
            "/roles/me",
            headers={"authorization": "Bearer sometoken"},
        )
    assert resp.status_code == 200
    assert call["url"] == f"{server.AUTH_HTTP}/roles/me"
    assert call["headers"].get("authorization") == "Bearer sometoken"


def test_roles_proxy_forwards_idempotency_key(client):
    """POST /roles/{path} forwards Idempotency-Key -- rsx-auth's
    role-change endpoint requires it (400 without one)."""
    class FakeResp:
        status = 200
        headers = {}
        async def read(self):
            return b'{"user_id": 7, "role": "admin"}'
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    call = {}

    class FakeSession:
        def request(self, method, url, **kwargs):
            call["headers"] = kwargs.get("headers", {})
            return FakeResp()
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    with patch("aiohttp.ClientSession", return_value=FakeSession()):
        resp = client.post(
            "/roles/users/7/role",
            headers={
                "authorization": "Bearer sometoken",
                "idempotency-key": "ui-7-123",
                "content-type": "application/json",
            },
            content=b'{"role":"admin","reason":"promo","ticket_id":"T1"}',
        )
    assert resp.status_code == 200
    assert call["headers"].get("idempotency-key") == "ui-7-123"
