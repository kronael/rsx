"""Tests for /api/stress/run: gateway-down returns HTTP 502 + error payload.

Covers:
- JSON path: POST /api/stress/run returns 502 with {"status":"error","error":...}
  when gateway is unreachable
- HTMX path: same POST with hx-request:true returns 200 HTML with error span
- Exception path: unexpected exception in run_stress_test returns 502 + payload
- Successful path: gateway reachable returns 200 with completed status
- _probe_gateway fast-fail: unreachable gateway is detected before workers spawn
"""

from unittest.mock import AsyncMock
from unittest.mock import patch

import pytest
from fastapi.testclient import TestClient

from server import app


@pytest.fixture
def client():
    return TestClient(app)


# ── Gateway unreachable: JSON path ─────────────────────────────────────────


@pytest.mark.allow_5xx
def test_stress_run_gateway_down_returns_502(client):
    """POST /api/stress/run returns HTTP 502 when gateway unreachable."""
    error_result = {
        "error": "gateway unreachable at ws://localhost:8080: Connection refused",
        "config": {"target_rate": 100, "duration": 1, "connections": 10},
        "metrics": {
            "submitted": 0, "accepted": 0, "rejected": 0,
            "errors": 1, "elapsed_sec": 0.0,
            "actual_rate": 0.0, "accept_rate": 0.0,
        },
        "latency_us": {"p50": 0, "p95": 0, "p99": 0, "min": 0, "max": 0},
    }
    with patch(
        "server.run_stress_test",
        new=AsyncMock(return_value=error_result),
    ):
        resp = client.post("/api/stress/run?rate=100&duration=1")

    assert resp.status_code == 502


@pytest.mark.allow_5xx
def test_stress_run_gateway_down_returns_json_error_status(client):
    """502 response body has status=error."""
    error_result = {
        "error": "gateway unreachable at ws://localhost:8080: Connection refused",
        "config": {"target_rate": 100, "duration": 1, "connections": 10},
        "metrics": {
            "submitted": 0, "accepted": 0, "rejected": 0,
            "errors": 1, "elapsed_sec": 0.0,
            "actual_rate": 0.0, "accept_rate": 0.0,
        },
        "latency_us": {"p50": 0, "p95": 0, "p99": 0, "min": 0, "max": 0},
    }
    with patch(
        "server.run_stress_test",
        new=AsyncMock(return_value=error_result),
    ):
        resp = client.post("/api/stress/run?rate=100&duration=1")

    data = resp.json()
    assert data["status"] == "error"


@pytest.mark.allow_5xx
def test_stress_run_gateway_down_returns_json_error_key(client):
    """502 response body has non-empty error field."""
    msg = "gateway unreachable at ws://localhost:8080: Connection refused"
    error_result = {
        "error": msg,
        "config": {"target_rate": 100, "duration": 1, "connections": 10},
        "metrics": {
            "submitted": 0, "accepted": 0, "rejected": 0,
            "errors": 1, "elapsed_sec": 0.0,
            "actual_rate": 0.0, "accept_rate": 0.0,
        },
        "latency_us": {"p50": 0, "p95": 0, "p99": 0, "min": 0, "max": 0},
    }
    with patch(
        "server.run_stress_test",
        new=AsyncMock(return_value=error_result),
    ):
        resp = client.post("/api/stress/run?rate=100&duration=1")

    data = resp.json()
    assert "error" in data
    assert data["error"] == msg


@pytest.mark.allow_5xx
def test_stress_run_gateway_down_error_mentions_gateway(client):
    """502 error message mentions gateway or unreachable."""
    msg = "gateway unreachable at ws://localhost:8080: Connection refused"
    error_result = {
        "error": msg,
        "config": {"target_rate": 100, "duration": 1, "connections": 10},
        "metrics": {
            "submitted": 0, "accepted": 0, "rejected": 0,
            "errors": 1, "elapsed_sec": 0.0,
            "actual_rate": 0.0, "accept_rate": 0.0,
        },
        "latency_us": {"p50": 0, "p95": 0, "p99": 0, "min": 0, "max": 0},
    }
    with patch(
        "server.run_stress_test",
        new=AsyncMock(return_value=error_result),
    ):
        resp = client.post("/api/stress/run?rate=100&duration=1")

    data = resp.json()
    err = data["error"].lower()
    assert "gateway" in err or "unreachable" in err or "connection" in err


@pytest.mark.allow_5xx
def test_stress_run_gateway_down_content_type_json(client):
    """502 response content-type is application/json."""
    error_result = {
        "error": "gateway unreachable at ws://localhost:8080: Connection refused",
        "config": {"target_rate": 100, "duration": 1, "connections": 10},
        "metrics": {
            "submitted": 0, "accepted": 0, "rejected": 0,
            "errors": 1, "elapsed_sec": 0.0,
            "actual_rate": 0.0, "accept_rate": 0.0,
        },
        "latency_us": {"p50": 0, "p95": 0, "p99": 0, "min": 0, "max": 0},
    }
    with patch(
        "server.run_stress_test",
        new=AsyncMock(return_value=error_result),
    ):
        resp = client.post("/api/stress/run?rate=100&duration=1")

    assert "application/json" in resp.headers["content-type"]


# ── HTMX path: gateway down returns 200 HTML error span ───────────────────


def test_stress_run_htmx_gateway_down_returns_200(client):
    """HTMX POST /api/stress/run returns 200 (not 502) with hx-request:true."""
    error_result = {
        "error": "gateway unreachable at ws://localhost:8080: Connection refused",
        "config": {"target_rate": 100, "duration": 1, "connections": 10},
        "metrics": {
            "submitted": 0, "accepted": 0, "rejected": 0,
            "errors": 1, "elapsed_sec": 0.0,
            "actual_rate": 0.0, "accept_rate": 0.0,
        },
        "latency_us": {"p50": 0, "p95": 0, "p99": 0, "min": 0, "max": 0},
    }
    with patch(
        "server.run_stress_test",
        new=AsyncMock(return_value=error_result),
    ):
        resp = client.post(
            "/api/stress/run?rate=100&duration=1",
            headers={"hx-request": "true"},
        )

    assert resp.status_code == 200


def test_stress_run_htmx_gateway_down_returns_html(client):
    """HTMX path returns text/html content-type on gateway down."""
    error_result = {
        "error": "gateway unreachable at ws://localhost:8080: Connection refused",
        "config": {"target_rate": 100, "duration": 1, "connections": 10},
        "metrics": {
            "submitted": 0, "accepted": 0, "rejected": 0,
            "errors": 1, "elapsed_sec": 0.0,
            "actual_rate": 0.0, "accept_rate": 0.0,
        },
        "latency_us": {"p50": 0, "p95": 0, "p99": 0, "min": 0, "max": 0},
    }
    with patch(
        "server.run_stress_test",
        new=AsyncMock(return_value=error_result),
    ):
        resp = client.post(
            "/api/stress/run?rate=100&duration=1",
            headers={"hx-request": "true"},
        )

    assert "text/html" in resp.headers["content-type"]


def test_stress_run_htmx_gateway_down_contains_error_text(client):
    """HTMX path error span contains 'gateway unreachable'."""
    error_result = {
        "error": "gateway unreachable at ws://localhost:8080: Connection refused",
        "config": {"target_rate": 100, "duration": 1, "connections": 10},
        "metrics": {
            "submitted": 0, "accepted": 0, "rejected": 0,
            "errors": 1, "elapsed_sec": 0.0,
            "actual_rate": 0.0, "accept_rate": 0.0,
        },
        "latency_us": {"p50": 0, "p95": 0, "p99": 0, "min": 0, "max": 0},
    }
    with patch(
        "server.run_stress_test",
        new=AsyncMock(return_value=error_result),
    ):
        resp = client.post(
            "/api/stress/run?rate=100&duration=1",
            headers={"hx-request": "true"},
        )

    assert "gateway unreachable" in resp.text.lower()


def test_stress_run_htmx_gateway_down_contains_red_span(client):
    """HTMX path error response uses red-coloured span element."""
    error_result = {
        "error": "gateway unreachable at ws://localhost:8080: Connection refused",
        "config": {"target_rate": 100, "duration": 1, "connections": 10},
        "metrics": {
            "submitted": 0, "accepted": 0, "rejected": 0,
            "errors": 1, "elapsed_sec": 0.0,
            "actual_rate": 0.0, "accept_rate": 0.0,
        },
        "latency_us": {"p50": 0, "p95": 0, "p99": 0, "min": 0, "max": 0},
    }
    with patch(
        "server.run_stress_test",
        new=AsyncMock(return_value=error_result),
    ):
        resp = client.post(
            "/api/stress/run?rate=100&duration=1",
            headers={"hx-request": "true"},
        )

    # The HTMX error path uses text-red-400 span
    assert "text-red" in resp.text or "<span" in resp.text


# ── Exception path: unexpected error returns 502 ──────────────────────────


@pytest.mark.allow_5xx
def test_stress_run_exception_returns_502(client):
    """Unexpected exception from run_stress_test returns HTTP 502."""
    with patch(
        "server.run_stress_test",
        new=AsyncMock(side_effect=RuntimeError("internal crash")),
    ):
        resp = client.post("/api/stress/run?rate=100&duration=1")

    assert resp.status_code == 502


@pytest.mark.allow_5xx
def test_stress_run_exception_returns_json_error(client):
    """Exception path returns JSON with status=error."""
    with patch(
        "server.run_stress_test",
        new=AsyncMock(side_effect=RuntimeError("internal crash")),
    ):
        resp = client.post("/api/stress/run?rate=100&duration=1")

    data = resp.json()
    assert data["status"] == "error"
    assert "error" in data


def test_stress_run_exception_htmx_returns_200(client):
    """Exception + hx-request returns 200 HTML span, not 502."""
    with patch(
        "server.run_stress_test",
        new=AsyncMock(side_effect=RuntimeError("internal crash")),
    ):
        resp = client.post(
            "/api/stress/run?rate=100&duration=1",
            headers={"hx-request": "true"},
        )

    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


# ── Success path: gateway reachable returns 200 completed ─────────────────


def test_stress_run_success_returns_200(client):
    """POST /api/stress/run returns 200 when stress test completes."""
    success_result = {
        "config": {"target_rate": 100, "duration": 1, "connections": 10},
        "metrics": {
            "submitted": 10, "accepted": 8, "rejected": 0,
            "errors": 0, "elapsed_sec": 1.0,
            "actual_rate": 10.0, "accept_rate": 80.0,
        },
        "latency_us": {"p50": 500, "p95": 1000, "p99": 2000, "min": 100, "max": 3000},
    }
    with patch(
        "server.run_stress_test",
        new=AsyncMock(return_value=success_result),
    ):
        resp = client.post("/api/stress/run?rate=100&duration=1")

    assert resp.status_code == 200


def test_stress_run_success_returns_completed_status(client):
    """Successful stress run returns status=completed in JSON."""
    success_result = {
        "config": {"target_rate": 100, "duration": 1, "connections": 10},
        "metrics": {
            "submitted": 10, "accepted": 8, "rejected": 0,
            "errors": 0, "elapsed_sec": 1.0,
            "actual_rate": 10.0, "accept_rate": 80.0,
        },
        "latency_us": {"p50": 500, "p95": 1000, "p99": 2000, "min": 100, "max": 3000},
    }
    with patch(
        "server.run_stress_test",
        new=AsyncMock(return_value=success_result),
    ):
        resp = client.post("/api/stress/run?rate=100&duration=1")

    data = resp.json()
    assert data["status"] == "completed"


def test_stress_run_success_returns_report_id(client):
    """Successful run returns report_id for linking to saved report."""
    success_result = {
        "config": {"target_rate": 100, "duration": 1, "connections": 10},
        "metrics": {
            "submitted": 10, "accepted": 8, "rejected": 0,
            "errors": 0, "elapsed_sec": 1.0,
            "actual_rate": 10.0, "accept_rate": 80.0,
        },
        "latency_us": {"p50": 500, "p95": 1000, "p99": 2000, "min": 100, "max": 3000},
    }
    with patch(
        "server.run_stress_test",
        new=AsyncMock(return_value=success_result),
    ):
        resp = client.post("/api/stress/run?rate=100&duration=1")

    data = resp.json()
    assert "report_id" in data
    assert data["report_id"]  # non-empty


# ── _probe_gateway in stress_client: unit tests ────────────────────────────


@pytest.mark.asyncio
async def test_probe_gateway_connection_refused_returns_error():
    """_probe_gateway returns error string when port is not listening."""
    import aiohttp
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
