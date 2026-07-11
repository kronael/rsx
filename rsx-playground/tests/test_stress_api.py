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
from collections import deque
from unittest.mock import patch
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
    """No samples are represented as null percentiles, never fake zeroes."""
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
    assert lat["samples"] == 0
    assert lat["p50"] is None
    assert lat["p99"] is None


def _valid_result(**metrics):
    base = {
        "offered": 100, "submitted": 100, "accepted": 96,
        "rejected": 4, "completed": 100, "timed_out": 0,
        "pending": 0, "errors": 0, "send_errors": 0,
    }
    base.update(metrics)
    return {
        "metrics": base,
        "latency_us": {
            "samples": base["accepted"], "p50": 10, "p95": 20,
            "p99": 30, "p99_9": 35, "min": 5, "max": 40,
        },
    }


def test_zero_acceptance_is_invalid():
    from stress import validate_results
    result = _valid_result(
        accepted=0, rejected=100, completed=100,
    )
    result["latency_us"] = {
        "samples": 0, "p50": None, "p95": None, "p99": None,
        "p99_9": None, "min": None, "max": None,
    }
    assert (
        "zero accepted/completed latency samples"
        in validate_results(result, 50)
    )


def test_partial_acceptance_with_closed_accounting_is_valid():
    from stress import validate_results
    assert validate_results(_valid_result(), 50) == []


def test_open_accounting_is_invalid():
    from stress import validate_results
    failures = validate_results(_valid_result(submitted=101), 50)
    assert "order accounting does not close" in failures


def test_timeout_accounting_is_valid_above_terminal_threshold():
    from stress import validate_results
    result = _valid_result(
        accepted=95, rejected=0, completed=95, timed_out=5,
    )
    result["latency_us"]["samples"] = 95
    assert validate_results(result, 50) == []


def test_malformed_result_is_invalid():
    from stress import validate_results
    assert validate_results({"metrics": []}, 50) == ["malformed stress result"]


def test_stress_response_routes_interleaved_fills_by_order_id():
    """F is non-terminal; only the matching terminal U closes each order."""
    from stress_client import StressClient, StressConfig

    client = StressClient(0, StressConfig())
    awaiting = deque(["cid-a", "cid-b"])
    pending = {"cid-a": 1_000_000, "cid-b": 2_000_000}
    by_oid = {}
    with patch("stress_client.time.perf_counter_ns", return_value=5_000_000):
        client._handle_response(
            {"F": ["oid-a", "maker-x", 100, 2, 3, 0]},
            awaiting, pending, by_oid,
        )
        client._handle_response(
            {"F": ["oid-a", "maker-y", 100, 3, 4, 0]},
            awaiting, pending, by_oid,
        )
        assert client.metrics.completed == 0
        assert set(pending) == {"cid-a", "cid-b"}

        # B's update can arrive before A's terminal update.
        by_oid["oid-b"] = awaiting.popleft()
        client._handle_response(
            {"U": ["oid-b", 1, 0, 5, 0]},
            awaiting, pending, by_oid,
        )
        client._handle_response(
            {"U": ["oid-a", 0, 5, 0, 0]},
            awaiting, pending, by_oid,
        )

    assert client.metrics.completed == 2
    assert client.metrics.accepted == 2
    assert pending == {}
    assert sorted(client.metrics.latencies_us) == [3000, 4000]


def test_stress_terminal_reject_closes_bound_order():
    from stress_client import StressClient, StressConfig

    client = StressClient(0, StressConfig())
    awaiting = deque(["cid-a"])
    pending = {"cid-a": 1}
    client._handle_response(
        {"U": ["oid-a", 3, 0, 1, 7]}, awaiting, pending, {})
    assert client.metrics.completed == 1
    assert client.metrics.rejected == 1
    assert client.metrics.rejected_by_reason == {"7": 1}
    assert pending == {}


def test_stress_report_names_do_not_collide_within_one_second(tmp_path):
    from stress import _save_report

    first = _save_report({"run": 1}, tmp_path)
    second = _save_report({"run": 2}, tmp_path)
    assert first != second
    assert first.exists() and second.exists()


def test_failed_report_list_preserves_failure_and_null_p99(
    client, tmp_path, monkeypatch,
):
    import json
    import server

    monkeypatch.setattr(server, "STRESS_REPORTS_DIR", tmp_path)
    report = {
        "config": {"target_rate": 100, "duration": 1},
        "metrics": {
            "submitted": 100, "accepted": 0, "accept_rate": 0,
        },
        "latency_us": {"p99": None},
        "status": "failed",
        "failures": ["zero accepted/completed latency samples"],
    }
    (tmp_path / "stress-20260711-010203-000001.json").write_text(
        json.dumps(report)
    )

    listing = client.get("/api/stress/reports").json()
    assert listing[0]["status"] == "failed"
    assert listing[0]["failures"] == report["failures"]
    assert listing[0]["p99_latency"] is None

    html_response = client.get("/x/stress-reports-list")
    assert html_response.status_code == 200
    assert "unavailable (failed)" in html_response.text
    assert report["failures"][0] in html_response.text
