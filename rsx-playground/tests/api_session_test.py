"""Session/identity chip tests (/x/session-chip).

Covers the nav login affordance: logged-out (no auth) vs logged-in
(caller's own resolved user, per `_resolve_request_user`) vs rsx-auth
unreachable (graceful fallback, never a 5xx).

Run with: cd rsx-playground && uv run pytest tests/api_session_test.py -v
"""

from unittest.mock import patch


def test_session_chip_logged_out(client):
    """No auth header/cookie -> a login link, no identity leaked."""
    resp = client.get("/x/session-chip")
    assert resp.status_code == 200
    assert "log in with GitHub" in resp.text
    assert "/oauth/github/login" in resp.text


def test_session_chip_logged_in(client, auth_headers):
    """A caller's own Bearer token resolves to their identity + role,
    fetched from rsx-auth's /auth/me using the caller's forwarded
    headers (never a second local decode of provider identity)."""
    class FakeResp:
        status = 200
        async def json(self):
            return {
                "user_id": 1, "login": "octocat", "role": "admin",
            }
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    class FakeSession:
        def get(self, url, headers: dict | None = None, **kwargs):
            assert headers is not None
            assert headers.get("authorization") == \
                auth_headers["authorization"]
            return FakeResp()
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    with patch("aiohttp.ClientSession", return_value=FakeSession()):
        resp = client.get("/x/session-chip", headers=auth_headers)

    assert resp.status_code == 200
    assert "octocat" in resp.text
    assert "admin" in resp.text
    assert "log out" in resp.text


def test_session_chip_falls_back_when_auth_unreachable(client, auth_headers):
    """rsx-auth down -> chip still renders (bare user_id), no 5xx."""
    class FakeSession:
        def get(self, *a, **kw):
            raise ConnectionRefusedError("not running")
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    with patch("aiohttp.ClientSession", return_value=FakeSession()):
        resp = client.get("/x/session-chip", headers=auth_headers)

    assert resp.status_code == 200
    assert "user 1" in resp.text
    assert "log out" in resp.text


def test_session_chip_ignores_spoofed_user_id_header(client):
    """A bare x-user-id header (no PLAYGROUND_ALLOW_INSECURE_USER_ID,
    no loopback dev mode assumed here) never resolves an identity --
    only a verified token does."""
    resp = client.get("/x/session-chip", headers={"x-user-id": "999"})
    assert resp.status_code == 200
    assert "log in with GitHub" in resp.text
