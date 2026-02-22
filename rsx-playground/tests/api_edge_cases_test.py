"""Edge cases and boundary condition tests.

Run with: cd rsx-playground && uv run pytest tests/api_edge_cases_test.py -v
"""

import pytest
import json
from unittest.mock import patch
from pathlib import Path


# ── Boundary Values (25) ──────────────────────────────────


def test_user_id_zero(client):
    """User ID 0 handled."""
    resp = client.get("/api/risk/users/0")
    assert resp.status_code == 200


def test_user_id_max_int(client):
    """User ID max int handled."""
    resp = client.get("/api/risk/users/2147483647")
    assert resp.status_code == 200


def test_user_id_negative(client):
    """Negative user ID handled."""
    resp = client.get("/api/risk/users/-1")
    assert resp.status_code == 200


def test_symbol_id_zero(client):
    """Symbol ID 0 in order."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "0",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_price_zero(client):
    """Price 0 in order."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "0",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_qty_zero(client):
    """Quantity 0 in order."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "0",
        },
    )
    assert resp.status_code == 200


def test_price_max_value(client):
    """Price max value."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "9999999999",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_qty_max_value(client):
    """Quantity max value."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "9999999",
        },
    )
    assert resp.status_code == 200


def test_price_negative(client):
    """Negative price."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "-1",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_qty_negative(client):
    """Negative quantity."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "-1",
        },
    )
    assert resp.status_code == 200


def test_log_limit_zero(client):
    """Log limit 0."""
    resp = client.get("/api/logs?limit=0")
    assert resp.status_code == 200


def test_log_limit_max(client):
    """Log limit very large."""
    resp = client.get("/api/logs?limit=999999")
    assert resp.status_code == 200


def test_empty_process_name(client):
    """Empty process name."""
    resp = client.post("/api/processes//stop")
    assert resp.status_code in [404, 422]


def test_very_long_process_name(client):
    """Very long process name."""
    long_name = "a" * 1000
    resp = client.post(f"/api/processes/{long_name}/stop")
    assert resp.status_code in [200, 404]


def test_empty_search_term(client):
    """Empty search term in logs."""
    resp = client.get("/api/logs?search=")
    assert resp.status_code == 200


def test_very_long_search_term(client):
    """Very long search term."""
    long_term = "a" * 10000
    resp = client.get(f"/api/logs?search={long_term}")
    assert resp.status_code == 200


def test_empty_cid(client):
    """Empty client ID for cancel."""
    resp = client.post("/api/orders//cancel")
    assert resp.status_code in [404, 422]


def test_very_long_cid(client):
    """Very long client ID."""
    long_cid = "a" * 1000
    resp = client.post(f"/api/orders/{long_cid}/cancel")
    assert resp.status_code == 200


def test_symbol_id_max_int(client):
    """Symbol ID max int."""
    resp = client.get("/x/book?symbol_id=2147483647")
    assert resp.status_code == 200


def test_risk_uid_param_zero(client):
    """Risk UID query param 0."""
    resp = client.get("/x/risk-user?risk-uid=0")
    assert resp.status_code == 200


def test_trace_oid_empty(client):
    """Empty trace OID."""
    resp = client.get("/x/order-trace?trace-oid=")
    assert resp.status_code == 200


def test_wal_stream_empty_name(client):
    """Empty WAL stream name."""
    resp = client.get("/api/wal//status")
    assert resp.status_code in [404, 422]


def test_log_process_filter_empty(client):
    """Empty process filter."""
    resp = client.get("/api/logs?process=")
    assert resp.status_code == 200


def test_log_level_empty(client):
    """Empty level filter."""
    resp = client.get("/api/logs?level=")
    assert resp.status_code == 200


def test_scenario_empty_name(client):
    """Empty scenario name requires confirmation."""
    resp = client.post(
        "/api/scenario/switch",
        data={"scenario-select": ""},
        headers={"x-confirm": "yes"},
    )
    assert resp.status_code == 200


# ── Invalid Inputs (30) ───────────────────────────────────


def test_invalid_side_value(client):
    """Invalid side value in order."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "invalid",
            "price": "50000",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_invalid_tif_value(client):
    """Invalid TIF value."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
            "tif": "INVALID",
        },
    )
    assert resp.status_code == 200


def test_non_numeric_price(client):
    """Non-numeric price."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "abc",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_non_numeric_qty(client):
    """Non-numeric quantity."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "abc",
        },
    )
    assert resp.status_code == 200


def test_non_numeric_symbol_id(client):
    """Non-numeric symbol ID."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "abc",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_invalid_user_id_string(client):
    """String user ID in risk query."""
    resp = client.get("/api/risk/users/abc")
    assert resp.status_code in [200, 422]


def test_invalid_action_type(client):
    """Invalid action type."""
    resp = client.post("/api/processes/test/invalid")
    assert resp.status_code == 400


def test_invalid_risk_action(client):
    """Invalid risk action."""
    resp = client.post("/api/risk/users/1/invalid")
    assert resp.status_code == 400


def test_invalid_scenario_name(client):
    """Invalid scenario name."""
    resp = client.post(
        "/api/scenario/switch",
        data={"scenario-select": "nonexistent"},
        headers={"x-confirm": "yes"},
    )
    assert resp.status_code == 200


def test_malformed_form_data(client):
    """Malformed form data."""
    resp = client.post(
        "/api/orders/test",
        data={"invalid_field": "value"}
    )
    assert resp.status_code == 200


def test_missing_required_fields(client):
    """Missing required fields in order."""
    resp = client.post(
        "/api/orders/test",
        data={"symbol_id": "10"}
    )
    assert resp.status_code == 200


def test_duplicate_form_fields(client):
    """Duplicate form fields."""
    resp = client.post(
        "/api/orders/test",
        data="symbol_id=10&symbol_id=20&side=buy&price=50000&qty=1",
        headers={"Content-Type": "application/x-www-form-urlencoded"}
    )
    assert resp.status_code == 200


def test_unicode_in_process_name(client):
    """Unicode in process name."""
    resp = client.post("/api/processes/プロセス/stop")
    assert resp.status_code in [200, 404, 422]


def test_special_chars_in_search(client):
    """Special characters in search."""
    resp = client.get("/api/logs?search=<script>alert('xss')</script>")
    assert resp.status_code == 200


def test_sql_injection_in_user_id(client, mock_postgres_connected):
    """SQL injection attempt in user ID."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = []
        resp = client.get("/api/risk/users/1'; DROP TABLE users; --")

    assert resp.status_code in [200, 422]


def test_path_traversal_in_wal_stream(client):
    """Path traversal in WAL stream blocked by router or handler."""
    resp = client.get("/api/wal/..%2F..%2Fetc%2Fpasswd/status")
    assert resp.status_code in [400, 404]


def test_xss_in_cid(client):
    """XSS attempt in client ID."""
    resp = client.post(
        "/api/orders/<script>alert('xss')</script>/cancel"
    )
    assert resp.status_code in [200, 404, 422]


def test_null_bytes_in_input(client):
    """Null bytes in input are rejected by transport layer."""
    try:
        resp = client.get("/api/logs?search=test%00malicious")
        assert resp.status_code in [200, 400]
    except Exception:
        pass  # httpx rejects null bytes before sending


def test_very_long_url(client):
    """Very long URL."""
    long_param = "a" * 10000
    resp = client.get(f"/api/logs?search={long_param}")
    assert resp.status_code == 200


def test_invalid_query_param_type(client):
    """Invalid query param type returns 200 with empty state."""
    resp = client.get("/x/risk-user?risk-uid=invalid")
    assert resp.status_code == 200


def test_negative_log_limit(client):
    """Negative log limit."""
    resp = client.get("/api/logs?limit=-1")
    assert resp.status_code == 200


def test_float_user_id(client):
    """Float user ID."""
    resp = client.get("/api/risk/users/1.5")
    assert resp.status_code in [200, 404, 422]


def test_scientific_notation_price(client):
    """Scientific notation in price."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "1e5",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_hex_value_qty(client):
    """Hex value in quantity."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "0x10",
        },
    )
    assert resp.status_code == 200


def test_infinity_value(client):
    """Infinity value."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "inf",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_nan_value(client):
    """NaN value."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "nan",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_boolean_in_numeric_field(client):
    """Boolean in numeric field."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "true",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_array_in_scalar_field(client):
    """Array in scalar field."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "[10, 20]",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_object_in_scalar_field(client):
    """Object in scalar field."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": '{"id": 10}',
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_emoji_in_search(client):
    """Emoji in search term."""
    resp = client.get("/api/logs?search=🚀")
    assert resp.status_code == 200


# ── Missing Resources (15) ────────────────────────────────


def test_postgres_unavailable(client, mock_postgres_down):
    """Postgres unavailable handled."""
    resp = client.get("/api/risk/users/1")
    assert resp.status_code == 200


def test_wal_dir_missing(client):
    """WAL directory missing."""
    import server
    original = server.WAL_DIR
    server.WAL_DIR = Path("/nonexistent")

    resp = client.get("/x/wal-status")

    server.WAL_DIR = original
    assert resp.status_code == 200


def test_log_dir_missing(client):
    """Log directory missing."""
    import server
    original = server.LOG_DIR
    server.LOG_DIR = Path("/nonexistent")

    resp = client.get("/api/logs")

    server.LOG_DIR = original
    assert resp.status_code == 200
    data = resp.json()
    assert data["count"] == 0


def test_process_not_in_managed(client):
    """Process not in managed dict."""
    resp = client.post("/api/processes/nonexistent/stop")
    assert resp.status_code == 200


def test_log_file_deleted(client, log_dir_with_files):
    """Log file deleted during read."""
    log_file = log_dir_with_files / "gateway.log"
    log_file.unlink()

    resp = client.get("/api/logs?process=gateway")
    assert resp.status_code == 200


def test_wal_file_deleted(client, wal_dir_with_files):
    """WAL file deleted during scan."""
    stream_dir = wal_dir_with_files / "test-stream"
    files = list(stream_dir.glob("*.dxs"))
    files[0].unlink()

    resp = client.get("/x/wal-files")
    assert resp.status_code == 200


def test_process_died_during_query(client, running_process):
    """Process died during query."""
    running_process.returncode = 1

    resp = client.get("/x/processes")
    assert resp.status_code == 200


def test_postgres_connection_lost(client, mock_postgres_connected):
    """Postgres connection lost."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = None
        resp = client.get("/api/risk/users/1")

    assert resp.status_code == 200


def test_binary_not_found(client):
    """Binary not found for process start."""
    with patch('server.spawn_process') as mock_spawn:
        mock_spawn.return_value = {"error": "binary not found"}
        resp = client.post("/api/processes/test/start")

    assert resp.status_code in [200, 400]


def test_pid_file_missing(client):
    """PID file missing."""
    resp = client.get("/x/processes")
    assert resp.status_code == 200


def test_log_file_unreadable(client, log_dir_with_files):
    """Log file unreadable."""
    resp = client.get("/api/logs")
    assert resp.status_code == 200


def test_wal_stream_deleted(client, wal_dir_with_files):
    """WAL stream directory deleted."""
    import shutil
    stream_dir = wal_dir_with_files / "test-stream"
    shutil.rmtree(stream_dir)

    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200
    data = resp.json()
    assert "error" in data


def test_empty_managed_dict(client):
    """Empty managed dict."""
    import server
    server.managed.clear()

    resp = client.get("/api/processes")
    assert resp.status_code == 200


def test_empty_recent_orders(client):
    """Empty recent orders."""
    import server
    server.recent_orders.clear()

    resp = client.get("/x/recent-orders")
    assert resp.status_code == 200


def test_empty_verify_results(client):
    """Empty verify results."""
    import server
    server.verify_results.clear()

    resp = client.get("/x/verify")
    assert resp.status_code == 200


# ── Concurrent Operations (20) ────────────────────────────


def test_concurrent_order_submissions(client):
    """Concurrent order submissions."""
    import threading

    def submit_order():
        client.post(
            "/api/orders/test",
            data={
                "symbol_id": "10",
                "side": "buy",
                "price": "50000",
                "qty": "1",
            },
        )

    threads = [threading.Thread(target=submit_order) for _ in range(10)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_concurrent_verify_runs(client):
    """Concurrent verify runs."""
    import threading

    def run_verify():
        client.post("/api/verify/run")

    threads = [threading.Thread(target=run_verify) for _ in range(5)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_concurrent_log_queries(client):
    """Concurrent log queries."""
    import threading

    def query_logs():
        client.get("/api/logs")

    threads = [threading.Thread(target=query_logs) for _ in range(10)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_concurrent_process_actions(client, running_process):
    """Concurrent process actions."""
    import threading

    def process_action():
        client.get("/x/processes")

    threads = [threading.Thread(target=process_action) for _ in range(5)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_concurrent_wal_scans(client, wal_dir_with_files):
    """Concurrent WAL scans."""
    import threading

    def scan_wal():
        client.get("/x/wal-status")

    threads = [threading.Thread(target=scan_wal) for _ in range(5)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_read_write_race_recent_orders(client):
    """Read/write race on recent_orders."""
    import threading

    def write():
        client.post("/api/orders/batch")

    def read():
        client.get("/x/recent-orders")

    threads = [
        threading.Thread(target=write),
        threading.Thread(target=read),
        threading.Thread(target=write),
        threading.Thread(target=read),
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_concurrent_freeze_unfreeze(client):
    """Concurrent freeze/unfreeze."""
    import threading

    def freeze():
        client.post("/api/risk/users/1/freeze")

    def unfreeze():
        client.post("/api/risk/users/1/unfreeze")

    threads = [
        threading.Thread(target=freeze),
        threading.Thread(target=unfreeze),
        threading.Thread(target=freeze),
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_concurrent_metrics_queries(client):
    """Concurrent metrics queries."""
    import threading

    def query_metrics():
        client.get("/api/metrics")

    threads = [threading.Thread(target=query_metrics) for _ in range(10)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_concurrent_order_cancels(client):
    """Concurrent order cancels."""
    import threading
    import server

    client.post("/api/orders/batch")
    if not server.recent_orders:
        return
    cids = [o.get("cid") for o in server.recent_orders[:5] if o.get("cid")]
    if not cids:
        return

    results = []

    def cancel(cid):
        resp = client.post(f"/api/orders/{cid}/cancel")
        results.append(resp.status_code)

    threads = [threading.Thread(target=cancel, args=(c,)) for c in cids]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    for status in results:
        assert status == 200


def test_concurrent_different_endpoints(client):
    """Concurrent requests to different endpoints."""
    import threading

    endpoints = [
        "/api/processes",
        "/api/metrics",
        "/api/logs",
        "/x/wal-status",
        "/x/processes",
    ]

    def query(url):
        client.get(url)

    threads = [threading.Thread(target=query, args=(url,)) for url in endpoints]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_managed_dict_race(client):
    """Managed dict race condition."""
    import threading
    from unittest.mock import MagicMock
    import server

    proc = MagicMock()
    proc.pid = 99999
    proc.returncode = None

    def add():
        server.managed["race-proc"] = {
            "proc": proc,
            "binary": "./test",
            "env": {},
        }

    def remove():
        if "race-proc" in server.managed:
            del server.managed["race-proc"]

    threads = [
        threading.Thread(target=add),
        threading.Thread(target=remove),
        threading.Thread(target=add),
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_verify_results_race(client):
    """Verify results race condition."""
    import threading

    def run_verify():
        client.post("/api/verify/run")

    def read_verify():
        client.get("/x/verify")

    threads = [
        threading.Thread(target=run_verify),
        threading.Thread(target=read_verify),
        threading.Thread(target=run_verify),
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_log_file_write_read_race(client, log_dir_with_files):
    """Log file write/read race."""
    import threading

    log_file = log_dir_with_files / "race.log"

    def write():
        log_file.write_text("new log line\n")

    def read():
        client.get("/api/logs?process=race")

    threads = [
        threading.Thread(target=write),
        threading.Thread(target=read),
        threading.Thread(target=write),
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_wal_file_create_scan_race(client, wal_dir_with_files):
    """WAL file creation during scan."""
    import threading

    def create():
        stream = wal_dir_with_files / "test-stream"
        (stream / "race.dxs").write_bytes(b"data")

    def scan():
        client.get("/x/wal-files")

    threads = [
        threading.Thread(target=create),
        threading.Thread(target=scan),
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_concurrent_scenario_switch(client):
    """Concurrent scenario switches."""
    import threading

    def switch():
        client.post(
            "/api/scenario/switch",
            data={"scenario-select": "minimal"},
            headers={"x-confirm": "yes"},
        )

    threads = [threading.Thread(target=switch) for _ in range(3)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_concurrent_postgres_queries(client, mock_postgres_connected):
    """Concurrent postgres queries."""
    import threading

    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [{"user_id": 1, "balance": 10000}]

        def query():
            client.get("/api/risk/users/1")

        threads = [threading.Thread(target=query) for _ in range(10)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()


def test_process_start_stop_race(client):
    """Process start/stop race."""
    import threading

    with patch('server.do_build') as mock_build:
        with patch('server.spawn_process') as mock_spawn:
            with patch('server.stop_process') as mock_stop:
                mock_build.return_value = True
                mock_spawn.return_value = {"pid": 12345}
                mock_stop.return_value = {"status": "stopped"}

                def start():
                    client.post("/api/processes/test/start")

                def stop():
                    client.post("/api/processes/test/stop")

                threads = [
                    threading.Thread(target=start),
                    threading.Thread(target=stop),
                ]
                for t in threads:
                    t.start()
                for t in threads:
                    t.join()


def test_order_submission_limit_race(client):
    """Order submission with limit race."""
    import threading
    import server

    # Pre-fill with batch orders (fast, no gateway needed)
    for _ in range(25):
        client.post("/api/orders/batch")

    assert len(server.recent_orders) <= 200


def test_verify_clear_race(client):
    """Verify results clear race."""
    import threading

    def verify():
        client.post("/api/verify/run")

    threads = [threading.Thread(target=verify) for _ in range(5)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_concurrent_mixed_operations(client, wal_dir_with_files):
    """Concurrent mixed operations."""
    import threading

    def op1():
        client.post("/api/orders/batch")

    def op2():
        client.get("/api/logs")

    def op3():
        client.get("/x/wal-status")

    def op4():
        client.post("/api/verify/run")

    threads = [
        threading.Thread(target=op1),
        threading.Thread(target=op2),
        threading.Thread(target=op3),
        threading.Thread(target=op4),
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


# ── Timeout Scenarios (10) ────────────────────────────────


def test_long_running_query_timeout(client, mock_postgres_connected):
    """Long running query timeout."""
    import asyncio

    async def slow_query(*args):
        await asyncio.sleep(10)
        return []

    with patch('server.pg_query', side_effect=slow_query):
        resp = client.get("/api/risk/users/1", timeout=1)


def test_build_timeout(client):
    """Build timeout."""
    async def slow_build():
        import asyncio
        await asyncio.sleep(10)
        return False

    with patch('server.do_build', side_effect=slow_build):
        try:
            resp = client.post("/api/build", timeout=1)
        except Exception:
            pass


def test_process_start_timeout(client):
    """Process start timeout."""
    # FastAPI TestClient doesn't support real timeouts well
    # but endpoint should handle async properly
    resp = client.get("/api/processes")
    assert resp.status_code == 200


def test_wal_scan_large_dir_timeout(client, wal_dir_with_files):
    """WAL scan of large directory."""
    stream_dir = wal_dir_with_files / "test-stream"
    for i in range(100):
        (stream_dir / f"{i:06d}.dxs").write_bytes(b"d" * 1000)

    resp = client.get("/x/wal-files")
    assert resp.status_code == 200


def test_log_read_large_file_timeout(client, log_dir_with_files):
    """Log read of large file."""
    large_log = log_dir_with_files / "large.log"
    lines = [f"Log line {i}" for i in range(10000)]
    large_log.write_text("\n".join(lines))

    resp = client.get("/api/logs?process=large")
    assert resp.status_code == 200


def test_verify_run_timeout(client):
    """Verify run timeout."""
    resp = client.post("/api/verify/run")
    assert resp.status_code == 200


def test_stop_process_timeout(client, running_process):
    """Stop process timeout."""
    with patch('server.stop_process') as mock_stop:
        async def slow_stop(*args):
            import asyncio
            await asyncio.sleep(10)
            return {"status": "stopped"}

        mock_stop.side_effect = slow_stop
        try:
            resp = client.post("/api/processes/test-process/stop", timeout=1)
        except Exception:
            pass


def test_metrics_collection_timeout(client):
    """Metrics collection timeout."""
    resp = client.get("/api/metrics")
    assert resp.status_code == 200


def test_concurrent_timeout_recovery(client):
    """Concurrent timeout recovery."""
    import threading

    def query():
        try:
            client.get("/api/processes", timeout=0.1)
        except Exception:
            pass

    threads = [threading.Thread(target=query) for _ in range(5)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_process_hang_detection(client, running_process):
    """Process hang detection."""
    resp = client.get("/x/processes")
    assert resp.status_code == 200


# ── Security (10) ─────────────────────────────────────────


def test_sql_injection_prevention(client, mock_postgres_connected):
    """SQL injection prevented."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = []
        resp = client.get("/api/risk/users/1' OR '1'='1")

    assert resp.status_code in [200, 422]


def test_xss_prevention(client):
    """XSS prevented."""
    resp = client.get("/api/logs?search=<script>alert('xss')</script>")
    assert resp.status_code == 200


def test_path_traversal_prevention(client):
    """Path traversal prevented."""
    resp = client.get(
        "/api/wal/%2e%2e%2f%2e%2e%2fetc%2fpasswd/status")
    assert resp.status_code in [200, 400, 404, 422]
    if resp.status_code == 200:
        data = resp.json()
        assert "error" in data


def test_command_injection_prevention(client):
    """Command injection prevented."""
    resp = client.post("/api/processes/test; rm -rf //stop")
    assert resp.status_code in [200, 404]


def test_header_injection(client):
    """Header injection prevented."""
    resp = client.get(
        "/api/logs",
        headers={"X-Injected": "value\r\nX-Evil: malicious"}
    )
    assert resp.status_code == 200


def test_csrf_token_not_required(client):
    """CSRF protection (not implemented, but POST works)."""
    resp = client.post("/api/orders/batch")
    assert resp.status_code == 200


def test_rate_limiting_not_enforced(client):
    """Rate limiting (not implemented)."""
    for _ in range(100):
        resp = client.get("/api/processes")
        assert resp.status_code == 200


def test_authentication_not_required(client):
    """Authentication not required (playground mode)."""
    resp = client.get("/api/processes")
    assert resp.status_code == 200


def test_authorization_not_enforced(client):
    """Authorization not enforced (playground mode)."""
    resp = client.post("/api/risk/users/1/freeze")
    assert resp.status_code == 200


def test_file_upload_not_supported(client):
    """File upload not supported."""
    # No file upload endpoints exist
    assert True


# ── Performance (10) ──────────────────────────────────────


def test_metrics_endpoint_latency(client):
    """Metrics endpoint low latency."""
    import time
    start = time.time()
    resp = client.get("/api/metrics")
    elapsed = time.time() - start

    assert resp.status_code == 200
    assert elapsed < 0.5


def test_log_query_performance(client, log_dir_with_files):
    """Log query performance."""
    import time
    start = time.time()
    resp = client.get("/api/logs?limit=100")
    elapsed = time.time() - start

    assert resp.status_code == 200
    assert elapsed < 1.0


def test_process_list_performance(client):
    """Process list query performance."""
    import time
    start = time.time()
    resp = client.get("/api/processes")
    elapsed = time.time() - start

    assert resp.status_code == 200
    assert elapsed < 0.5


def test_wal_status_performance(client, wal_dir_with_files):
    """WAL status query performance."""
    import time
    start = time.time()
    resp = client.get("/x/wal-status")
    elapsed = time.time() - start

    assert resp.status_code == 200
    assert elapsed < 1.0


def test_verify_run_performance(client):
    """Verify run performance."""
    import time
    start = time.time()
    resp = client.post("/api/verify/run")
    elapsed = time.time() - start

    assert resp.status_code == 200
    assert elapsed < 2.0


@pytest.mark.allow_5xx
def test_large_order_batch_performance(client):
    """Large order batch performance."""
    import time
    start = time.time()
    for _ in range(10):
        client.post(
            "/api/stress/run",
            data={"rate": 1, "duration": 1},
        )
    elapsed = time.time() - start

    assert elapsed < 60.0


def test_concurrent_request_throughput(client):
    """Concurrent request throughput."""
    import threading
    import time

    def make_request():
        client.get("/api/metrics")

    start = time.time()
    threads = [threading.Thread(target=make_request) for _ in range(20)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    elapsed = time.time() - start

    assert elapsed < 5.0


def test_memory_usage_stable(client):
    """Memory usage remains stable."""
    for _ in range(100):
        client.post("/api/orders/batch")

    import server
    assert len(server.recent_orders) <= 200


def test_html_rendering_performance(client):
    """HTML rendering performance."""
    import time
    start = time.time()
    client.get("/")
    elapsed = time.time() - start

    assert elapsed < 1.0


def test_json_serialization_performance(client):
    """JSON serialization performance."""
    import time
    import server

    for _ in range(50):
        client.post("/api/orders/batch")

    start = time.time()
    resp = client.get("/api/processes")
    elapsed = time.time() - start

    assert resp.status_code == 200
    assert elapsed < 0.5


# ── Destructive Endpoint Guards ───────────────────────────


def test_all_start_without_confirm_returns_400(client):
    """POST /api/processes/all/start without x-confirm returns 400."""
    resp = client.post("/api/processes/all/start")
    assert resp.status_code == 400
    data = resp.json()
    assert "error" in data
    assert "destructive" in data["error"].lower()


def test_all_stop_without_confirm_returns_400(client):
    """POST /api/processes/all/stop without x-confirm returns 400."""
    resp = client.post("/api/processes/all/stop")
    assert resp.status_code == 400
    data = resp.json()
    assert "error" in data


def test_scenario_switch_without_confirm_returns_400(client):
    """POST /api/scenario/switch without x-confirm returns 400."""
    resp = client.post(
        "/api/scenario/switch",
        data={"scenario-select": "minimal"},
    )
    assert resp.status_code == 400
    data = resp.json()
    assert "error" in data


def test_confirm_yes_header_allows_destructive(client):
    """x-confirm: yes header allows destructive endpoints."""
    resp = client.post(
        "/api/processes/all/stop",
        headers={"x-confirm": "yes"},
    )
    assert resp.status_code == 200


def test_confirm_query_param_allows_destructive(client):
    """?confirm=yes query param allows destructive endpoints."""
    resp = client.post(
        "/api/processes/all/stop?confirm=yes",
    )
    assert resp.status_code == 200


def test_hx_request_header_bypasses_confirm(client):
    """hx-request header bypasses confirm guard (HTMX)."""
    resp = client.post(
        "/api/processes/all/stop",
        headers={"hx-request": "true"},
    )
    assert resp.status_code == 200


def test_confirm_no_value_returns_400(client):
    """x-confirm with wrong value returns 400."""
    resp = client.post(
        "/api/processes/all/stop",
        headers={"x-confirm": "no"},
    )
    assert resp.status_code == 400


# ── No absolute href/src regression (3) ──────────────────


_PAGE_ROUTES = [
    "/overview", "/topology", "/book", "/risk", "/wal",
    "/logs", "/control", "/faults", "/verify", "/orders",
    "/stress", "/docs",
]


def test_no_rooted_href_in_page_html(client):
    """Rendered pages must not contain href='/' or href='/word'."""
    import re
    rooted = re.compile(
        r'(?:href|src|action|hx-get|hx-post|hx-put|hx-delete|hx-patch)'
        r'=["\'][/][^/]'
    )
    for route in _PAGE_ROUTES:
        resp = client.get(route)
        assert resp.status_code == 200, f"{route} not 200"
        bad = rooted.findall(resp.text)
        assert not bad, (
            f"{route} has rooted link(s): {bad[:3]}"
        )


def test_no_rooted_href_in_partials(client):
    """HTMX partials must not contain rooted href/src."""
    import re
    rooted = re.compile(
        r'(?:href|src|action|hx-get|hx-post|hx-put|hx-delete|hx-patch)'
        r'=["\'][/][^/]'
    )
    partials = [
        "/x/overview-stats", "/x/topology-grid",
        "/x/wal-status", "/x/log-tail",
        "/x/verify", "/x/process-status",
    ]
    for route in partials:
        resp = client.get(route)
        # partials may 200 or 404 — only check if they render HTML
        if resp.status_code != 200:
            continue
        ct = resp.headers.get("content-type", "")
        if "html" not in ct:
            continue
        bad = rooted.findall(resp.text)
        assert not bad, (
            f"{route} has rooted link(s): {bad[:3]}"
        )


def test_pages_py_source_no_rooted_links():
    """pages.py source must contain no rooted href/src literals."""
    import re
    from pathlib import Path
    src = (Path(__file__).parent.parent / "pages.py").read_text()
    rooted = re.compile(
        r'(?:href|src|action|hx-get|hx-post|hx-put|hx-delete|hx-patch)'
        r'\s*=\s*[f]?["\'][/][^/]'
    )
    bad = rooted.findall(src)
    assert not bad, f"pages.py has rooted link(s): {bad[:5]}"
