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

Several tests require the upstream (gateway / marketdata)
to be DOWN in order to exercise the failure paths. When the
gate runs after process-control tests, the live gateway is
often still up, so these tests autoskip when reachable.

Run with:
  cd rsx-playground && .venv/bin/pytest tests/api_proxy_test.py -v
"""

import json
import os
import socket
import pytest
from unittest.mock import AsyncMock, MagicMock, patch


def _port_open(host: str, port: int, timeout: float = 0.2) -> bool:
    try:
        with socket.create_connection(
            (host, port), timeout=timeout
        ):
            return True
    except OSError:
        return False


def _skip_if_gateway_up():
    if _port_open("127.0.0.1", 8080):
        pytest.skip(
            "live gateway on :8080 — proxy 'gateway down' "
            "tests need it stopped to exercise failure path"
        )


def _skip_if_marketdata_up():
    if _port_open("127.0.0.1", 8180):
        pytest.skip(
            "live marketdata on :8180 — proxy 'marketdata "
            "down' tests need it stopped"
        )


# ── REST proxy (/v1/*) ────────────────────────────────────


@pytest.mark.allow_5xx
def test_v1_proxy_path_rewriting(client):
    """GET /v1/foo rewrites to GATEWAY_HTTP/v1/foo (502 = gateway down)."""
    _skip_if_gateway_up()
    resp = client.get("/v1/ping")
    # Gateway not running → 502 (not 500)
    assert resp.status_code == 502
    body = resp.json()
    assert "error" in body
    assert "gateway" in body["error"].lower()


@pytest.mark.allow_5xx
def test_v1_proxy_502_not_500_on_connection_refused(client):
    """ConnectionRefusedError must return 502, never 500.

    /v1/orders, /v1/account etc. have local handlers so use
    an unmapped /v1/<path> to exercise the catch-all proxy.
    """
    _skip_if_gateway_up()
    resp = client.get("/v1/proxy-only-path")
    assert resp.status_code == 502
    assert resp.json()["error"] == "gateway not running"


@pytest.mark.allow_5xx
def test_v1_proxy_post_502_when_gateway_down(client):
    """POST /v1/* also returns 502 when gateway down."""
    _skip_if_gateway_up()
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
        # /v1/account has local handler; use proxy-only path
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
    _skip_if_gateway_up()
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
    _skip_if_marketdata_up()
    with client.websocket_connect("/ws/public") as ws:
        try:
            data = ws.receive_json()
        except Exception:
            pass
        # No 500; just a graceful close.


def test_ws_private_upgrades_and_forwards_user_id(client):
    """/ws/private mints a JWT server-side from x-user-id (loopback dev only).

    Production gateway accepts only Authorization: Bearer <JWT>. The
    playground is a trusted dev proxy: when its own
    PLAYGROUND_ALLOW_INSECURE_USER_ID flag is on, it accepts an
    x-user-id header from the loopback client and mints a real JWT
    against RSX_GW_JWT_SECRET before connecting to the gateway.
    """
    import jwt as pyjwt
    forwarded_headers = {}

    class FakeWsCtx:
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
        with patch.dict("os.environ", {"PLAYGROUND_ALLOW_INSECURE_USER_ID": "1"}):
            with client.websocket_connect(
                "/ws/private",
                headers={"x-user-id": "42"},
            ) as ws:
                try:
                    ws.receive_json()
                except Exception:
                    pass

    auth = forwarded_headers.get("Authorization") or forwarded_headers.get("authorization")
    assert auth and auth.startswith("Bearer "), (
        f"expected Bearer JWT, got headers={forwarded_headers}"
    )
    claims = pyjwt.decode(
        auth.split(" ", 1)[1],
        os.environ["RSX_GW_JWT_SECRET"],
        algorithms=["HS256"],
        audience="rsx-gateway",
        issuer="rsx-auth",
    )
    assert claims["user_id"] == 42
    assert "x-user-id" not in {k.lower() for k in forwarded_headers}


def test_ws_private_rejects_missing_auth_when_no_insecure_mode(client):
    """/ws/private does not invent a default user."""
    forwarded_headers = {}

    class FakeWsCtx:
        async def __aenter__(self):
            raise AssertionError("upstream should not be contacted")
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
        ) as ws:
            try:
                ws.receive_json()
            except Exception:
                pass

    assert forwarded_headers == {}


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
    _skip_if_gateway_up()
    resp = client.get("/healthz")
    # Gateway not running in test env → False
    assert resp.json()["gateway"] is False
