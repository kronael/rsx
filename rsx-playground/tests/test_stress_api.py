"""Tests for /api/stress/run: gateway-reachability probe envelope.

The endpoint probes the gateway via TCP connect and returns a
structured envelope (`code` / `message` / `context`):

- gateway unreachable: 502 with `code=GATEWAY_UNREACHABLE`
  (or HTMX 200 HTML for `hx-request: true`)
- bad input (rate<=0): 400 with `code=BAD_REQUEST`
- gateway reachable: 200 with `code=OK`

Plus unit tests for `_probe_gateway` + `run_stress_test` from
stress_client when no gateway is up.

History: an earlier design exposed `server.run_stress_test` and
returned `{"status":"completed"/"error","report_id":...}`. That
contract was removed; tests that mocked `server.run_stress_test`
or asserted on the old envelope have been replaced with the
ones below.
"""

import pytest
from fastapi.testclient import TestClient

from server import app


@pytest.fixture
def client():
    return TestClient(app)


# ── Gateway unreachable: JSON path ─────────────────────────────────────────


@pytest.mark.allow_5xx
def test_stress_run_gateway_down_returns_502(client):
    """POST /api/stress/run returns 502 when gateway is unreachable."""
    # Point at an obviously-closed port (no gateway listening).
    resp = client.post(
        "/api/stress/run",
        data={
            "rate": "100",
            "duration": "1",
            "gateway_url": "ws://127.0.0.1:1",
        },
    )
    assert resp.status_code == 502


@pytest.mark.allow_5xx
def test_stress_run_gateway_down_returns_gateway_unreachable_code(client):
    """502 body has code=GATEWAY_UNREACHABLE."""
    resp = client.post(
        "/api/stress/run",
        data={
            "rate": "100",
            "duration": "1",
            "gateway_url": "ws://127.0.0.1:1",
        },
    )
    data = resp.json()
    assert data["code"] == "GATEWAY_UNREACHABLE"


@pytest.mark.allow_5xx
def test_stress_run_gateway_down_returns_message(client):
    """502 body has a non-empty `message` describing the failure."""
    resp = client.post(
        "/api/stress/run",
        data={
            "rate": "100",
            "duration": "1",
            "gateway_url": "ws://127.0.0.1:1",
        },
    )
    data = resp.json()
    assert "message" in data and data["message"]


@pytest.mark.allow_5xx
def test_stress_run_gateway_down_message_mentions_gateway(client):
    """502 message mentions gateway or reachability."""
    resp = client.post(
        "/api/stress/run",
        data={
            "rate": "100",
            "duration": "1",
            "gateway_url": "ws://127.0.0.1:1",
        },
    )
    data = resp.json()
    msg = data["message"].lower()
    assert "gateway" in msg or "reach" in msg


@pytest.mark.allow_5xx
def test_stress_run_gateway_down_content_type_json(client):
    """502 response content-type is application/json."""
    resp = client.post(
        "/api/stress/run",
        data={
            "rate": "100",
            "duration": "1",
            "gateway_url": "ws://127.0.0.1:1",
        },
    )
    assert "application/json" in resp.headers["content-type"]


# ── HTMX path: gateway down returns 200 HTML error span ───────────────────


def test_stress_run_htmx_gateway_down_returns_200(client):
    """HTMX POST returns 200 (not 502) when gateway is unreachable."""
    resp = client.post(
        "/api/stress/run",
        data={
            "rate": "100",
            "duration": "1",
            "gateway_url": "ws://127.0.0.1:1",
        },
        headers={"hx-request": "true"},
    )
    assert resp.status_code == 200


def test_stress_run_htmx_gateway_down_returns_html(client):
    """HTMX path returns text/html on gateway down."""
    resp = client.post(
        "/api/stress/run",
        data={
            "rate": "100",
            "duration": "1",
            "gateway_url": "ws://127.0.0.1:1",
        },
        headers={"hx-request": "true"},
    )
    assert "text/html" in resp.headers["content-type"]


def test_stress_run_htmx_gateway_down_contains_error_text(client):
    """HTMX error span contains 'gateway unreachable'."""
    resp = client.post(
        "/api/stress/run",
        data={
            "rate": "100",
            "duration": "1",
            "gateway_url": "ws://127.0.0.1:1",
        },
        headers={"hx-request": "true"},
    )
    assert "gateway unreachable" in resp.text.lower()


def test_stress_run_htmx_gateway_down_contains_red_span(client):
    """HTMX error response uses a red-coloured span element."""
    resp = client.post(
        "/api/stress/run",
        data={
            "rate": "100",
            "duration": "1",
            "gateway_url": "ws://127.0.0.1:1",
        },
        headers={"hx-request": "true"},
    )
    assert "text-red" in resp.text or "<span" in resp.text


# ── Bad request: rate <= 0 returns 400 BAD_REQUEST ─────────────────────────


@pytest.mark.allow_5xx
def test_stress_run_zero_rate_returns_400(client):
    """rate=0 returns 400 BAD_REQUEST."""
    resp = client.post(
        "/api/stress/run",
        data={"rate": "0", "duration": "1"},
    )
    assert resp.status_code == 400
    assert resp.json()["code"] == "BAD_REQUEST"


# ── _probe_gateway in stress_client: unit tests ────────────────────────────


@pytest.mark.asyncio
async def test_probe_gateway_connection_refused_returns_error():
    """_probe_gateway returns error string when port is not listening."""
    from stress_client import _probe_gateway

    # Port 1 is never open; should fail immediately
    err = await _probe_gateway("ws://127.0.0.1:1")
    assert err is not None
    assert "unreachable" in err.lower() or "connection" in err.lower()


@pytest.mark.asyncio
async def test_run_stress_test_gateway_down_returns_error_dict():
    """run_stress_test returns dict with 'error' key when gateway down."""
    from stress_client import StressConfig
    from stress_client import run_stress_test

    config = StressConfig(
        gateway_url="ws://127.0.0.1:1",
        rate=10,
        duration=1,
        connections=1,
    )
    result = await run_stress_test(config)
    assert "error" in result
    assert result["metrics"]["submitted"] == 0
    assert result["metrics"]["errors"] >= 1


@pytest.mark.asyncio
async def test_run_stress_test_gateway_down_includes_config():
    """run_stress_test error result includes config block."""
    from stress_client import StressConfig
    from stress_client import run_stress_test

    config = StressConfig(
        gateway_url="ws://127.0.0.1:1",
        rate=10,
        duration=1,
        connections=1,
    )
    result = await run_stress_test(config)
    assert "config" in result
    assert result["config"]["target_rate"] == 10


@pytest.mark.asyncio
async def test_run_stress_test_gateway_down_includes_latency():
    """run_stress_test error result includes latency_us block with zeros."""
    from stress_client import StressConfig
    from stress_client import run_stress_test

    config = StressConfig(
        gateway_url="ws://127.0.0.1:1",
        rate=10,
        duration=1,
        connections=1,
    )
    result = await run_stress_test(config)
    assert "latency_us" in result
    lat = result["latency_us"]
    assert lat["p50"] == 0
    assert lat["p99"] == 0
