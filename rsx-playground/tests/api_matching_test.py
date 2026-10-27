# pyright: basic
"""Matching-engine API tests: /api/matching/{counters,wal,config}.

Run with: cd rsx-playground && uv run pytest tests/api_matching_test.py -v
"""

import json
import struct
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest.mock import AsyncMock, patch

import pytest
from fake_gateway import ScriptedGateway

import order_cli
import rsx_cli
import server
from test_utils import wal_frame
from test_utils import write_wal_stream

ME_COUNTERS = {
    "orders_processed": 12,
    "publishes": 10,
    "drops": 1,
    "dedup_entries": 9,
    "rejects_validation": 1,
    "rejects_reduce_only": 0,
    "rejects_fill_or_kill": 0,
    "rejects_duplicate": 1,
    "rejects_total": 2,
    "wal_last_seq": 57,
    "wal_flushed_seq": 55,
    "checkpoint_wal_seq": 40,
    "config_version": 2,
}


def metrics_body(counters=None, ready=True):
    return {
        "live": True,
        "ready": ready,
        "state": "running",
        "queues": [],
        "counters": [
            {"name": name, "value": value}
            for name, value in (counters or ME_COUNTERS).items()
        ],
    }


@pytest.fixture(autouse=True)
def fresh_cache(monkeypatch):
    monkeypatch.setattr(server, "_TTL_CACHE", {})


@pytest.fixture
def me_targets(monkeypatch):
    targets = [
        ("me-pengu", 10, "127.0.0.1:19810"),
        ("me-sol", 3, "127.0.0.1:19803"),
    ]
    monkeypatch.setattr(server, "_me_health_targets", lambda: targets)

    async def no_refresh():
        return None

    monkeypatch.setattr(server, "refresh_catalog", no_refresh)
    return targets


def pids_from(values):
    """`_running_pid` answering the pids in order."""
    answers = iter(values)

    async def running_pid(name):
        return next(answers)

    return running_pid


@pytest.fixture
def metrics_endpoint():
    """A real HTTP server answering GET /metrics like an ME does."""
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


# ── /api/matching/counters ────────────────────────────────


def test_counters_read_from_a_live_me_metrics_endpoint(
        client, monkeypatch, metrics_endpoint):
    monkeypatch.setattr(
        server, "_me_health_targets",
        lambda: [("me-pengu", 10, metrics_endpoint["addr"])])
    resp = client.get("/api/matching/counters")
    assert resp.status_code == 200
    body = resp.json()
    assert len(body["mes"]) == 1
    me = body["mes"][0]
    assert me["name"] == "me-pengu"
    assert me["symbol_id"] == 10
    assert me["health_addr"] == metrics_endpoint["addr"]
    assert me["error"] == ""
    assert me["live"] is True
    assert me["ready"] is True
    assert me["state"] == "running"
    assert me["counters"] == ME_COUNTERS
    assert body["ts_ns"] > 0


def test_counters_one_me_down_reports_its_reason(
        client, me_targets, monkeypatch):
    def fetch(addr):
        if addr.endswith(":19803"):
            raise ConnectionRefusedError(111, "Connection refused")
        return metrics_body()

    monkeypatch.setattr(server, "_fetch_daemon_metrics", fetch)
    resp = client.get("/api/matching/counters")
    assert resp.status_code == 200
    by_name = {me["name"]: me for me in resp.json()["mes"]}
    assert by_name["me-pengu"]["error"] == ""
    assert by_name["me-pengu"]["counters"]["wal_last_seq"] == 57
    down = by_name["me-sol"]
    assert "127.0.0.1:19803/metrics" in down["error"]
    assert "Connection refused" in down["error"]
    assert down["counters"] == {}
    assert down["ready"] is False


@pytest.mark.allow_5xx
def test_counters_all_me_down_is_503_with_reasons(
        client, me_targets, monkeypatch):
    def fetch(addr):
        raise ConnectionRefusedError(111, "Connection refused")

    monkeypatch.setattr(server, "_fetch_daemon_metrics", fetch)
    resp = client.get("/api/matching/counters")
    assert resp.status_code == 503
    body = resp.json()
    assert "reachable" in body["error"]
    assert len(body["mes"]) == 2
    assert all("Connection refused" in me["error"] for me in body["mes"])


@pytest.mark.parametrize("bad_body", [
    {"live": True, "ready": True, "state": "running"},
    {"live": True, "ready": True, "state": "running",
     "counters": [{"name": "drops", "value": "many"}]},
    {"live": True, "ready": True, "state": "running",
     "counters": [{"value": 3}]},
])
@pytest.mark.allow_5xx
def test_counters_malformed_metrics_surface_as_an_error(
        client, monkeypatch, metrics_endpoint, bad_body):
    metrics_endpoint["body"] = bad_body
    monkeypatch.setattr(
        server, "_me_health_targets",
        lambda: [("me-pengu", 10, metrics_endpoint["addr"])])
    resp = client.get("/api/matching/counters")
    assert resp.status_code == 503
    me = resp.json()["mes"][0]
    assert me["error"]
    assert me["counters"] == {}


@pytest.mark.allow_5xx
def test_counters_not_json_surfaces_as_an_error(
        client, me_targets, monkeypatch):
    def fetch(addr):
        raise json.JSONDecodeError("Expecting value", "<html>", 0)

    monkeypatch.setattr(server, "_fetch_daemon_metrics", fetch)
    resp = client.get("/api/matching/counters")
    assert resp.status_code == 503
    assert "Expecting value" in resp.json()["mes"][0]["error"]


def test_counters_are_cached_between_requests(
        client, monkeypatch, metrics_endpoint):
    monkeypatch.setattr(
        server, "_me_health_targets",
        lambda: [("me-pengu", 10, metrics_endpoint["addr"])])
    for _ in range(5):
        assert client.get("/api/matching/counters").status_code == 200
    assert metrics_endpoint["hits"] == 1


def test_counters_refetch_after_the_cache_expires(
        client, monkeypatch, metrics_endpoint):
    monkeypatch.setattr(
        server, "_me_health_targets",
        lambda: [("me-pengu", 10, metrics_endpoint["addr"])])
    assert client.get("/api/matching/counters").status_code == 200
    key = "me_metrics"
    stamped, value = server._TTL_CACHE[key]
    server._TTL_CACHE[key] = (stamped - 2.0, value)
    assert client.get("/api/matching/counters").status_code == 200
    assert metrics_endpoint["hits"] == 2


def test_me_health_targets_come_from_the_spawn_plan():
    targets = server._me_health_targets()
    assert targets
    for name, symbol_id, addr in targets:
        assert name.startswith("me-")
        assert symbol_id > 0
        assert addr.startswith("127.0.0.1:")


def test_fetch_daemon_counter_stays_best_effort(metrics_endpoint):
    addr = metrics_endpoint["addr"]
    assert server._fetch_daemon_counter(addr, "drops") == 1
    assert server._fetch_daemon_counter(addr, "absent") is None
    assert server._fetch_daemon_counter("127.0.0.1:1", "drops") is None


# ── /api/matching/wal ─────────────────────────────────────


def order_failed(seq, ts_ns, user_id, oid_hi, oid_lo, reason):
    payload = struct.pack(
        "<QQIIQQB", seq, ts_ns, user_id, 0, oid_hi, oid_lo, reason)
    return wal_frame(12, payload, 64)


def order_cancelled(seq, ts_ns, symbol_id, user_id, remaining, reason):
    payload = struct.pack(
        "<QQIIQQqBBBB", seq, ts_ns, symbol_id, user_id,
        0xAA, 0xBB, remaining, reason, 0, 0, 0)
    return wal_frame(3, payload, 64)


def config_applied(seq, ts_ns, symbol_id, version, effective_ms,
                   applied_ns):
    payload = struct.pack(
        "<QQIIQQQ", seq, ts_ns, symbol_id, 0, version,
        effective_ms, applied_ns)
    return wal_frame(5, payload, 64)


def order_done(seq, ts_ns, symbol_id, user_id, filled, remaining):
    payload = struct.pack(
        "<QQIIQQqqBBBB", seq, ts_ns, symbol_id, user_id,
        1, 2, filled, remaining, 0, 0, 0, 0)
    return wal_frame(4, payload, 128)


def write_wal(directory, name, *frames, stream_id=10):
    write_wal_stream(directory, name, frames, stream_id=stream_id)


@pytest.fixture
def me_wal(tmp_path, follow_wal):
    wal_dir = tmp_path / "wal"
    write_wal(
        wal_dir, "pengu",
        order_failed(1, 1_000, 7, 0x11, 0x22, 1),
        order_failed(2, 2_000, 7, 0x11, 0x23, 7),
        order_failed(3, 3_000, 8, 0x11, 0x24, 13),
        order_failed(4, 4_000, 8, 0x11, 0x25, 3),
        order_cancelled(5, 5_000, 10, 7, 100, 0),
        order_cancelled(6, 6_000, 10, 7, 50, 4),
        config_applied(7, 7_000, 10, 2, 1_700_000_000_000, 7_000),
        order_done(8, 8_000, 10, 7, 5, 0),
    )
    write_wal(
        wal_dir, "sol",
        order_cancelled(1, 9_000, 3, 9, 25, 1),
        stream_id=3,
    )
    write_wal(
        wal_dir, "mark",
        order_failed(1, 99_000, 1, 0, 0, 0),
        stream_id=1,
    )
    follow_wal(wal_dir, 10)
    return wal_dir


def wal_records(client, **params):
    resp = client.get("/api/matching/wal", params=params)
    assert resp.status_code == 200, resp.text
    return resp.json()


def test_wal_decodes_order_failed_with_reason_text(client, me_wal):
    body = wal_records(client, type="order_failed", stream="pengu")
    reasons = [(r["seq"], r["reason"], r["reason_text"])
               for r in body["records"]]
    assert reasons == [
        (4, 3, "duplicate order id"),
        (3, 13, "fill-or-kill not fillable"),
        (2, 7, "reduce-only violation"),
        (1, 1, "invalid lot size"),
    ]
    newest = body["records"][0]
    assert newest["type"] == "order_failed"
    assert newest["user_id"] == 8
    assert newest["order_id"] == f"{0x11:016x}{0x25:016x}"
    assert newest["stream"] == "pengu"
    assert newest["ts_ns"] == 4_000


def test_wal_decodes_cancel_reason_wire_code_to_text(client, me_wal):
    body = wal_records(client, type="order_cancelled")
    by_seq = {(r["stream"], r["seq"]): r for r in body["records"]}
    user = by_seq[("pengu", 5)]
    assert (user["reason"], user["reason_text"]) == (0, "user_cancel")
    assert user["remaining_qty"] == 100
    assert user["order_id"] == f"{0xAA:016x}{0xBB:016x}"
    post_only = by_seq[("pengu", 6)]
    assert (post_only["reason"], post_only["reason_text"]) == (
        4, "post_only_reject")
    assert post_only["remaining_qty"] == 50
    reduce_only = by_seq[("sol", 1)]
    assert (reduce_only["reason"], reduce_only["reason_text"]) == (
        1, "reduce_only")
    assert reduce_only["symbol_id"] == 3


@pytest.mark.parametrize("code,text", [
    (0, "user_cancel"), (1, "reduce_only"), (2, "expiry"),
    (3, "system"), (4, "post_only_reject"), (5, "other"),
    (9, "unknown_9"),
])
def test_wal_cancel_reason_table_matches_spec_18(
        client, tmp_path, follow_wal, code, text):
    write_wal(tmp_path, "pengu", order_cancelled(1, 1, 10, 7, 1, code))
    follow_wal(tmp_path, 1)
    record = wal_records(client)["records"][0]
    assert record["reason"] == code
    assert record["reason_text"] == text


def test_wal_unknown_failure_reason_is_named_not_dropped(
        client, tmp_path, follow_wal):
    write_wal(tmp_path, "pengu", order_failed(1, 1, 7, 1, 2, 77))
    follow_wal(tmp_path, 1)
    record = wal_records(client, type="order_failed")["records"][0]
    assert record["reason"] == 77
    assert record["reason_text"] == "unknown_77"


def test_wal_decodes_config_applied(client, me_wal):
    body = wal_records(client, type="config_applied")
    assert body["matched"] == 1
    record = body["records"][0]
    assert record["type"] == "config_applied"
    assert record["symbol_id"] == 10
    assert record["config_version"] == 2
    assert record["effective_at_ms"] == 1_700_000_000_000
    assert record["applied_at_ns"] == 7_000
    assert record["seq"] == 7


def test_wal_without_filter_returns_all_me_streams_newest_first(
        client, me_wal):
    body = wal_records(client)
    assert body["matched"] == 9
    stamps = [r["ts_ns"] for r in body["records"]]
    assert stamps == sorted(stamps, reverse=True)
    assert {r["stream"] for r in body["records"]} == {"pengu", "sol"}


def test_wal_excludes_the_mark_stream(client, me_wal):
    body = wal_records(client, type="order_failed")
    assert body["matched"] == 4
    assert all(r["stream"] != "mark" for r in body["records"])
    resp = client.get("/api/matching/wal", params={"stream": "mark"})
    assert resp.status_code == 404


def test_wal_filters_by_symbol_id(client, me_wal):
    body = wal_records(client, type="order_cancelled", symbol_id=3)
    assert [r["stream"] for r in body["records"]] == ["sol"]


def test_wal_limit_caps_the_page_but_not_the_match_count(
        client, me_wal):
    body = wal_records(client, limit=3)
    assert body["count"] == 3
    assert len(body["records"]) == 3
    assert body["matched"] == 9


def test_wal_unknown_type_is_400_and_lists_the_types(client, me_wal):
    resp = client.get("/api/matching/wal", params={"type": "bogus"})
    assert resp.status_code == 400
    body = resp.json()
    assert "bogus" in body["error"]
    assert "order_failed" in body["types"]
    assert "config_applied" in body["types"]


def test_wal_unknown_stream_is_404(client, me_wal):
    resp = client.get("/api/matching/wal", params={"stream": "nope"})
    assert resp.status_code == 404
    assert "nope" in resp.json()["error"]


@pytest.mark.parametrize("params", [{"limit": 0}, {"limit": 1001}])
def test_wal_rejects_out_of_range_limit(client, me_wal, params):
    resp = client.get("/api/matching/wal", params=params)
    assert resp.status_code == 422


def test_wal_with_no_streams_is_an_empty_list(
        client, tmp_path, monkeypatch):
    monkeypatch.setattr(server, "WAL_DIR", tmp_path / "missing")
    body = wal_records(client)
    assert body == {"records": [], "count": 0, "matched": 0}


def test_wal_timeline_still_lists_the_new_types(client, me_wal):
    resp = client.get("/x/wal-timeline", params={"filter": "order_failed"})
    assert resp.status_code == 200
    assert "ORDER_FAILED" in resp.text


def test_decoded_fill_and_done_are_unchanged(client, me_wal):
    body = wal_records(client, type="order_done")
    record = body["records"][0]
    assert record["filled_qty"] == 5
    assert record["remaining_qty"] == 0
    assert record["symbol_id"] == 10


# ── /api/matching/config ──────────────────────────────────


def now_ms():
    return int(time.time() * 1000)


def applied_row(symbol_id=10, version=2, name="PENGU"):
    return {
        "symbol_id": symbol_id, "symbol_name": name,
        "config_version": version, "effective_at_ms": now_ms() - 5_000,
        "applied_at_ns": 123, "tick_size": 5, "lot_size": 100,
        "price_decimals": 2, "qty_decimals": 3, "status": "active",
    }


def scheduled_row(version, effective_at_ms, symbol_id=10, name="PENGU"):
    return {
        "symbol_id": symbol_id, "symbol_name": name,
        "config_version": version, "effective_at_ms": effective_at_ms,
        "tick_size": 1, "lot_size": 100, "price_decimals": 2,
        "qty_decimals": 3, "status": "active",
        "created_at_ms": now_ms() - 10_000,
    }


def patch_pg(applied, scheduled):
    async def pg_query(sql, *args):
        if "symbol_config_applied" in sql:
            return applied
        assert "symbol_config_schedule" in sql
        return scheduled

    return patch.object(server, "pg_query", AsyncMock(side_effect=pg_query))


def test_config_shows_applied_beside_scheduled_with_states(client):
    scheduled = [
        scheduled_row(1, now_ms() - 60_000),
        scheduled_row(2, now_ms() - 5_000),
        scheduled_row(3, now_ms() - 1_000),
        scheduled_row(4, now_ms() + 3_600_000),
    ]
    with patch_pg([applied_row()], scheduled):
        resp = client.get("/api/matching/config")
    assert resp.status_code == 200
    body = resp.json()
    assert abs(body["now_ms"] - now_ms()) < 60_000
    assert len(body["symbols"]) == 1
    entry = body["symbols"][0]
    assert entry["symbol_id"] == 10
    assert entry["symbol_name"] == "PENGU"
    assert entry["applied"]["config_version"] == 2
    assert entry["applied"]["tick_size"] == 5
    assert entry["applied"]["status"] == "active"
    states = {row["config_version"]: row["state"]
              for row in entry["scheduled"]}
    assert states == {
        1: "superseded", 2: "applied", 3: "due", 4: "pending"}


def test_config_symbol_with_no_applied_row_is_all_due_or_pending(client):
    scheduled = [
        scheduled_row(1, now_ms() - 60_000, symbol_id=3, name="SOL"),
        scheduled_row(2, now_ms() + 60_000, symbol_id=3, name="SOL"),
    ]
    with patch_pg([], scheduled):
        resp = client.get("/api/matching/config")
    assert resp.status_code == 200
    entry = resp.json()["symbols"][0]
    assert entry["applied"] is None
    assert [row["state"] for row in entry["scheduled"]] == [
        "due", "pending"]


def test_config_lists_symbols_in_id_order(client):
    with patch_pg(
            [applied_row(symbol_id=10), applied_row(symbol_id=3, name="SOL")],
            [scheduled_row(1, now_ms(), symbol_id=2, name="ETH")]):
        resp = client.get("/api/matching/config")
    assert [entry["symbol_id"] for entry in resp.json()["symbols"]] == [
        2, 3, 10]


def test_config_exposes_matching_columns_only(client):
    with patch_pg([applied_row()], [scheduled_row(2, now_ms())]):
        resp = client.get("/api/matching/config")
    row = resp.json()["symbols"][0]["scheduled"][0]
    assert "initial_margin_rate_bps" not in row
    assert "maker_fee_bps" not in row
    assert "initial_margin_rate_bps" not in server.ME_APPLIED_SQL
    assert "maker_fee_bps" not in server.ME_SCHEDULED_SQL


@pytest.mark.allow_5xx
def test_config_without_postgres_is_503(client, mock_postgres_down):
    resp = client.get("/api/matching/config")
    assert resp.status_code == 503
    assert resp.json() == {"error": "postgres not connected"}


@pytest.mark.allow_5xx
def test_config_query_error_is_500_with_the_reason(client):
    failure = {"error": 'relation "symbol_config_applied" does not exist'}
    with patch_pg(failure, []):
        resp = client.get("/api/matching/config")
    assert resp.status_code == 500
    assert "symbol_config_applied" in resp.json()["error"]


@pytest.mark.allow_5xx
def test_config_schedule_query_error_is_500_with_the_reason(client):
    with patch_pg([applied_row()], {"error": "connection reset"}):
        resp = client.get("/api/matching/config")
    assert resp.status_code == 500
    assert "connection reset" in resp.json()["error"]


def test_config_postgres_is_read_once_per_cache_window(client):
    with patch_pg([applied_row()], [scheduled_row(2, now_ms())]) as pg:
        for _ in range(4):
            assert client.get("/api/matching/config").status_code == 200
    assert pg.call_count == 2


def test_classify_scheduled_follows_the_me_poll_rule():
    rows = [
        scheduled_row(1, 100), scheduled_row(2, 100),
        scheduled_row(3, 200), scheduled_row(4, 201),
    ]
    states = [
        row["state"] for row in server.classify_scheduled(2, rows, 200)]
    assert states == ["superseded", "applied", "due", "pending"]


# ── Matching tab: partials ────────────────────────────────


@pytest.fixture
def me_down_and_up(me_targets, monkeypatch):
    def fetch(addr):
        if addr.endswith(":19803"):
            raise ConnectionRefusedError(111, "Connection refused")
        return metrics_body()

    monkeypatch.setattr(server, "_fetch_daemon_metrics", fetch)


def test_matching_counters_partial_shows_each_me(client, me_down_and_up):
    resp = client.get("/x/matching-counters")
    assert resp.status_code == 200
    text = resp.text
    assert "me-pengu" in text
    assert ">12<" in text
    assert ">10<" in text
    assert "me-sol" in text
    assert "127.0.0.1:19803/metrics" in text
    assert "Connection refused" in text


def test_matching_recovery_partial_shows_tip_checkpoint_config(
        client, me_down_and_up):
    text = client.get("/x/matching-recovery").text
    assert ">57<" in text
    assert ">55<" in text
    assert ">40<" in text
    assert ">15<" in text
    assert ">2<" in text
    assert 'hx-post="./api/matching/crash/me-pengu"' in text
    assert "api/matching/crash/me-sol" not in text


def test_matching_recovery_partial_names_no_checkpoint_and_env_config(
        client, me_targets, monkeypatch):
    counters = {**ME_COUNTERS, "checkpoint_wal_seq": 0, "config_version": 0}
    monkeypatch.setattr(
        server, "_fetch_daemon_metrics",
        lambda addr: metrics_body(counters))
    text = client.get("/x/matching-recovery").text
    assert ">none<" in text
    assert ">env<" in text


def test_matching_rejects_partial_lists_failures_with_wire_reasons(
        client, me_wal):
    text = client.get("/x/matching-rejects").text
    assert "duplicate order id" in text
    assert "fill-or-kill not fillable" in text
    assert "reduce-only violation" in text
    assert "invalid lot size" in text
    assert "post_only_reject" in text
    assert "reduce_only" in text
    assert f"{0x11:016x}{0x25:016x}" in text


def test_matching_rejects_partial_leaves_out_user_cancels(
        client, me_wal):
    text = client.get("/x/matching-rejects").text
    assert "user_cancel" not in text
    assert text.count(">cancelled<") == 2


def test_matching_rejects_partial_without_records(
        client, tmp_path, follow_wal):
    follow_wal(tmp_path, 0)
    assert "no rejects or engine cancels" in client.get(
        "/x/matching-rejects").text


def test_matching_page_has_every_card_and_the_nav_tab(client):
    text = client.get("/matching").text
    for card in (
            "Matching Engines", "Order Entry", "Recent Orders",
            "Duplicate Order Id", "Recovery",
            "Rejects and Engine Cancels"):
        assert card in text
    assert 'href="./matching"' in text
    assert 'hx-get="./x/matching-counters"' in text


@pytest.mark.parametrize("path", ["/matching", "/orders"])
def test_order_form_offers_fok_and_post_only(client, path):
    text = client.get(path).text
    assert 'name="tif" value="FOK"' in text
    assert 'name="post_only"' in text


# ── Order entry: FOK and post-only reach the wire ─────────


def test_fok_and_post_only_reach_the_order_call(client, monkeypatch):
    gateway = ScriptedGateway([order_outcome("resting")])
    monkeypatch.setattr(server, "ORDER_GATEWAY", gateway)

    async def no_error(symbol_id):
        return None

    monkeypatch.setattr(server, "_execution_venue_error", no_error)
    server.recent_orders.clear()
    client.post("/api/orders/test", data={
        "symbol_id": "10", "side": "buy", "price": "0.000001",
        "qty": "1000", "tif": "FOK", "post_only": "on"})
    order = gateway.placed[0]
    assert order.tif == "FOK"
    assert order.post_only is True
    assert order.reduce_only is False
    assert server.recent_orders[-1]["tif"] == "FOK"
    assert server.recent_orders[-1]["post_only"] is True
    server.recent_orders.clear()


# ── Cancel button: a resting order is cancelled at the gateway ──


@pytest.fixture
def resting_order():
    order = {
        "cid": "pgrest0000000000001", "user_id": 7, "symbol": "10",
        "side": "buy", "price": "0.000001", "qty": "10",
        "tif": "GTC", "status": "resting",
    }
    server.recent_orders.append(order)
    yield order
    server.recent_orders.remove(order)


def order_outcome(status, reason=None):
    return order_cli.Outcome(
        cid="abc123", status=status, reason=reason, order_id="a" * 32,
        rtt_us=120, frames=[])


def gateway_answers(monkeypatch, scripted):
    gateway = ScriptedGateway(scripted)
    monkeypatch.setattr(server, "ORDER_GATEWAY", gateway)
    return gateway


def test_recent_orders_offers_cancel_for_resting_orders(
        client, resting_order):
    text = client.get("/x/recent-orders").text
    assert f'hx-post="./api/orders/{resting_order["cid"]}/cancel"' in text


def test_cancel_resting_order_goes_to_the_gateway_as_its_user(
        client, resting_order, monkeypatch):
    gateway = gateway_answers(monkeypatch, [order_outcome("cancelled")])
    resp = client.post(f"/api/orders/{resting_order['cid']}/cancel")
    assert gateway.cancelled == [(7, resting_order["cid"])]
    assert "cancelled" in resp.text
    assert resting_order["status"] == "cancelled"


def test_cancel_refused_by_the_gateway_keeps_the_order_resting(
        client, resting_order, monkeypatch):
    gateway_answers(
        monkeypatch, [order_outcome("rejected", "order not found")])
    resp = client.post(f"/api/orders/{resting_order['cid']}/cancel")
    assert "not cancelled" in resp.text
    assert "order not found" in resp.text
    assert resting_order["status"] == "resting"


def test_cancel_without_a_gateway_answer_says_so(
        client, resting_order, monkeypatch):
    gateway_answers(monkeypatch, [order_outcome("timeout")])
    resp = client.post(f"/api/orders/{resting_order['cid']}/cancel")
    assert "cancel timeout" in resp.text
    assert resting_order["status"] == "resting"


def test_cancel_with_gateway_down_surfaces_the_error(
        client, resting_order, monkeypatch):
    gateway_answers(
        monkeypatch, [rsx_cli.CliError("connect failed: refused", 1)])
    resp = client.post(f"/api/orders/{resting_order['cid']}/cancel")
    assert "cancel error: connect failed: refused" in resp.text
    assert resting_order["status"] == "resting"


def test_filled_order_is_not_cancellable(client, resting_order):
    resting_order["status"] = "filled"
    resp = client.post(f"/api/orders/{resting_order['cid']}/cancel")
    assert "not found or not cancellable" in resp.text


# ── Recovery card: crash one ME and compare before / after ──


def test_crash_unknown_me_is_404(client, me_targets):
    resp = client.post("/api/matching/crash/me-nope")
    assert resp.status_code == 404
    assert "me-nope" in resp.json()["error"]


def test_crash_of_a_stopped_me_says_not_running(
        client, me_targets, monkeypatch):
    monkeypatch.setattr(server, "_running_pid", pids_from([None]))
    resp = client.post("/api/matching/crash/me-pengu")
    assert "me-pengu not running" in resp.text


def test_crash_reports_tip_and_checkpoint_before_and_after(
        client, me_targets, monkeypatch):
    monkeypatch.setattr(
        server, "_running_pid", pids_from([100, 100, 200]))
    killed = []
    monkeypatch.setattr(
        server.os, "kill", lambda pid, sig: killed.append((pid, sig)))
    states = iter([
        ME_COUNTERS,
        {**ME_COUNTERS, "wal_last_seq": 61, "checkpoint_wal_seq": 55},
    ])
    monkeypatch.setattr(
        server, "_fetch_daemon_metrics",
        lambda addr: metrics_body(next(states)))
    resp = client.post("/api/matching/crash/me-pengu")
    assert killed == [(100, server.signal.SIGKILL)]
    assert "me-pengu is back" in resp.text
    assert "before crash" in resp.text
    assert ">57<" in resp.text and ">40<" in resp.text
    assert "after recovery" in resp.text
    assert ">61<" in resp.text and ">55<" in resp.text


def test_crash_that_loses_the_unflushed_tail_says_so(
        client, me_targets, monkeypatch):
    monkeypatch.setattr(
        server, "_running_pid", pids_from([100, 100, 200]))
    monkeypatch.setattr(server.os, "kill", lambda pid, sig: None)
    states = iter([
        {**ME_COUNTERS, "wal_last_seq": 70, "wal_flushed_seq": 60},
        {**ME_COUNTERS, "wal_last_seq": 61, "wal_flushed_seq": 61},
    ])
    monkeypatch.setattr(
        server, "_fetch_daemon_metrics",
        lambda addr: metrics_body(next(states)))
    resp = client.post("/api/matching/crash/me-pengu")
    assert "WAL tip went back from 70 to 61" in resp.text
    assert "everything flushed came back" in resp.text


def test_crash_that_keeps_the_tail_does_not_claim_a_loss(
        client, me_targets, monkeypatch):
    monkeypatch.setattr(
        server, "_running_pid", pids_from([100, 100, 200]))
    monkeypatch.setattr(server.os, "kill", lambda pid, sig: None)
    states = iter([
        ME_COUNTERS, {**ME_COUNTERS, "wal_last_seq": 58}])
    monkeypatch.setattr(
        server, "_fetch_daemon_metrics",
        lambda addr: metrics_body(next(states)))
    resp = client.post("/api/matching/crash/me-pengu")
    assert "me-pengu is back" in resp.text
    assert "WAL tip went back" not in resp.text


def test_crash_waits_for_the_first_flush_before_reporting(
        client, me_targets, monkeypatch):
    monkeypatch.setattr(
        server, "_running_pid", pids_from([100, 100, 200, 200]))
    monkeypatch.setattr(server.os, "kill", lambda pid, sig: None)
    states = iter([
        ME_COUNTERS,
        {**ME_COUNTERS, "wal_flushed_seq": 0},
        {**ME_COUNTERS, "wal_flushed_seq": 58, "wal_last_seq": 58},
    ])
    monkeypatch.setattr(
        server, "_fetch_daemon_metrics",
        lambda addr: metrics_body(next(states)))
    resp = client.post("/api/matching/crash/me-pengu")
    assert "me-pengu is back" in resp.text
    assert ">58<" in resp.text


def test_crash_without_a_restart_fails_loudly(
        client, me_targets, monkeypatch):
    async def always_running(name):
        return 100

    monkeypatch.setattr(server, "_running_pid", always_running)
    monkeypatch.setattr(server.os, "kill", lambda pid, sig: None)
    monkeypatch.setattr(
        server, "_fetch_daemon_metrics", lambda addr: metrics_body())
    monkeypatch.setattr(server, "CRASH_RECOVERY_TIMEOUT_S", 0.3)
    resp = client.post("/api/matching/crash/me-pengu")
    assert "Failed: me-pengu did not come back" in resp.text


# ── Duplicate demo: scratch ME, real binary ───────────────


def order_accepted(seq, ts_ns, user_id, symbol_id, oid_hi, oid_lo):
    payload = struct.pack(
        "<QQIIQQqqBBBB12s", seq, ts_ns, user_id, symbol_id,
        oid_hi, oid_lo, 100, 1, 1, 0, 0, 0, b"\0" * 12)
    return wal_frame(7, payload, 128)


def scratch_wal(tmp_path, order_id_lo, reject=True):
    wal_dir = tmp_path / "wal"
    frames = [order_accepted(1, 1_000, 1, 999, 0, order_id_lo)]
    if reject:
        frames.append(order_failed(2, 2_000, 1, 0, order_id_lo, 3))
    write_wal(wal_dir, "", *frames, stream_id=999)
    return wal_dir


def test_duplicate_demo_shows_the_second_copy_rejected(
        client, tmp_path, monkeypatch):
    seen = {}

    def run(binary, workdir, order_id_lo):
        seen.update(binary=binary, workdir=workdir, lo=order_id_lo)
        return scratch_wal(tmp_path, order_id_lo)

    monkeypatch.setattr(server.me_demo, "run_duplicate_demo", run)
    resp = client.post("/api/matching/duplicate-demo")
    order_id = f"{0:016x}{seen['lo']:016x}"
    assert seen["binary"].name == "rsx-matching"
    assert seen["workdir"] == server.TMP / "me-demo"
    assert f"second copy of {order_id} rejected: duplicate order id" in (
        resp.text)
    assert "accepted: logged before matching" in resp.text
    assert "rejected: duplicate order id" in resp.text


def test_duplicate_demo_without_a_rejection_fails_loudly(
        client, tmp_path, monkeypatch):
    monkeypatch.setattr(
        server.me_demo, "run_duplicate_demo",
        lambda binary, workdir, lo: scratch_wal(
            tmp_path, lo, reject=False))
    resp = client.post("/api/matching/duplicate-demo")
    assert "did not reject the second copy" in resp.text


def test_duplicate_demo_missing_binary_surfaces_the_error(
        client, monkeypatch):
    def run(binary, workdir, order_id_lo):
        raise FileNotFoundError(f"binary not found: {binary}")

    monkeypatch.setattr(server.me_demo, "run_duplicate_demo", run)
    resp = client.post("/api/matching/duplicate-demo")
    assert "Failed:" in resp.text
    assert "binary not found" in resp.text


def test_duplicate_demo_engine_log_is_shown_on_failure(
        client, monkeypatch):
    def run(binary, workdir, order_id_lo):
        raise RuntimeError("scratch matching engine: not ready\nboom")

    monkeypatch.setattr(server.me_demo, "run_duplicate_demo", run)
    resp = client.post("/api/matching/duplicate-demo")
    assert "not ready" in resp.text
    assert "boom" in resp.text


def test_order_frame_is_a_valid_casting_frame():
    frame = server.me_demo.build_order_frame(
        seq=5, user_id=3, order_id_lo=9, side=1, price=100, qty=2)
    version, _, rtype, length, _, crc, _ = struct.unpack_from(
        "<BBHHHI4s", frame)
    payload = frame[16:]
    assert (version, rtype, length) == (1, 9, 128)
    assert len(payload) == 128
    assert crc == server.me_demo.crc32c(payload)
    seq, price, qty, side = struct.unpack_from("<QqqB", payload)
    assert (seq, price, qty, side) == (5, 100, 2, 1)
    user_id, = struct.unpack_from("<I", payload, 32)
    order_id_lo, = struct.unpack_from("<Q", payload, 56)
    assert (user_id, order_id_lo) == (3, 9)


def test_crc32c_matches_the_castagnoli_check_value():
    assert server.me_demo.crc32c(b"123456789") == 0xE3069283
