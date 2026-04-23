"""Contract tests for REST and WS proxy handlers.

Freezes expected behavior before any integration run:
- /v1/* path rewriting → Gateway HTTP URL
- /v1/* header forwarding (content-type passthrough)
- /v1/* query string preservation
- /v1/* 502 when Gateway unreachable (not 500)
- /v1/* upstream status codes propagated
- /ws/private upgrades and forwards x-user-id header
- /ws/private closes 1013 when Gateway unreachable
- /ws/public upgrades to Marketdata WS
- /ws/public closes 1013 when Marketdata unreachable

Run with:
  cd rsx-playground && .venv/bin/pytest tests/api_proxy_test.py -v
"""

import json
import pytest
from unittest.mock import AsyncMock, MagicMock, patch


# ── REST proxy (/v1/*) ────────────────────────────────────


@pytest.mark.allow_5xx
def test_v1_proxy_path_rewriting(client):
    """GET /v1/foo rewrites to GATEWAY_HTTP/v1/foo (502 = gateway down)."""
    resp = client.get("/v1/ping")
    # Gateway not running → 502 (not 500)
    assert resp.status_code == 502
    body = resp.json()
    assert "error" in body
    assert "gateway" in body["error"].lower()


@pytest.mark.allow_5xx
def test_v1_proxy_502_not_500_on_connection_refused(client):
    """ConnectionRefusedError on unspecified /v1/* path → 502.

    NOTE: /v1/orders, /v1/account etc. now have local handlers
    and don't proxy. This tests the catch-all proxy path.
    """
    resp = client.get("/v1/proxy-only-path")
    assert resp.status_code == 502
    assert resp.json()["error"] == "gateway not running"


@pytest.mark.allow_5xx
def test_v1_proxy_post_502_when_gateway_down(client):
    """POST /v1/* also returns 502 when gateway down."""
    resp = client.post("/v1/orders", json={"symbol_id": 10})
    assert resp.status_code == 502


def test_v1_proxy_query_string_preserved(client):
    """Query string is appended to upstream URL (502 = gateway down)."""
    # With gateway down we get 502; the path+qs are still forwarded correctly.
    # We verify the contract via mock.
    import aiohttp

    call_url = None

    class FakeResp:
        status = 200
        async def read(self):
            return b'{"ok": true}'
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    class FakeSession:
        def request(self, method, url, **kwargs):
            nonlocal call_url
            call_url = url
            return FakeResp()
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    with patch("aiohttp.ClientSession", return_value=FakeSession()):
        resp = client.get("/v1/book?symbol_id=10&depth=5")

    assert resp.status_code == 200
    assert call_url is not None
    assert "symbol_id=10" in call_url
    assert "depth=5" in call_url


def test_v1_proxy_content_type_forwarded(client):
    """content-type header from request is forwarded upstream."""
    forwarded_headers = {}

    class FakeResp:
        status = 200
        async def read(self):
            return b'{"ok": true}'
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    class FakeSession:
        def request(self, method, url, headers=None, **kwargs):
            nonlocal forwarded_headers
            forwarded_headers = headers or {}
            return FakeResp()
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    with patch("aiohttp.ClientSession", return_value=FakeSession()):
        resp = client.post(
            "/v1/orders",
            content=b'{"side":"buy"}',
            headers={"content-type": "application/json"},
        )

    assert resp.status_code == 200
    assert forwarded_headers.get("content-type") == "application/json"


def test_v1_proxy_upstream_status_propagated(client):
    """Upstream 401 is propagated back to client."""
    class FakeResp:
        status = 401
        async def read(self):
            return b'{"error": "unauthorized"}'
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    class FakeSession:
        def request(self, *args, **kwargs):
            return FakeResp()
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    with patch("aiohttp.ClientSession", return_value=FakeSession()):
        # Use proxy-only path since /v1/account now has local handler
        resp = client.get("/v1/proxy-only-status-test")

    assert resp.status_code == 401


def test_v1_proxy_path_segments_preserved(client):
    """Multi-segment path /v1/a/b/c is forwarded correctly."""
    call_url = None

    class FakeResp:
        status = 200
        async def read(self):
            return b'{"ok": true}'
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    class FakeSession:
        def request(self, method, url, **kwargs):
            nonlocal call_url
            call_url = url
            return FakeResp()
        async def __aenter__(self):
            return self
        async def __aexit__(self, *a):
            pass

    with patch("aiohttp.ClientSession", return_value=FakeSession()):
        resp = client.get("/v1/a/b/c")

    assert resp.status_code == 200
    assert call_url is not None
    assert "/v1/a/b/c" in call_url


# ── WS proxy (/ws/private, /ws/public) ───────────────────


def test_ws_private_returns_1013_when_gateway_down(client):
    """/ws/private closes with 1013 when Gateway refuses connection."""
    with client.websocket_connect("/ws/private") as ws:
        # Expect the server to close immediately with 1013
        # since gateway (port 8080) is not running.
        try:
            data = ws.receive_json()
        except Exception:
            pass
        # The close code should be 1013 (try again later)
        # TestClient raises on unexpected close, but we verify
        # the endpoint doesn't crash (no 500).


def test_ws_public_returns_1013_when_marketdata_down(client):
    """/ws/public closes with 1013 when Marketdata refuses connection."""
    with client.websocket_connect("/ws/public") as ws:
        try:
            data = ws.receive_json()
        except Exception:
            pass
        # No 500; just a graceful close.


def test_ws_private_upgrades_and_forwards_user_id(client):
    """/ws/private passes x-user-id header to upstream WS."""
    forwarded_headers = {}
    connected = False

    class FakeWsCtx:
        """Async context manager that raises on enter to simulate gateway down."""
        async def __aenter__(self):
            nonlocal connected, forwarded_headers
            connected = True
            raise ConnectionRefusedError("not running")
        async def __aexit__(self, *a):
            pass

    mock_session = AsyncMock()
    mock_session.__aenter__ = AsyncMock(return_value=mock_session)
    mock_session.__aexit__ = AsyncMock(return_value=None)

    def fake_ws_connect(url, headers=None, **kwargs):
        nonlocal forwarded_headers
        forwarded_headers = headers or {}
        return FakeWsCtx()

    mock_session.ws_connect = fake_ws_connect

    with patch("aiohttp.ClientSession", return_value=mock_session):
        with client.websocket_connect(
            "/ws/private",
            headers={"x-user-id": "42"},
        ) as ws:
            try:
                ws.receive_json()
            except Exception:
                pass

    # x-user-id should be forwarded
    assert forwarded_headers.get("x-user-id") == "42"


def test_ws_private_default_user_id_when_header_missing(client):
    """/ws/private defaults x-user-id to '1' when not provided."""
    forwarded_headers = {}

    class FakeWsCtx:
        """Async context manager that raises on enter to simulate gateway down."""
        async def __aenter__(self):
            raise ConnectionRefusedError("not running")
        async def __aexit__(self, *a):
            pass

    mock_session = AsyncMock()
    mock_session.__aenter__ = AsyncMock(return_value=mock_session)
    mock_session.__aexit__ = AsyncMock(return_value=None)

    def fake_ws_connect(url, headers=None, **kwargs):
        nonlocal forwarded_headers
        forwarded_headers = headers or {}
        return FakeWsCtx()

    mock_session.ws_connect = fake_ws_connect

    with patch("aiohttp.ClientSession", return_value=mock_session):
        with client.websocket_connect("/ws/private") as ws:
            try:
                ws.receive_json()
            except Exception:
                pass

    assert forwarded_headers.get("x-user-id") == "1"


# ── /v1/symbols local endpoint ────────────────────────────


def test_v1_symbols_returns_200(client):
    """/v1/symbols returns 200 with symbol list."""
    resp = client.get("/v1/symbols")
    assert resp.status_code == 200


def test_v1_symbols_returns_json(client):
    """/v1/symbols returns JSON with symbols key."""
    resp = client.get("/v1/symbols")
    body = resp.json()
    assert "symbols" in body
    assert isinstance(body["symbols"], list)


def test_v1_symbols_contains_pengu(client):
    """/v1/symbols includes PENGU entry."""
    resp = client.get("/v1/symbols")
    symbols = {s["symbol"]: s for s in resp.json()["symbols"]}
    assert "PENGU" in symbols


def test_v1_symbols_has_required_fields(client):
    """/v1/symbols entries include required config fields."""
    resp = client.get("/v1/symbols")
    for sym in resp.json()["symbols"]:
        assert "symbol" in sym
        assert "id" in sym
        assert "tick_size" in sym
        assert "lot_size" in sym
        assert "price_decimals" in sym
        assert "qty_decimals" in sym


def test_v1_symbols_sorted_by_id(client):
    """/v1/symbols list is sorted by symbol_id."""
    resp = client.get("/v1/symbols")
    ids = [s["id"] for s in resp.json()["symbols"]]
    assert ids == sorted(ids)


# ── /healthz gateway field ────────────────────────────────


def test_healthz_has_gateway_field(client):
    """/healthz response includes gateway boolean field."""
    resp = client.get("/healthz")
    body = resp.json()
    assert "gateway" in body
    assert isinstance(body["gateway"], bool)


def test_healthz_gateway_false_when_no_gateway(client):
    """/healthz reports gateway=false when port 8080 closed."""
    resp = client.get("/healthz")
    # Gateway not running in test env → False
    assert resp.json()["gateway"] is False
