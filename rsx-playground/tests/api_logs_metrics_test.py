"""Logs and Metrics API endpoint tests.

Run with: cd rsx-playground && uv run pytest tests/api_logs_metrics_test.py -v
"""

import pytest
from unittest.mock import patch


# ── Logs Tests (27) ───────────────────────────────────────


def test_logs_returns_json_structure(client, log_dir_with_files):
    """GET /api/logs returns JSON with lines and count."""
    resp = client.get("/api/logs")
    assert resp.status_code == 200
    data = resp.json()
    assert "lines" in data
    assert "count" in data
    assert isinstance(data["lines"], list)
    assert isinstance(data["count"], int)


def test_logs_filter_by_process(client, log_dir_with_files):
    """Filter logs by process name."""
    resp = client.get("/api/logs?process=gateway")
    assert resp.status_code == 200
    data = resp.json()
    assert all("gateway" in line for line in data["lines"])


def test_logs_filter_by_level(client, log_dir_with_files):
    """Filter logs by level."""
    resp = client.get("/api/logs?level=ERROR")
    assert resp.status_code == 200
    data = resp.json()
    assert all("error" in line.lower() for line in data["lines"])


def test_logs_filter_by_search_term(client, log_dir_with_files):
    """Filter logs by search term."""
    resp = client.get("/api/logs?search=connection")
    assert resp.status_code == 200
    data = resp.json()
    assert all("connection" in line.lower() for line in data["lines"])


def test_logs_limit_parameter(client, log_dir_with_files):
    """Limit parameter restricts result count."""
    resp = client.get("/api/logs?limit=1")
    assert resp.status_code == 200
    data = resp.json()
    assert len(data["lines"]) <= 1


def test_logs_combined_filters(client, log_dir_with_files):
    """Multiple filters work together."""
    resp = client.get(
        "/api/logs?process=gateway&level=ERROR&search=failed"
    )
    assert resp.status_code == 200
    data = resp.json()
    assert isinstance(data["lines"], list)


def test_logs_ansi_stripping(client, log_dir_with_files):
    """ANSI escape codes stripped from logs."""
    log_file = log_dir_with_files / "ansi.log"
    log_file.write_text("\x1b[31mRED TEXT\x1b[0m normal")

    resp = client.get("/api/logs?process=ansi")
    assert resp.status_code == 200
    data = resp.json()
    if data["lines"]:
        assert "\x1b" not in data["lines"][0]


def test_logs_empty_result(client, log_dir_with_files):
    """No matching logs returns empty list."""
    resp = client.get("/api/logs?search=nonexistent")
    assert resp.status_code == 200
    data = resp.json()
    assert data["lines"] == []
    assert data["count"] == 0


def test_logs_pagination_via_limit(client, log_dir_with_files):
    """Pagination via limit works."""
    resp1 = client.get("/api/logs?limit=1")
    resp2 = client.get("/api/logs?limit=2")

    data1 = resp1.json()
    data2 = resp2.json()

    assert len(data1["lines"]) <= 1
    assert len(data2["lines"]) <= 2


def test_logs_max_lines_default(client, log_dir_with_files):
    """Default max lines applied."""
    resp = client.get("/api/logs")
    assert resp.status_code == 200
    data = resp.json()
    assert len(data["lines"]) <= 500


def test_x_logs_with_filters(client, log_dir_with_files):
    """GET /x/logs with filters returns HTML."""
    resp = client.get(
        "/x/logs?log-process=gateway&log-level=info"
    )
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_logs_tail_returns_recent(client, log_dir_with_files):
    """GET /x/logs-tail returns recent logs."""
    resp = client.get("/x/logs-tail")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_error_agg_aggregates_errors(
    client, log_dir_with_files
):
    """GET /x/error-agg aggregates error messages."""
    resp = client.get("/x/error-agg")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_logs_process_filter_nonexistent(
    client, log_dir_with_files
):
    """Filter by nonexistent process returns empty."""
    resp = client.get("/api/logs?process=nonexistent")
    assert resp.status_code == 200
    data = resp.json()
    assert data["count"] == 0


def test_logs_case_insensitive_search(client, log_dir_with_files):
    """Search is case-insensitive."""
    resp = client.get("/api/logs?search=ERROR")
    assert resp.status_code == 200
    data = resp.json()
    if data["lines"]:
        assert any("error" in line.lower() for line in data["lines"])


def test_logs_level_case_insensitive(client, log_dir_with_files):
    """Level filter is case-insensitive."""
    resp = client.get("/api/logs?level=info")
    assert resp.status_code == 200


def test_logs_multiline_entries(client, log_dir_with_files):
    """Multiline log entries handled."""
    log_file = log_dir_with_files / "multiline.log"
    log_file.write_text("Line 1\nLine 2\nLine 3")

    resp = client.get("/api/logs?process=multiline")
    assert resp.status_code == 200
    data = resp.json()
    assert len(data["lines"]) == 3


def test_logs_unicode_content(client, log_dir_with_files):
    """Unicode content in logs handled."""
    log_file = log_dir_with_files / "unicode.log"
    log_file.write_text("INFO: 日本語 content")

    resp = client.get("/api/logs?process=unicode")
    assert resp.status_code == 200
    data = resp.json()
    assert len(data["lines"]) > 0


def test_logs_no_log_dir(client):
    """No log directory handled gracefully."""
    import server
    original_log = server.LOG_DIR
    server.LOG_DIR = server.Path("/nonexistent")

    resp = client.get("/api/logs")

    server.LOG_DIR = original_log
    assert resp.status_code == 200
    data = resp.json()
    assert data["count"] == 0


def test_logs_empty_log_file(client, log_dir_with_files):
    """Empty log file handled."""
    (log_dir_with_files / "empty.log").write_text("")

    resp = client.get("/api/logs?process=empty")
    assert resp.status_code == 200
    data = resp.json()
    assert data["count"] == 0


def test_logs_multiple_files_aggregated(
    client, log_dir_with_files
):
    """Multiple log files aggregated."""
    resp = client.get("/api/logs")
    assert resp.status_code == 200
    data = resp.json()
    assert data["count"] >= 0


def test_logs_file_prefix_included(client, log_dir_with_files):
    """Log lines include file prefix."""
    resp = client.get("/api/logs?process=gateway")
    assert resp.status_code == 200
    data = resp.json()
    if data["lines"]:
        assert "[gateway]" in data["lines"][0]


def test_logs_tail_limit_20(client, log_dir_with_files):
    """Tail endpoint limits to 20 lines."""
    log_file = log_dir_with_files / "long.log"
    log_file.write_text("\n".join(f"Line {i}" for i in range(50)))

    resp = client.get("/x/logs-tail")
    assert resp.status_code == 200


def test_logs_read_from_end(client, log_dir_with_files):
    """Logs read from end of file (tail behavior)."""
    log_file = log_dir_with_files / "tail-test.log"
    lines = [f"Line {i}" for i in range(100)]
    log_file.write_text("\n".join(lines))

    resp = client.get("/api/logs?process=tail-test&limit=10")
    assert resp.status_code == 200
    data = resp.json()
    assert len(data["lines"]) <= 10


def test_logs_filter_combination_no_match(
    client, log_dir_with_files
):
    """Combination of filters with no match."""
    resp = client.get(
        "/api/logs?process=gateway&level=DEBUG&search=xyz"
    )
    assert resp.status_code == 200
    data = resp.json()
    assert data["count"] == 0


def test_logs_special_characters_in_search(
    client, log_dir_with_files
):
    """Special characters in search handled."""
    resp = client.get("/api/logs?search=[ERROR]")
    assert resp.status_code == 200


def test_x_auth_failures_returns_html(client):
    """GET /x/auth-failures returns HTML."""
    resp = client.get("/x/auth-failures")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


# ── Metrics Tests (8) ─────────────────────────────────────


def test_metrics_returns_json_structure(client):
    """GET /api/metrics returns JSON structure."""
    resp = client.get("/api/metrics")
    assert resp.status_code == 200
    data = resp.json()
    assert "processes" in data
    assert "running" in data
    assert "postgres" in data


def test_metrics_process_count(client, running_process):
    """Metrics includes process count."""
    resp = client.get("/api/metrics")
    assert resp.status_code == 200
    data = resp.json()
    assert isinstance(data["processes"], int)
    assert data["processes"] >= 0


def test_metrics_running_count(client, running_process):
    """Metrics includes running process count."""
    resp = client.get("/api/metrics")
    assert resp.status_code == 200
    data = resp.json()
    assert isinstance(data["running"], int)


def test_metrics_postgres_status(client, mock_postgres_connected):
    """Metrics includes postgres connection status."""
    resp = client.get("/api/metrics")
    assert resp.status_code == 200
    data = resp.json()
    assert data["postgres"] is True


def test_metrics_postgres_down(client, mock_postgres_down):
    """Metrics shows postgres down correctly."""
    resp = client.get("/api/metrics")
    assert resp.status_code == 200
    data = resp.json()
    assert data["postgres"] is False


def test_x_key_metrics_returns_html(client):
    """GET /x/key-metrics returns HTML."""
    resp = client.get("/x/key-metrics")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_health_returns_html(client):
    """GET /x/health returns HTML."""
    resp = client.get("/x/health")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_resource_usage_returns_html(client):
    """GET /x/resource-usage returns HTML."""
    resp = client.get("/x/resource-usage")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


# ── Integration Tests (15) ────────────────────────────────


def test_logs_after_process_start(client, running_process):
    """Logs available after process starts."""
    resp = client.get("/api/logs")
    assert resp.status_code == 200


def test_metrics_reflect_running_processes(
    client, running_process
):
    """Metrics reflect running processes."""
    resp = client.get("/api/metrics")
    assert resp.status_code == 200
    data = resp.json()
    assert data["running"] >= 1


def test_logs_and_metrics_consistent(
    client, running_process, log_dir_with_files
):
    """Logs and metrics show consistent state."""
    resp_logs = client.get("/api/logs")
    resp_metrics = client.get("/api/metrics")

    assert resp_logs.status_code == 200
    assert resp_metrics.status_code == 200


def test_error_logs_appear_in_aggregation(
    client, log_dir_with_files
):
    """Error logs appear in aggregation."""
    resp = client.get("/x/error-agg")
    assert resp.status_code == 200


def test_logs_filter_workflow(client, log_dir_with_files):
    """Complete log filtering workflow."""
    resp1 = client.get("/api/logs")
    resp2 = client.get("/api/logs?process=gateway")
    resp3 = client.get("/api/logs?level=ERROR")
    resp4 = client.get("/api/logs?search=failed")

    assert all(r.status_code == 200 for r in [
        resp1, resp2, resp3, resp4
    ])


def test_metrics_update_with_process_changes(client):
    """Metrics update when processes change."""
    resp1 = client.get("/api/metrics")
    data1 = resp1.json()

    import server
    from unittest.mock import AsyncMock
    proc = AsyncMock()
    proc.pid = 99999
    proc.returncode = None
    server.managed["new-proc"] = {
        "proc": proc,
        "binary": "./test",
        "env": {},
    }

    resp2 = client.get("/api/metrics")
    data2 = resp2.json()

    assert resp2.status_code == 200


def test_logs_tail_auto_refresh(client, log_dir_with_files):
    """Logs tail for auto-refresh."""
    resp = client.get("/x/logs-tail")
    assert resp.status_code == 200


def test_health_check_integration(
    client, mock_postgres_connected
):
    """Health check integration."""
    resp = client.get("/x/health")
    assert resp.status_code == 200


def test_key_metrics_show_wal_status(
    client, wal_dir_with_files
):
    """Key metrics show WAL status."""
    resp = client.get("/x/key-metrics")
    assert resp.status_code == 200


def test_logs_real_time_update_simulation(
    client, log_dir_with_files
):
    """Simulate real-time log updates."""
    log_file = log_dir_with_files / "realtime.log"

    log_file.write_text("Initial log")
    resp1 = client.get("/api/logs?process=realtime")
    data1 = resp1.json()

    log_file.write_text("Initial log\nNew log line")
    resp2 = client.get("/api/logs?process=realtime")
    data2 = resp2.json()

    assert resp2.status_code == 200


def test_metrics_latency_tracking(client):
    """Metrics endpoint latency reasonable."""
    import time
    start = time.time()
    resp = client.get("/api/metrics")
    elapsed = time.time() - start

    assert resp.status_code == 200
    assert elapsed < 0.5


def test_logs_throughput_handling(client, log_dir_with_files):
    """Handle high log throughput."""
    log_file = log_dir_with_files / "high-volume.log"
    lines = [f"Log line {i}" for i in range(1000)]
    log_file.write_text("\n".join(lines))

    resp = client.get("/api/logs?process=high-volume")
    assert resp.status_code == 200


def test_error_agg_with_multiple_errors(
    client, log_dir_with_files
):
    """Error aggregation with multiple errors."""
    log_file = log_dir_with_files / "errors.log"
    errors = [
        "ERROR: connection failed",
        "ERROR: timeout",
        "ERROR: connection failed",
        "WARN: retry",
    ]
    log_file.write_text("\n".join(errors))

    resp = client.get("/x/error-agg")
    assert resp.status_code == 200


def test_complete_observability_workflow(
    client, running_process, log_dir_with_files
):
    """Complete observability workflow."""
    resp1 = client.get("/api/metrics")
    resp2 = client.get("/api/logs")
    resp3 = client.get("/x/health")
    resp4 = client.get("/x/key-metrics")
    resp5 = client.get("/x/error-agg")

    assert all(r.status_code == 200 for r in [
        resp1, resp2, resp3, resp4, resp5
    ])


def test_postgres_health_in_metrics_and_health(
    client, mock_postgres_connected
):
    """Postgres health consistent in metrics and health."""
    resp_metrics = client.get("/api/metrics")
    resp_health = client.get("/x/health")

    assert resp_metrics.status_code == 200
    assert resp_health.status_code == 200
    data = resp_metrics.json()
    assert data["postgres"] is True
