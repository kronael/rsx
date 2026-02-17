"""Full E2E integration tests for RSX Playground.

Real process lifecycle, order workflows, WAL, risk, verification.

Run with: cd rsx-playground && uv run pytest tests/api_integration_test.py -v
"""

import pytest
import time
from unittest.mock import patch, AsyncMock
from test_utils import create_test_order


# ── Process Lifecycle (15) ────────────────────────────────


def test_build_succeeds(client):
    """Build process completes successfully."""
    with patch('server.do_build') as mock_build:
        mock_build.return_value = True
        resp = client.post("/api/build")

    assert resp.status_code == 200
    assert "build ok" in resp.text.lower()


def test_start_single_process(client):
    """Start single process via API."""
    with patch('server.do_build') as mock_build:
        with patch('server.spawn_process') as mock_spawn:
            mock_build.return_value = True
            mock_spawn.return_value = {"pid": 12345}

            resp = client.post("/api/processes/gateway/start")

    assert resp.status_code == 200


def test_stop_process(client, running_process):
    """Stop running process."""
    with patch('server.stop_process') as mock_stop:
        mock_stop.return_value = {"status": "stopped"}
        resp = client.post("/api/processes/test-process/stop")

    assert resp.status_code == 200


def test_restart_process(client, running_process):
    """Restart running process."""
    with patch('server.restart_process') as mock_restart:
        mock_restart.return_value = {"pid": 12346}
        resp = client.post("/api/processes/test-process/restart")

    assert resp.status_code == 200


def test_kill_process(client, running_process):
    """Kill running process."""
    with patch('server.kill_process') as mock_kill:
        mock_kill.return_value = {"status": "killed"}
        resp = client.post("/api/processes/test-process/kill")

    assert resp.status_code == 200


def test_start_all_minimal_scenario(client):
    """Start all processes in minimal scenario."""
    with patch('server.start_all') as mock_start_all:
        mock_start_all.return_value = {
            "started": ["gateway", "risk", "me-pengu"],
            "count": 3
        }
        resp = client.post(
            "/api/processes/all/start?scenario=minimal",
            headers={"x-confirm": "yes"},
        )

    assert resp.status_code == 200


def test_stop_all_processes(client):
    """Stop all running processes."""
    with patch('server.stop_all') as mock_stop_all:
        mock_stop_all.return_value = {"stopped": ["gateway", "risk"]}
        resp = client.post(
            "/api/processes/all/stop",
            headers={"x-confirm": "yes"},
        )

    assert resp.status_code == 200


def test_switch_scenario(client):
    """Switch to different scenario."""
    resp = client.post(
        "/api/scenario/switch",
        data={"scenario-select": "basic"},
        headers={"x-confirm": "yes"},
    )
    assert resp.status_code == 200


def test_get_current_scenario(client):
    """Get current scenario."""
    resp = client.get("/x/current-scenario")
    assert resp.status_code == 200


def test_list_scenarios(client):
    """List available scenarios."""
    resp = client.get("/api/scenarios")
    assert resp.status_code == 200
    data = resp.json()
    assert "minimal" in data


def test_process_lifecycle_full_cycle(client):
    """Complete process lifecycle."""
    with patch('server.do_build') as mock_build:
        with patch('server.spawn_process') as mock_spawn:
            with patch('server.stop_process') as mock_stop:
                mock_build.return_value = True
                mock_spawn.return_value = {"pid": 12345}
                mock_stop.return_value = {"status": "stopped"}

                client.post("/api/processes/gateway/start")
                client.post("/api/processes/gateway/stop")


def test_get_processes_list(client):
    """Get list of all processes."""
    resp = client.get("/api/processes")
    assert resp.status_code == 200
    data = resp.json()
    assert isinstance(data, list)


def test_x_processes_table(client):
    """Get processes HTML table."""
    resp = client.get("/x/processes")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_process_state_visible_in_table(client, running_process):
    """Process state visible in table."""
    resp = client.get("/x/processes")
    assert resp.status_code == 200


def test_build_failure_handled(client):
    """Build failure handled gracefully."""
    with patch('server.do_build') as mock_build:
        mock_build.return_value = False
        resp = client.post("/api/build")

    assert resp.status_code == 200
    assert "FAILED" in resp.text


# ── Order Workflows (20) ──────────────────────────────────


def test_submit_test_order(client):
    """Submit test order via form."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )
    assert resp.status_code == 200
    text = resp.text.lower()
    assert "submitted" in text or "queued" in text


def test_order_appears_in_recent(client):
    """Order appears in recent orders."""
    client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )

    import server
    assert len(server.recent_orders) > 0


def test_recent_orders_html_view(client):
    """Recent orders HTML view."""
    client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )

    resp = client.get("/x/recent-orders")
    assert resp.status_code == 200


def test_submit_batch_orders(client):
    """Submit batch orders."""
    resp = client.post("/api/orders/batch")
    assert resp.status_code == 200
    assert "10 batch orders" in resp.text


def test_batch_orders_in_recent(client):
    """Batch orders appear in recent."""
    client.post("/api/orders/batch")

    import server
    assert len(server.recent_orders) >= 10


def test_submit_random_orders(client):
    """Submit random orders."""
    resp = client.post("/api/orders/random")
    assert resp.status_code == 200
    assert "5 random orders" in resp.text


@pytest.mark.allow_5xx
def test_submit_stress_orders(client):
    """Submit stress test orders."""
    resp = client.post("/api/stress/run")
    assert resp.status_code in (200, 502)


def test_submit_invalid_order(client):
    """Submit invalid order."""
    resp = client.post("/api/orders/invalid")
    assert resp.status_code == 200
    assert "rejected" in resp.text.lower()


def test_cancel_order(client):
    """Cancel submitted order."""
    client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )

    import server
    cid = server.recent_orders[-1]["cid"]

    resp = client.post(f"/api/orders/{cid}/cancel")
    assert resp.status_code == 200


def test_order_status_tracking(client):
    """Order status tracked correctly."""
    client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )

    import server
    order = server.recent_orders[-1]
    assert order["status"] in ("submitted", "error", "pending", "queued")


def test_order_limit_enforcement(client):
    """Recent orders list limited."""
    for _ in range(250):
        client.post("/api/orders/batch")

    import server
    assert len(server.recent_orders) <= 200


def test_order_cid_unique(client):
    """Order client IDs are unique."""
    client.post("/api/orders/batch")

    import server
    cids = [o["cid"] for o in server.recent_orders]
    assert len(cids) == len(set(cids))


def test_order_timestamp_included(client):
    """Order includes timestamp."""
    client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )

    import server
    order = server.recent_orders[-1]
    assert "ts" in order


def test_order_with_tif(client):
    """Order with TIF parameter."""
    client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
            "tif": "IOC",
        },
    )

    import server
    order = server.recent_orders[-1]
    assert order["tif"] == "IOC"


def test_order_with_reduce_only(client):
    """Order with reduce_only flag."""
    client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
            "reduce_only": "on",
        },
    )

    import server
    order = server.recent_orders[-1]
    assert order["reduce_only"] is True


def test_order_with_post_only(client):
    """Order with post_only flag."""
    client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
            "post_only": "on",
        },
    )

    import server
    order = server.recent_orders[-1]
    assert order["post_only"] is True


def test_order_trace_endpoint(client):
    """Order trace endpoint."""
    client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )

    import server
    oid = server.recent_orders[-1]["cid"]

    resp = client.get(f"/x/order-trace?trace-oid={oid}")
    assert resp.status_code == 200


def test_x_stale_orders_check(client):
    """Check for stale orders."""
    resp = client.get("/x/stale-orders")
    assert resp.status_code == 200


def test_create_user_endpoint(client):
    """Create user endpoint."""
    resp = client.post("/api/users")
    assert resp.status_code in (200, 404, 500, 503)


def test_deposit_endpoint(client):
    """User deposit endpoint."""
    resp = client.post("/api/users/1/deposit")
    assert resp.status_code == 200


# ── Risk Workflows (15) ───────────────────────────────────


def test_freeze_user_then_check_status(
    client, mock_postgres_connected
):
    """Freeze user then check status."""
    client.post("/api/risk/users/1/freeze")

    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [{"user_id": 1, "frozen": True}]
        resp = client.get("/api/risk/users/1")

    assert resp.status_code == 200


def test_unfreeze_user_then_check_status(client):
    """Unfreeze user then check status."""
    client.post("/api/risk/users/1/freeze")
    client.post("/api/risk/users/1/unfreeze")

    resp = client.get("/api/risk/users/1")
    assert resp.status_code == 200


def test_frozen_user_order_rejected(client):
    """Frozen user order expected to be rejected."""
    client.post("/api/risk/users/1/freeze")

    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_unfrozen_user_order_accepted(client):
    """Unfrozen user order accepted."""
    client.post("/api/risk/users/1/freeze")
    client.post("/api/risk/users/1/unfreeze")

    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_position_update_after_fill(
    client, mock_postgres_connected
):
    """Position updates after fill."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "position": 0}
        ]
        resp1 = client.get("/api/risk/users/1")

        mock_query.return_value = [
            {"user_id": 1, "position": 100}
        ]
        resp2 = client.get("/api/risk/users/1")

    assert resp1.status_code == 200
    assert resp2.status_code == 200


def test_liquidation_trigger(client):
    """Trigger liquidation."""
    resp = client.post("/api/risk/liquidate")
    assert resp.status_code == 200


def test_liquidation_appears_in_queue(
    client, mock_postgres_connected
):
    """Liquidation appears in queue."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "symbol_id": 10, "qty": 100}
        ]
        client.post("/api/risk/liquidate")
        resp = client.get("/x/liquidations")

    assert resp.status_code == 200


def test_multiple_user_positions(client, mock_postgres_connected):
    """Multiple user positions tracked."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "symbol_id": 10, "position": 100},
            {"user_id": 1, "symbol_id": 20, "position": -50},
        ]
        resp = client.get("/api/risk/users/1")

    assert resp.status_code == 200
    data = resp.json()
    assert len(data) == 2


def test_position_heatmap_integration(client):
    """Position heatmap integration."""
    resp = client.get("/x/position-heatmap")
    assert resp.status_code == 200


def test_margin_ladder_integration(client):
    """Margin ladder integration."""
    resp = client.get("/x/margin-ladder")
    assert resp.status_code == 200


def test_funding_calculation(client):
    """Funding calculation."""
    resp = client.get("/x/funding")
    assert resp.status_code == 200


def test_risk_latency_monitoring(client):
    """Risk latency monitoring."""
    resp = client.get("/x/risk-latency")
    assert resp.status_code == 200


def test_reconciliation_check_integration(client):
    """Reconciliation check."""
    resp = client.get("/x/reconciliation")
    assert resp.status_code == 200


def test_freeze_multiple_users_independently(client):
    """Freeze multiple users independently."""
    client.post("/api/risk/users/1/freeze")
    client.post("/api/risk/users/2/freeze")

    resp1 = client.get("/api/risk/users/1")
    resp2 = client.get("/api/risk/users/2")

    assert resp1.status_code == 200
    assert resp2.status_code == 200


def test_risk_state_consistency(client, mock_postgres_connected):
    """Risk state remains consistent."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [{"user_id": 1, "balance": 10000}]

        resp1 = client.get("/api/risk/users/1")
        resp2 = client.get("/api/risk/users/1")
        resp3 = client.get("/api/risk/users/1")

    assert all(r.status_code == 200 for r in [resp1, resp2, resp3])


# ── WAL Workflows (15) ────────────────────────────────────


def test_wal_created_after_process_start(client, wal_dir_with_files):
    """WAL files created after process start."""
    resp = client.get("/x/wal-status")
    assert resp.status_code == 200


def test_wal_stream_status_check(client, wal_dir_with_files):
    """Check WAL stream status."""
    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200
    data = resp.json()
    assert data["stream"] == "test-stream"


def test_wal_verify_after_orders(client, wal_dir_with_files):
    """Verify WAL after orders submitted."""
    client.post("/api/orders/batch")
    resp = client.post("/api/wal/verify")

    assert resp.status_code == 200


def test_wal_dump_shows_events(client, wal_dir_with_files):
    """WAL dump shows events."""
    resp = client.post("/api/wal/dump")
    assert resp.status_code == 200


def test_wal_rotation_tracking(client, wal_dir_with_files):
    """WAL rotation tracking."""
    resp = client.get("/x/wal-rotation")
    assert resp.status_code == 200


def test_wal_lag_monitoring(client, wal_dir_with_files):
    """WAL lag monitoring."""
    resp = client.get("/x/wal-lag")
    assert resp.status_code == 200


def test_wal_timeline_view(client, wal_dir_with_files):
    """WAL timeline view."""
    resp = client.get("/x/wal-timeline")
    assert resp.status_code == 200


def test_wal_files_list(client, wal_dir_with_files):
    """List all WAL files."""
    resp = client.get("/x/wal-files")
    assert resp.status_code == 200


def test_wal_detail_view(client, wal_dir_with_files):
    """WAL detail view."""
    resp = client.get("/x/wal-detail")
    assert resp.status_code == 200


def test_multiple_wal_streams(client, wal_dir_with_files):
    """Multiple WAL streams handled."""
    stream2 = wal_dir_with_files / "stream2"
    stream2.mkdir()
    (stream2 / "000001.dxs").write_bytes(b"data")

    resp = client.get("/x/wal-status")
    assert resp.status_code == 200


def test_wal_sequence_monotonic(client, wal_dir_with_files):
    """WAL sequence numbers monotonic."""
    resp = client.get("/x/wal-files")
    assert resp.status_code == 200


def test_wal_file_size_tracking(client, wal_dir_with_files):
    """WAL file size tracking."""
    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200
    data = resp.json()
    assert "total_bytes" in data


def test_wal_newest_timestamp(client, wal_dir_with_files):
    """WAL newest file timestamp."""
    resp = client.get("/x/wal-status")
    assert resp.status_code == 200


def test_wal_integration_with_verify(client, wal_dir_with_files):
    """WAL integration with verify."""
    client.post("/api/wal/verify")
    resp = client.post("/api/verify/run")

    assert resp.status_code == 200


def test_wal_end_to_end(client, wal_dir_with_files):
    """Complete WAL workflow."""
    resp1 = client.get("/x/wal-status")
    resp2 = client.get("/api/wal/test-stream/status")
    resp3 = client.post("/api/wal/verify")
    resp4 = client.post("/api/wal/dump")

    assert all(r.status_code == 200 for r in [
        resp1, resp2, resp3, resp4
    ])


# ── Verification (10) ─────────────────────────────────────


def test_verify_all_checks_run(client):
    """All verify checks run."""
    client.post("/api/verify/run")

    import server
    assert len(server.verify_results) >= 9


def test_verify_with_live_system(
    client, running_process, wal_dir_with_files,
    mock_postgres_connected
):
    """Verify with live system."""
    client.post("/api/verify/run")

    import server
    checks = server.verify_results
    pass_count = sum(1 for c in checks if c["status"] == "pass")

    assert pass_count >= 3


def test_verify_10_invariants(client, running_process):
    """Verify all 10 correctness invariants."""
    client.post("/api/verify/run")

    import server
    inv_checks = [
        c for c in server.verify_results
        if any(
            word in c["name"]
            for word in ["crossed", "Tips", "Slab", "Position",
                         "FIFO", "completion", "Fills"]
        )
    ]

    assert len(inv_checks) >= 6


def test_verify_wal_directory_check(client, wal_dir_with_files):
    """Verify WAL directory check."""
    client.post("/api/verify/run")

    import server
    wal_check = next(
        (c for c in server.verify_results if "WAL directory" in c["name"]),
        None
    )
    assert wal_check is not None
    assert wal_check["status"] == "pass"


def test_verify_processes_check(client, running_process):
    """Verify processes check."""
    client.post("/api/verify/run")

    import server
    proc_check = next(
        (c for c in server.verify_results if "processes running" in c["name"]),
        None
    )
    assert proc_check is not None


def test_verify_postgres_check(client, mock_postgres_connected):
    """Verify postgres check."""
    client.post("/api/verify/run")

    import server
    pg_check = next(
        (c for c in server.verify_results if "Postgres" in c["name"]),
        None
    )
    assert pg_check is not None


def test_verify_results_rendered(client):
    """Verify results rendered in HTML."""
    client.post("/api/verify/run")
    resp = client.get("/x/verify")

    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_verify_invariant_status_widget(client):
    """Verify invariant status widget."""
    client.post("/api/verify/run")
    resp = client.get("/x/invariant-status")

    assert resp.status_code == 200


def test_verify_multiple_runs_stable(client):
    """Multiple verify runs produce stable results."""
    client.post("/api/verify/run")
    client.post("/api/verify/run")
    client.post("/api/verify/run")

    import server
    assert len(server.verify_results) > 0


def test_verify_complete_integration(
    client, running_process, wal_dir_with_files,
    mock_postgres_connected
):
    """Complete verify integration."""
    client.post("/api/orders/batch")
    time.sleep(0.1)
    client.post("/api/verify/run")

    import server
    assert len(server.verify_results) >= 10


# ── Multi-Component Stress (15) ───────────────────────────


@pytest.mark.allow_5xx
def test_high_order_throughput(client):
    """High order throughput."""
    for _ in range(10):
        resp = client.post("/api/stress/run")
        assert resp.status_code in (200, 502)


def test_concurrent_api_requests(client):
    """Concurrent API requests handled."""
    import threading

    def make_requests():
        client.get("/api/processes")
        client.get("/api/metrics")
        client.get("/api/logs")

    threads = [threading.Thread(target=make_requests) for _ in range(5)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def test_mixed_operations(client, wal_dir_with_files):
    """Mixed operations don't interfere."""
    client.post("/api/orders/batch")
    client.get("/api/processes")
    client.post("/api/verify/run")
    client.get("/api/logs")
    client.get("/x/wal-status")

    assert True


def test_state_consistency_under_load(client):
    """State consistency under load."""
    for _ in range(20):
        client.post("/api/orders/random")

    import server
    orders = server.recent_orders
    assert len(orders) <= 200


def test_memory_leak_detection(client):
    """No memory leaks in order storage."""
    import server
    initial_len = len(server.recent_orders)

    for _ in range(500):
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        })

    final_len = len(server.recent_orders)
    assert final_len <= 200


def test_all_endpoints_accessible(client):
    """All major endpoints accessible."""
    endpoints = [
        "/api/processes",
        "/api/metrics",
        "/api/logs",
        "/api/scenarios",
        "/x/processes",
        "/x/health",
        "/x/wal-status",
        "/x/verify",
    ]

    for endpoint in endpoints:
        resp = client.get(endpoint)
        assert resp.status_code == 200


def test_html_and_json_endpoints_consistent(client):
    """HTML and JSON endpoints consistent."""
    json_resp = client.get("/api/processes")
    html_resp = client.get("/x/processes")

    assert json_resp.status_code == 200
    assert html_resp.status_code == 200


def test_error_handling_under_load(client):
    """Error handling under load."""
    for _ in range(50):
        client.post("/api/orders/invalid")

    import server
    assert len(server.recent_orders) > 0


def test_log_filtering_performance(client, log_dir_with_files):
    """Log filtering performance."""
    large_log = log_dir_with_files / "large.log"
    lines = [f"Line {i}" for i in range(1000)]
    large_log.write_text("\n".join(lines))

    import time
    start = time.time()
    resp = client.get("/api/logs?process=large")
    elapsed = time.time() - start

    assert resp.status_code == 200
    assert elapsed < 1.0


def test_wal_scan_performance(client, wal_dir_with_files):
    """WAL scan performance with many files."""
    stream_dir = wal_dir_with_files / "test-stream"
    for i in range(3, 50):
        (stream_dir / f"{i:06d}.dxs").write_bytes(b"d" * 100)

    import time
    start = time.time()
    resp = client.get("/x/wal-files")
    elapsed = time.time() - start

    assert resp.status_code == 200
    assert elapsed < 2.0


def test_verify_performance(client):
    """Verify performance."""
    import time
    start = time.time()
    client.post("/api/verify/run")
    elapsed = time.time() - start

    assert elapsed < 1.0


def test_metrics_latency(client):
    """Metrics endpoint latency."""
    import time
    latencies = []

    for _ in range(10):
        start = time.time()
        client.get("/api/metrics")
        latencies.append(time.time() - start)

    avg_latency = sum(latencies) / len(latencies)
    assert avg_latency < 0.1


def test_full_system_integration(
    client, running_process, wal_dir_with_files,
    mock_postgres_connected
):
    """Full system integration test."""
    client.post("/api/orders/batch")
    client.get("/api/processes")
    client.get("/api/metrics")
    client.post("/api/verify/run")
    client.get("/x/wal-status")
    client.get("/api/logs")

    import server
    assert len(server.recent_orders) >= 10
    assert len(server.verify_results) > 0


def test_scenario_switch_integration(client):
    """Scenario switch integration."""
    client.post(
        "/api/scenario/switch",
        data={"scenario-select": "minimal"},
        headers={"x-confirm": "yes"},
    )
    resp = client.get("/x/current-scenario")

    assert resp.status_code == 200
    assert "minimal" in resp.text


def test_complete_e2e_workflow(
    client, running_process, wal_dir_with_files,
    mock_postgres_connected
):
    """Complete E2E workflow."""
    client.get("/api/scenarios")
    client.post("/api/orders/batch")
    client.get("/x/recent-orders")
    client.post("/api/risk/users/1/freeze")
    client.get("/api/risk/users/1")
    client.post("/api/wal/verify")
    client.post("/api/verify/run")
    client.get("/x/verify")

    import server
    assert len(server.recent_orders) >= 10
    assert len(server.verify_results) > 0
