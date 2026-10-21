# pyright: basic
"""Gateway API tests: /api/gateway/metrics.

Run with: cd rsx-playground && uv run pytest tests/api_gateway_test.py -v
"""

import json
import re
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

import server

REJECT_COUNTERS = {
    f"rejects_{name.lower()}": 0
    for name in server.FAILURE_REASON_NAMES.values()
}

GATEWAY_COUNTERS = {
    "connections": 3,
    "orders_processed": 41,
    **REJECT_COUNTERS,
    "rejects_rate_limit": 4,
    "rejects_duplicate_order_id": 2,
    "rejects_total": 6,
    "circuit_state": 0,
}


def metrics_body(counters=None, pending=(5, 10000), ready=True):
    return {
        "live": True,
        "ready": ready,
        "saturation": pending[0] / pending[1],
        "state": "running",
        "queues": [
            {"name": "pending", "used": pending[0], "cap": pending[1]},
        ],
        "counters": [
            {"name": name, "value": value}
            for name, value in (counters or GATEWAY_COUNTERS).items()
        ],
    }


@pytest.fixture(autouse=True)
def fresh_cache(monkeypatch):
    monkeypatch.setattr(server, "_TTL_CACHE", {})


@pytest.fixture
def gateway_targets(monkeypatch):
    targets = [
        ("gw-0", "127.0.0.1:19900"),
        ("gw-1", "127.0.0.1:19901"),
    ]
    monkeypatch.setattr(server, "_gateway_health_targets", lambda: targets)
    return targets


@pytest.fixture
def metrics_endpoint():
    """A real HTTP server answering GET /metrics like a gateway does."""
    served = {"body": metrics_body(), "hits": 0}

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            served["hits"] += 1
            payload = json.dumps(served["body"]).encode()
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, *args):
            pass

    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    served["addr"] = f"127.0.0.1:{httpd.server_address[1]}"
    yield served
    httpd.shutdown()
    httpd.server_close()
    thread.join()


def one_gateway(monkeypatch, endpoint):
    monkeypatch.setattr(
        server, "_gateway_health_targets",
        lambda: [("gw-0", endpoint["addr"])])


# ── /api/gateway/metrics ──────────────────────────────────


def test_metrics_read_from_a_live_gateway_endpoint(
        client, monkeypatch, metrics_endpoint):
    one_gateway(monkeypatch, metrics_endpoint)
    resp = client.get("/api/gateway/metrics")
    assert resp.status_code == 200
    body = resp.json()
    assert len(body["gateways"]) == 1
    gateway = body["gateways"][0]
    assert gateway["name"] == "gw-0"
    assert gateway["health_addr"] == metrics_endpoint["addr"]
    assert gateway["error"] == ""
    assert gateway["live"] is True
    assert gateway["ready"] is True
    assert gateway["state"] == "running"
    assert gateway["pending"] == {"used": 5, "cap": 10000}
    assert gateway["circuit"] == "closed"
    assert gateway["counters"] == GATEWAY_COUNTERS
    assert body["ts_ns"] > 0


def test_rejects_carry_the_spec_18_names_and_codes(
        client, monkeypatch, metrics_endpoint):
    one_gateway(monkeypatch, metrics_endpoint)
    rejects = client.get("/api/gateway/metrics").json()[
        "gateways"][0]["rejects"]
    assert [reject["code"] for reject in rejects] == list(range(14))
    by_name = {reject["name"]: reject for reject in rejects}
    assert by_name["INVALID_TICK_SIZE"]["code"] == 0
    assert by_name["DUPLICATE_ORDER_ID"] == {
        "code": 3, "name": "DUPLICATE_ORDER_ID", "count": 2}
    assert by_name["RATE_LIMIT"]["count"] == 4
    assert by_name["FILL_OR_KILL"]["code"] == 13
    assert sum(reject["count"] for reject in rejects) == 6


def test_reason_names_cover_the_playground_reason_codes():
    assert server.FAILURE_REASON_NAMES.keys() == server.REJECT_REASONS.keys()


def test_reject_counter_names_match_the_gateway_source():
    source = (Path(__file__).parent.parent.parent
              / "rsx-gateway" / "src" / "metrics.rs").read_text()
    array = re.search(
        r"const REJECT_NAMES:[^=]*=\s*\[(.*?)\];", source, re.DOTALL)
    assert array
    names = re.findall(r'"(rejects_[a-z_]+)"', array.group(1))
    assert names[:-1] == [
        f"rejects_{name.lower()}"
        for name in server.FAILURE_REASON_NAMES.values()]
    assert names[-1] == "rejects_other"


@pytest.mark.parametrize("state,label", [
    (0, "closed"), (1, "open"), (2, "half-open"), (9, "unknown_9"),
])
def test_circuit_state_is_named(
        client, monkeypatch, metrics_endpoint, state, label):
    metrics_endpoint["body"] = metrics_body(
        {**GATEWAY_COUNTERS, "circuit_state": state})
    one_gateway(monkeypatch, metrics_endpoint)
    gateway = client.get("/api/gateway/metrics").json()["gateways"][0]
    assert gateway["circuit"] == label


def test_not_ready_gateway_is_reported_not_ready(
        client, monkeypatch, metrics_endpoint):
    metrics_endpoint["body"] = metrics_body(
        pending=(9500, 10000), ready=False)
    one_gateway(monkeypatch, metrics_endpoint)
    gateway = client.get("/api/gateway/metrics").json()["gateways"][0]
    assert gateway["ready"] is False
    assert gateway["pending"] == {"used": 9500, "cap": 10000}
    assert gateway["error"] == ""


def test_one_gateway_down_reports_its_reason(
        client, gateway_targets, monkeypatch):
    def fetch(addr):
        if addr.endswith(":19901"):
            raise ConnectionRefusedError(111, "Connection refused")
        return metrics_body()

    monkeypatch.setattr(server, "_fetch_daemon_metrics", fetch)
    resp = client.get("/api/gateway/metrics")
    assert resp.status_code == 200
    by_name = {gw["name"]: gw for gw in resp.json()["gateways"]}
    assert by_name["gw-0"]["error"] == ""
    assert by_name["gw-0"]["counters"]["orders_processed"] == 41
    down = by_name["gw-1"]
    assert "127.0.0.1:19901/metrics" in down["error"]
    assert "Connection refused" in down["error"]
    assert down["counters"] == {}
    assert down["rejects"] == []
    assert down["ready"] is False


@pytest.mark.allow_5xx
def test_all_gateways_down_is_503_with_reasons(
        client, gateway_targets, monkeypatch):
    def fetch(addr):
        raise ConnectionRefusedError(111, "Connection refused")

    monkeypatch.setattr(server, "_fetch_daemon_metrics", fetch)
    resp = client.get("/api/gateway/metrics")
    assert resp.status_code == 503
    body = resp.json()
    assert "reachable" in body["error"]
    assert len(body["gateways"]) == 2
    assert all(
        "Connection refused" in gw["error"] for gw in body["gateways"])


def cut(counters, name):
    return {k: v for k, v in counters.items() if k != name}


@pytest.mark.parametrize("bad_body", [
    {"live": True, "ready": True, "state": "running", "queues": []},
    {**metrics_body(), "queues": []},
    {**metrics_body(), "queues": [{"name": "pending", "used": "x",
                                   "cap": 1}]},
    metrics_body(cut(GATEWAY_COUNTERS, "circuit_state")),
    metrics_body(cut(GATEWAY_COUNTERS, "rejects_rate_limit")),
    metrics_body({**GATEWAY_COUNTERS, "connections": "many"}),
])
@pytest.mark.allow_5xx
def test_malformed_metrics_surface_as_an_error(
        client, monkeypatch, metrics_endpoint, bad_body):
    metrics_endpoint["body"] = bad_body
    one_gateway(monkeypatch, metrics_endpoint)
    resp = client.get("/api/gateway/metrics")
    assert resp.status_code == 503
    gateway = resp.json()["gateways"][0]
    assert gateway["error"]
    assert gateway["counters"] == {}


@pytest.mark.allow_5xx
def test_metrics_not_json_surfaces_as_an_error(
        client, gateway_targets, monkeypatch):
    def fetch(addr):
        raise json.JSONDecodeError("Expecting value", "<html>", 0)

    monkeypatch.setattr(server, "_fetch_daemon_metrics", fetch)
    resp = client.get("/api/gateway/metrics")
    assert resp.status_code == 503
    assert "Expecting value" in resp.json()["gateways"][0]["error"]


def test_metrics_are_cached_between_requests(
        client, monkeypatch, metrics_endpoint):
    one_gateway(monkeypatch, metrics_endpoint)
    for _ in range(5):
        assert client.get("/api/gateway/metrics").status_code == 200
    assert metrics_endpoint["hits"] == 1


def test_metrics_refetch_after_the_cache_expires(
        client, monkeypatch, metrics_endpoint):
    one_gateway(monkeypatch, metrics_endpoint)
    assert client.get("/api/gateway/metrics").status_code == 200
    stamped, value = server._TTL_CACHE["gateway_metrics"]
    server._TTL_CACHE["gateway_metrics"] = (stamped - 2.0, value)
    assert client.get("/api/gateway/metrics").status_code == 200
    assert metrics_endpoint["hits"] == 2


def test_gateway_health_targets_come_from_the_spawn_plan():
    targets = server._gateway_health_targets()
    assert targets
    for name, addr in targets:
        assert name.startswith("gw-")
        assert addr.startswith("127.0.0.1:")


# ── E frames keep their code ──────────────────────────────


def test_an_e_frame_keeps_its_code_in_the_order_note():
    assert server._classify_order_response(
        {"E": [1006, "rate limited"]}) == (
        "rejected", "E[1006] rate limited")
    assert server._classify_order_response(
        {"E": [5, "overloaded"]}) == ("rejected", "E[5] overloaded")
