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
async def test_stress_report_keeps_users_and_connections_separate():
    from stress_client import StressConfig
    from stress_client import run_stress_test

    config = StressConfig(
        gateway_url="ws://127.0.0.1:1",
        users=3,
        connections=7,
        rate=10,
        duration=1,
    )
    result = await run_stress_test(config)
    assert result["config"]["users"] == 3
    assert result["config"]["connections"] == 7


def test_stress_workers_use_configured_symbols():
    from stress_client import StressClient
    from stress_client import StressConfig

    config = StressConfig(symbols=[10, 20], connections=3)
    assert [StressClient(i, config).symbol_id for i in range(3)] == [10, 20, 10]


def test_stress_workers_reserve_market_maker_user():
    from stress_client import StressClient
    from stress_client import StressConfig

    config = StressConfig(users=1000, connections=1000)
    assert StressClient(97, config).user_id == 98
    assert StressClient(98, config).user_id == 100
    assert StressClient(999, config).user_id == 1001


def test_process_partition_keeps_identity_global_and_pacing_local():
    from stress_client import StressClient
    from stress_client import StressConfig

    worker = StressClient(
        3,
        StressConfig(users=1000, worker_offset=200),
    )
    assert worker.worker_id == 203
    assert worker.user_id == 205
    assert worker.pacing_index == 3


def test_twenty_partitions_share_start_and_unique_workers():
    from stress import _partition_configs
    from stress_client import StressConfig

    start_at_ns = 123_456_789
    partitions = _partition_configs(
        StressConfig(users=1000, connections=1000, rate=2000),
        20,
        start_at_ns,
    )
    worker_ids = {
        partition.worker_offset + local_id
        for partition in partitions
        for local_id in range(partition.connections)
    }
    assert len(partitions) == 20
    assert worker_ids == set(range(1000))
    assert {partition.start_at_ns for partition in partitions} == {
        start_at_ns,
    }
    assert all(not partition.probe_gateway for partition in partitions)


def test_generated_quantity_respects_configured_lot_size():
    import json

    from stress_client import StressClient
    from stress_client import StressConfig

    worker = StressClient(0, StressConfig(symbols=[10], lot_size=100_000))
    order = json.loads(worker.generate_order())["N"]
    qty = order[3]
    assert qty > 0
    assert qty % 100_000 == 0
    assert order[5] == 1


def test_generated_order_uses_its_symbol_config():
    import json

    from stress_client import StressClient
    from stress_client import StressConfig

    config = StressConfig(
        symbols=[10, 3, 1],
        symbol_configs={
            10: {"tick_size": 1, "lot_size": 100_000,
                 "bid_price": 29, "ask_price": 31},
            3: {"tick_size": 1, "lot_size": 10_000,
                "bid_price": 149, "ask_price": 151},
            1: {"tick_size": 50, "lot_size": 100, "bid_price": 6_999_950,
                "ask_price": 7_000_000},
        },
    )
    orders = [
        json.loads(StressClient(i, config).generate_order())["N"]
        for i in range(3)
    ]
    assert [(order[0], order[2], order[3]) for order in orders] == [
        (10, 31, 100_000),
        (3, 151, 10_000),
        (1, 7_000_000, 100),
    ]


def test_partition_symbol_assignment_uses_global_worker_identity():
    from stress_client import StressClient
    from stress_client import StressConfig

    config = StressConfig(symbols=[10, 3, 1], worker_offset=4)
    assert StressClient(0, config).symbol_id == 3


def test_partition_merge_reports_each_symbol():
    from stress import _merge_partitions
    from stress_client import StressConfig

    def result(symbol):
        legs = {
            "internal": [symbol],
            "gateway_to_match_done": [symbol],
            "matching": [symbol],
            "match_done_to_gateway": [symbol],
        }
        metrics = {
            "offered": 1, "submitted": 1, "accepted": 1,
            "rejected": 0, "completed": 1, "timed_out": 0,
            "pending": 0, "errors": 0, "send_errors": 0,
            "fill_attempts": 1, "filled_orders": 1, "fills": 1,
            "elapsed_sec": 1,
        }
        return {
            "metrics": metrics,
            "_latency_samples_us": [symbol],
            "_server_latency_samples_ns": legs,
            "per_symbol": {
                str(symbol): {
                    "metrics": metrics,
                    "_latency_samples_us": [symbol],
                    "_server_latency_samples_ns": legs,
                },
            },
        }

    merged = _merge_partitions(
        [result(10), result(3), result(1)],
        StressConfig(symbols=[10, 3, 1], rate=3, connections=3),
    )
    assert merged["metrics"]["completed"] == 3
    assert set(merged["per_symbol"]) == {"10", "3", "1"}
    for symbol in (10, 3, 1):
        row = merged["per_symbol"][str(symbol)]
        assert row["metrics"]["completed"] == 1
        assert row["latency_us"]["samples"] == 1
        assert row["server_latency_ns"]["matching"]["samples"] == 1


def test_generated_orders_alternate_across_preflighted_touch():
    import json

    from stress_client import StressClient
    from stress_client import StressConfig

    worker = StressClient(0, StressConfig(
        bid_price=49_950,
        ask_price=50_050,
        lot_size=100_000,
    ))
    buy = json.loads(worker.generate_order())["N"]
    sell = json.loads(worker.generate_order())["N"]
    assert buy[1:4] == [0, 50_050, 100_000]
    assert sell[1:4] == [1, 49_950, 100_000]


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


def test_crossing_run_with_unfilled_accepted_order_is_invalid():
    from stress import validate_results

    result = _valid_result()
    result["metrics"].update({
        "fill_attempts": 100,
        "filled_orders": 95,
        "fills": 95,
    })
    failures = validate_results(result, 50)
    assert "accepted crossing orders did not all fill" in failures


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


def test_stress_correlates_server_legs_with_terminal_update():
    from stress_client import StressClient, StressConfig

    client = StressClient(0, StressConfig())
    awaiting = deque(["cid-a"])
    pending = {"cid-a": 1_000_000}
    by_oid = {}
    server_legs = {}
    client._handle_response(
        {"F": ["oid-a", "maker", 100, 2, 3, 0,
               1_000, 0, 3_000, 3_500, 5_000]},
        awaiting, pending, by_oid, server_legs,
    )
    with patch("stress_client.time.perf_counter_ns", return_value=2_000_000):
        client._handle_response(
            {"U": ["oid-a", 0, 2, 0, 0]},
            awaiting, pending, by_oid, server_legs,
        )

    assert client.metrics.server_latencies_ns == {
        "internal": [4000],
        "gateway_to_match_done": [2500],
        "matching": [500],
        "match_done_to_gateway": [1500],
    }
    assert client.metrics.filled_orders == 1
    assert client.metrics.fills == 1


def test_multiple_fills_count_one_filled_order():
    from stress_client import StressClient, StressConfig

    client = StressClient(0, StressConfig())
    awaiting = deque(["cid-a"])
    pending = {"cid-a": 1}
    by_oid = {}
    client._handle_response(
        {"F": ["oid-a", "maker-a", 100, 2, 3, 0]},
        awaiting, pending, by_oid,
    )
    client._handle_response(
        {"F": ["oid-a", "maker-b", 101, 2, 4, 0]},
        awaiting, pending, by_oid,
    )
    assert client.metrics.filled_orders == 1
    assert client.metrics.fills == 2


def test_terminal_update_reports_available_server_legs_without_fill():
    from stress_client import StressClient, StressConfig

    client = StressClient(0, StressConfig())
    awaiting = deque(["cid-a"])
    pending = {"cid-a": 1}
    client._handle_response(
        {"U": ["oid-a", 0, 0, 0, 0, 1_000, 0, 0, 3_500, 5_000]},
        awaiting, pending, {}, {},
    )

    assert client.metrics.server_latencies_ns == {
        "internal": [4000],
        "gateway_to_match_done": [2500],
        "matching": [],
        "match_done_to_gateway": [1500],
    }


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


def test_stress_ignores_lifecycle_update_after_latency_closes():
    from stress_client import StressClient, StressConfig

    client = StressClient(0, StressConfig())
    awaiting = deque(["cid-a"])
    pending = {"cid-a": 1}
    by_oid = {}
    client._handle_response(
        {"U": ["oid-a", 0, 0, 1, 0]}, awaiting, pending, by_oid)
    client._handle_response(
        {"U": ["oid-a", 1, 0, 1, 0]}, awaiting, pending, by_oid)
    assert client.metrics.completed == 1
    assert client.metrics.accepted == 1
    assert client.metrics.errors == 0


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
