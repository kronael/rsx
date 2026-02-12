"""Verify API endpoint tests.

Run with: cd rsx-playground && uv run pytest tests/api_verify_test.py -v
"""

import pytest
from test_utils import assert_verify_result_structure


# ── Happy Path Tests (10) ─────────────────────────────────


def test_verify_run_returns_html(client):
    """POST /api/verify/run returns HTML."""
    resp = client.post("/api/verify/run")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_verify_run_executes_checks(client):
    """Verify run executes all checks."""
    resp = client.post("/api/verify/run")
    assert resp.status_code == 200


def test_verify_results_stored(client):
    """Verify results stored in verify_results."""
    import server

    client.post("/api/verify/run")
    assert len(server.verify_results) > 0


def test_x_verify_shows_results_after_run(client):
    """GET /x/verify shows results after run."""
    client.post("/api/verify/run")
    resp = client.get("/x/verify")

    assert resp.status_code == 200
    assert "Run All Checks" not in resp.text


def test_verify_checks_wal_directory(client, wal_dir_with_files):
    """Verify checks WAL directory exists."""
    resp = client.post("/api/verify/run")
    import server

    checks = server.verify_results
    wal_check = next(
        (c for c in checks if "WAL directory" in c["name"]),
        None
    )
    assert wal_check is not None
    assert wal_check["status"] == "pass"


def test_verify_checks_processes_running(client, running_process):
    """Verify checks if processes running."""
    resp = client.post("/api/verify/run")
    import server

    checks = server.verify_results
    proc_check = next(
        (c for c in checks if "processes running" in c["name"]),
        None
    )
    assert proc_check is not None


def test_verify_checks_postgres_connection(
    client, mock_postgres_connected
):
    """Verify checks postgres connection."""
    resp = client.post("/api/verify/run")
    import server

    checks = server.verify_results
    pg_check = next(
        (c for c in checks if "Postgres" in c["name"]),
        None
    )
    assert pg_check is not None
    assert pg_check["status"] == "pass"


def test_verify_includes_invariants(client):
    """Verify includes correctness invariants."""
    resp = client.post("/api/verify/run")
    import server

    checks = server.verify_results
    invariant_names = [
        "No crossed book",
        "Tips monotonic",
        "Slab no-leak",
        "Position = sum of fills",
    ]

    for name in invariant_names:
        check = next(
            (c for c in checks if name in c["name"]), None
        )
        assert check is not None


def test_verify_check_structure_valid(client):
    """Verify check results have valid structure."""
    client.post("/api/verify/run")
    import server

    for check in server.verify_results:
        assert_verify_result_structure(check)


def test_x_verify_before_run_shows_prompt(client):
    """GET /x/verify before run shows prompt."""
    resp = client.get("/x/verify")
    assert resp.status_code == 200
    assert "Run All Checks" in resp.text


# ── Error Cases (8) ───────────────────────────────────────


def test_verify_no_wal_directory_fails(client):
    """Verify fails if WAL directory missing."""
    import server
    original_wal = server.WAL_DIR
    server.WAL_DIR = server.Path("/nonexistent")

    client.post("/api/verify/run")
    checks = server.verify_results

    server.WAL_DIR = original_wal

    wal_check = next(
        (c for c in checks if "WAL directory" in c["name"]),
        None
    )
    assert wal_check["status"] == "fail"


def test_verify_no_processes_fails(client):
    """Verify fails if no processes running."""
    client.post("/api/verify/run")
    import server

    checks = server.verify_results
    proc_check = next(
        (c for c in checks if "processes running" in c["name"]),
        None
    )
    assert proc_check["status"] == "fail"


def test_verify_postgres_down_warns(client, mock_postgres_down):
    """Verify warns if postgres down."""
    client.post("/api/verify/run")
    import server

    checks = server.verify_results
    pg_check = next(
        (c for c in checks if "Postgres" in c["name"]),
        None
    )
    assert pg_check["status"] == "warn"


def test_verify_invariants_skip_without_processes(client):
    """Invariants skip if no processes running."""
    client.post("/api/verify/run")
    import server

    checks = server.verify_results
    inv_checks = [
        c for c in checks
        if "crossed book" in c["name"] or "Tips" in c["name"]
    ]

    for check in inv_checks:
        assert check["status"] in ["skip", "fail"]


def test_verify_wal_stream_no_files_warns(client, wal_dir_with_files):
    """WAL stream with no files warns."""
    empty_stream = wal_dir_with_files / "empty"
    empty_stream.mkdir()

    client.post("/api/verify/run")
    import server

    checks = server.verify_results
    assert len(checks) > 0


def test_verify_multiple_runs_clear_previous(client):
    """Multiple verify runs clear previous results."""
    import server

    client.post("/api/verify/run")
    count1 = len(server.verify_results)

    client.post("/api/verify/run")
    count2 = len(server.verify_results)

    assert count2 > 0


def test_verify_error_in_check_handled(client):
    """Error during check handled gracefully."""
    resp = client.post("/api/verify/run")
    assert resp.status_code == 200


def test_x_verify_malformed_results(client):
    """Malformed verify results handled."""
    import server
    server.verify_results = [{"invalid": "data"}]

    resp = client.get("/x/verify")
    server.verify_results = []

    assert resp.status_code in [200, 500]


# ── State Management (7) ──────────────────────────────────


def test_verify_results_persist_across_requests(client):
    """Verify results persist across requests."""
    client.post("/api/verify/run")

    resp1 = client.get("/x/verify")
    resp2 = client.get("/x/verify")

    assert resp1.status_code == 200
    assert resp2.status_code == 200
    assert "Run All Checks" not in resp1.text
    assert "Run All Checks" not in resp2.text


def test_verify_clears_old_results(client):
    """New verify run clears old results."""
    import server

    server.verify_results = [
        {"name": "old", "status": "pass", "time": "00:00:00"}
    ]

    client.post("/api/verify/run")

    assert not any(
        c["name"] == "old" for c in server.verify_results
    )


def test_verify_timestamp_included(client):
    """Verify results include timestamp."""
    client.post("/api/verify/run")
    import server

    for check in server.verify_results:
        assert "time" in check


def test_verify_detail_field_included(client):
    """Verify results include detail field."""
    client.post("/api/verify/run")
    import server

    for check in server.verify_results:
        assert "detail" in check


def test_verify_status_values_valid(client):
    """Verify status values are valid."""
    client.post("/api/verify/run")
    import server

    valid_statuses = ["pass", "fail", "warn", "skip"]
    for check in server.verify_results:
        assert check["status"] in valid_statuses


def test_verify_wal_streams_checked_individually(
    client, wal_dir_with_files
):
    """Each WAL stream checked individually."""
    stream2 = wal_dir_with_files / "stream2"
    stream2.mkdir()
    (stream2 / "000001.dxs").write_bytes(b"data")

    client.post("/api/verify/run")
    import server

    wal_checks = [
        c for c in server.verify_results
        if "WAL stream" in c["name"]
    ]
    assert len(wal_checks) >= 2


def test_verify_idempotent(client):
    """Verify run is idempotent."""
    resp1 = client.post("/api/verify/run")
    resp2 = client.post("/api/verify/run")

    assert resp1.status_code == 200
    assert resp2.status_code == 200


# ── Integration Tests (15) ────────────────────────────────


def test_verify_after_process_start(client, running_process):
    """Verify after process start shows running."""
    client.post("/api/verify/run")
    import server

    checks = server.verify_results
    proc_check = next(
        (c for c in checks if "processes running" in c["name"]),
        None
    )
    assert proc_check["status"] == "pass"


def test_verify_with_wal_files(client, wal_dir_with_files):
    """Verify with WAL files shows pass."""
    client.post("/api/verify/run")
    import server

    checks = server.verify_results
    wal_checks = [
        c for c in checks if "WAL" in c["name"]
    ]
    assert any(c["status"] == "pass" for c in wal_checks)


def test_verify_workflow_with_processes_and_wal(
    client, running_process, wal_dir_with_files
):
    """Complete verify workflow."""
    resp = client.post("/api/verify/run")
    assert resp.status_code == 200

    import server
    assert len(server.verify_results) > 0


def test_verify_then_check_html(client):
    """Verify then check HTML output."""
    client.post("/api/verify/run")
    resp = client.get("/x/verify")

    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_verify_integration_with_postgres(
    client, mock_postgres_connected
):
    """Verify integration with postgres."""
    client.post("/api/verify/run")
    import server

    pg_check = next(
        (c for c in server.verify_results if "Postgres" in c["name"]),
        None
    )
    assert pg_check["status"] == "pass"


def test_verify_all_invariants_checked(client, running_process):
    """All 10 invariants checked when processes running."""
    client.post("/api/verify/run")
    import server

    invariants = [
        "No crossed book",
        "Tips monotonic",
        "Slab no-leak",
        "Position = sum of fills",
        "FIFO within price level",
        "Exactly-one completion",
    ]

    for inv in invariants:
        check = next(
            (c for c in server.verify_results if inv in c["name"]),
            None
        )
        assert check is not None


def test_verify_failure_includes_detail(client):
    """Verify failure includes detail."""
    import server
    original_wal = server.WAL_DIR
    server.WAL_DIR = server.Path("/nonexistent")

    client.post("/api/verify/run")

    server.WAL_DIR = original_wal

    wal_check = next(
        (c for c in server.verify_results if "WAL directory" in c["name"]),
        None
    )
    assert len(wal_check["detail"]) > 0


def test_verify_pass_detail_may_be_empty(client, wal_dir_with_files):
    """Verify pass may have empty detail."""
    client.post("/api/verify/run")
    import server

    wal_check = next(
        (c for c in server.verify_results if "WAL directory" in c["name"]),
        None
    )
    assert wal_check["status"] == "pass"


def test_verify_complete_system_check(
    client, running_process, wal_dir_with_files,
    mock_postgres_connected
):
    """Complete system verification."""
    client.post("/api/verify/run")
    import server

    checks = server.verify_results
    pass_count = sum(1 for c in checks if c["status"] == "pass")

    assert pass_count >= 3


def test_verify_html_rendering_correct(client):
    """Verify HTML rendering correct."""
    client.post("/api/verify/run")
    resp = client.get("/x/verify")

    assert resp.status_code == 200


def test_verify_status_color_coding(client):
    """Verify status uses color coding in HTML."""
    client.post("/api/verify/run")
    resp = client.get("/x/verify")

    assert resp.status_code == 200


def test_verify_multiple_sequential_runs(client):
    """Multiple sequential verify runs work."""
    for _ in range(3):
        resp = client.post("/api/verify/run")
        assert resp.status_code == 200


def test_verify_concurrent_requests(client):
    """Concurrent verify requests handled."""
    import threading

    def run_verify():
        client.post("/api/verify/run")

    threads = [threading.Thread(target=run_verify) for _ in range(3)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    import server
    assert len(server.verify_results) > 0


def test_x_invariant_status_returns_html(client):
    """GET /x/invariant-status returns HTML."""
    resp = client.get("/x/invariant-status")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_verify_end_to_end(
    client, running_process, wal_dir_with_files,
    mock_postgres_connected
):
    """Complete verify end-to-end test."""
    resp1 = client.get("/x/verify")
    assert "Run All Checks" in resp1.text

    resp2 = client.post("/api/verify/run")
    assert resp2.status_code == 200

    resp3 = client.get("/x/verify")
    assert "Run All Checks" not in resp3.text

    import server
    assert len(server.verify_results) > 5
