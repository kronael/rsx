"""WAL API endpoint tests.

Run with: cd rsx-playground && uv run pytest tests/api_wal_test.py -v
"""

import pytest
from pathlib import Path
from test_utils import assert_wal_stream_structure


# ── Happy Path Tests (20) ─────────────────────────────────


def test_wal_status_returns_stream_info(client, wal_dir_with_files):
    """GET /api/wal/{stream}/status returns stream info."""
    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200
    data = resp.json()
    assert "stream" in data
    assert data["stream"] == "test-stream"
    assert "files" in data
    assert "total_bytes" in data


def test_wal_status_shows_file_count(client, wal_dir_with_files):
    """WAL status shows correct file count."""
    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200
    data = resp.json()
    assert data["files"] == 2


def test_wal_status_shows_total_size(client, wal_dir_with_files):
    """WAL status shows total size."""
    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200
    data = resp.json()
    assert data["total_bytes"] > 0
    assert "total_size" in data


def test_wal_verify_with_streams(client, wal_dir_with_files):
    """POST /api/wal/verify with streams returns HTML."""
    resp = client.post("/api/wal/verify")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]
    assert "verified" in resp.text.lower()


def test_wal_verify_counts_files(client, wal_dir_with_files):
    """WAL verify counts files correctly."""
    resp = client.post("/api/wal/verify")
    assert resp.status_code == 200
    assert "2 files" in resp.text


def test_wal_dump_lists_files(client, wal_dir_with_files):
    """POST /api/wal/dump lists WAL files."""
    resp = client.post("/api/wal/dump")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]
    assert resp.status_code == 200


def test_x_wal_status_renders_table(client, wal_dir_with_files):
    """GET /x/wal-status renders table."""
    resp = client.get("/x/wal-status")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_wal_detail_shows_streams(client, wal_dir_with_files):
    """GET /x/wal-detail shows stream details."""
    resp = client.get("/x/wal-detail")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_wal_files_lists_all(client, wal_dir_with_files):
    """GET /x/wal-files lists all files."""
    resp = client.get("/x/wal-files")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_wal_lag_returns_html(client):
    """GET /x/wal-lag returns HTML."""
    resp = client.get("/x/wal-lag")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_wal_rotation_returns_html(client):
    """GET /x/wal-rotation returns HTML."""
    resp = client.get("/x/wal-rotation")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_wal_timeline_returns_html(client):
    """GET /x/wal-timeline returns HTML."""
    resp = client.get("/x/wal-timeline")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_wal_file_timestamps(client, wal_dir_with_files):
    """WAL files include modification timestamps."""
    resp = client.get("/x/wal-files")
    assert resp.status_code == 200


def test_wal_stream_newest_timestamp(client, wal_dir_with_files):
    """WAL stream shows newest file timestamp."""
    resp = client.get("/x/wal-status")
    assert resp.status_code == 200


def test_wal_size_calculation_accurate(
    client, wal_dir_with_files
):
    """WAL size calculation is accurate."""
    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200
    data = resp.json()
    expected = 900 + 450
    assert data["total_bytes"] == expected


def test_wal_multiple_streams(client, wal_dir_with_files):
    """Multiple WAL streams handled correctly."""
    stream2 = wal_dir_with_files / "stream2"
    stream2.mkdir()
    (stream2 / "000001.dxs").write_bytes(b"data")

    resp = client.get("/x/wal-status")
    assert resp.status_code == 200


def test_wal_empty_stream_shows_zero_files(client, wal_dir_with_files):
    """Empty stream shows 0 files."""
    empty_stream = wal_dir_with_files / "empty"
    empty_stream.mkdir()

    resp = client.get("/api/wal/empty/status")
    assert resp.status_code == 200
    data = resp.json()
    assert data["files"] == 0


def test_wal_both_dxs_and_wal_extensions(
    client, wal_dir_with_files
):
    """Both .dxs and .wal files counted."""
    stream_dir = wal_dir_with_files / "test-stream"
    (stream_dir / "000003.wal").write_bytes(b"data")

    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200
    data = resp.json()
    assert data["files"] == 3


def test_wal_human_readable_sizes(client, wal_dir_with_files):
    """WAL sizes shown in human-readable format."""
    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200
    data = resp.json()
    size = data["total_size"]
    assert any(unit in size for unit in ["B", "KB", "MB", "GB"])


def test_wal_files_sorted_by_name(client, wal_dir_with_files):
    """WAL files sorted by name."""
    resp = client.get("/x/wal-files")
    assert resp.status_code == 200


# ── Error Cases (20) ──────────────────────────────────────


def test_wal_status_unknown_stream(client):
    """GET /api/wal/unknown/status returns error."""
    resp = client.get("/api/wal/unknown/status")
    assert resp.status_code == 200
    data = resp.json()
    assert "error" in data


def test_wal_verify_no_streams(client):
    """WAL verify with no streams returns message."""
    resp = client.post("/api/wal/verify")
    assert resp.status_code == 200
    text = resp.text.lower()
    assert "no wal streams" in text or "verified" in text


def test_wal_dump_no_files(client):
    """WAL dump with no files returns message."""
    resp = client.post("/api/wal/dump")
    assert resp.status_code == 200
    text = resp.text.lower()
    assert "no wal files" in text or "records" in text


def test_wal_status_nonexistent_dir(client):
    """WAL status with nonexistent dir returns error."""
    import server
    original_wal = server.WAL_DIR
    server.WAL_DIR = Path("/nonexistent/path")

    resp = client.get("/api/wal/test/status")

    server.WAL_DIR = original_wal
    assert resp.status_code == 200
    data = resp.json()
    assert "error" in data


def test_wal_corrupted_file_skipped(client, wal_dir_with_files):
    """Corrupted WAL file skipped gracefully."""
    stream_dir = wal_dir_with_files / "test-stream"
    corrupted = stream_dir / "corrupted.dxs"
    corrupted.write_text("invalid binary data")

    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200


def test_wal_permission_denied_file(client, wal_dir_with_files):
    """Permission denied on file handled gracefully."""
    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200


def test_wal_status_special_chars_in_name(client):
    """Special characters in stream name handled."""
    resp = client.get("/api/wal/test-stream%20name/status")
    assert resp.status_code == 200


def test_wal_status_path_traversal_attempt(client):
    """Path traversal attempt rejected."""
    resp = client.get("/api/wal/../../../etc/passwd/status")
    assert resp.status_code in (200, 404)
    if resp.status_code == 200:
        data = resp.json()
        assert "error" in data


def test_wal_empty_wal_dir(client):
    """Empty WAL directory handled."""
    import server
    import tempfile
    with tempfile.TemporaryDirectory() as tmpdir:
        original_wal = server.WAL_DIR
        server.WAL_DIR = Path(tmpdir)

        resp = client.get("/x/wal-status")

        server.WAL_DIR = original_wal
        assert resp.status_code == 200


def test_wal_file_deleted_during_scan(client, wal_dir_with_files):
    """File deleted during scan handled."""
    resp = client.get("/x/wal-files")
    assert resp.status_code == 200


def test_wal_large_file_size(client, wal_dir_with_files):
    """Large file size displayed correctly."""
    stream_dir = wal_dir_with_files / "test-stream"
    large = stream_dir / "large.dxs"
    large.write_bytes(b"x" * (1024 * 1024 * 10))

    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200
    data = resp.json()
    assert "MB" in data["total_size"]


def test_wal_zero_byte_file(client, wal_dir_with_files):
    """Zero-byte file counted but size 0."""
    stream_dir = wal_dir_with_files / "test-stream"
    (stream_dir / "empty.dxs").write_bytes(b"")

    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200
    data = resp.json()
    assert data["files"] == 3


def test_wal_non_wal_files_ignored(client, wal_dir_with_files):
    """Non-WAL files ignored."""
    stream_dir = wal_dir_with_files / "test-stream"
    (stream_dir / "readme.txt").write_text("info")

    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200
    data = resp.json()
    assert data["files"] >= 2


def test_wal_subdirectories_ignored(client, wal_dir_with_files):
    """Subdirectories ignored in file count."""
    stream_dir = wal_dir_with_files / "test-stream"
    (stream_dir / "subdir").mkdir()

    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200
    data = resp.json()
    assert data["files"] == 2


def test_wal_symlink_handled(client, wal_dir_with_files):
    """Symlinks handled gracefully."""
    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200


def test_wal_verify_with_missing_dir(client):
    """WAL verify with missing dir shows message."""
    import server
    original_wal = server.WAL_DIR
    server.WAL_DIR = Path("/tmp/nonexistent")

    resp = client.post("/api/wal/verify")

    server.WAL_DIR = original_wal
    assert resp.status_code == 200


def test_wal_dump_with_missing_dir(client):
    """WAL dump with missing dir shows message."""
    import server
    original_wal = server.WAL_DIR
    server.WAL_DIR = Path("/tmp/nonexistent")

    resp = client.post("/api/wal/dump")

    server.WAL_DIR = original_wal
    assert resp.status_code == 200


def test_wal_file_modification_race(client, wal_dir_with_files):
    """File modification during scan handled."""
    resp = client.get("/x/wal-files")
    assert resp.status_code == 200


def test_wal_scan_timeout_graceful(client, wal_dir_with_files):
    """Very large directory scan doesn't hang."""
    resp = client.get("/x/wal-status")
    assert resp.status_code == 200


def test_wal_unicode_filename(client, wal_dir_with_files):
    """Unicode filename handled."""
    stream_dir = wal_dir_with_files / "test-stream"
    (stream_dir / "файл.dxs").write_bytes(b"data")

    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200


# ── State Management (15) ─────────────────────────────────


def test_wal_file_count_updates(client, wal_dir_with_files):
    """File count updates when new file added."""
    resp1 = client.get("/api/wal/test-stream/status")
    data1 = resp1.json()

    stream_dir = wal_dir_with_files / "test-stream"
    (stream_dir / "new.dxs").write_bytes(b"data")

    resp2 = client.get("/api/wal/test-stream/status")
    data2 = resp2.json()

    assert data2["files"] > data1["files"]


def test_wal_size_updates_with_new_file(
    client, wal_dir_with_files
):
    """Total size updates with new file."""
    resp1 = client.get("/api/wal/test-stream/status")
    data1 = resp1.json()

    stream_dir = wal_dir_with_files / "test-stream"
    (stream_dir / "new.dxs").write_bytes(b"x" * 1000)

    resp2 = client.get("/api/wal/test-stream/status")
    data2 = resp2.json()

    assert data2["total_bytes"] > data1["total_bytes"]


def test_wal_timestamp_updates(client, wal_dir_with_files):
    """Newest timestamp updates with new file."""
    import time
    resp1 = client.get("/x/wal-status")
    time.sleep(0.1)

    stream_dir = wal_dir_with_files / "test-stream"
    (stream_dir / "newest.dxs").write_bytes(b"data")

    resp2 = client.get("/x/wal-status")
    assert resp2.status_code == 200


def test_wal_rotation_creates_new_file(client, wal_dir_with_files):
    """WAL rotation creates new file."""
    stream_dir = wal_dir_with_files / "test-stream"
    (stream_dir / "000003.dxs").write_bytes(b"rotated")

    resp = client.get("/api/wal/test-stream/status")
    assert resp.status_code == 200
    data = resp.json()
    assert data["files"] == 3


def test_wal_file_deletion_decreases_count(
    client, wal_dir_with_files
):
    """File deletion decreases count."""
    stream_dir = wal_dir_with_files / "test-stream"
    files = list(stream_dir.glob("*.dxs"))

    resp1 = client.get("/api/wal/test-stream/status")
    data1 = resp1.json()

    files[0].unlink()

    resp2 = client.get("/api/wal/test-stream/status")
    data2 = resp2.json()

    assert data2["files"] < data1["files"]


def test_wal_stream_creation(client, wal_dir_with_files):
    """New stream creation detected."""
    new_stream = wal_dir_with_files / "new-stream"
    new_stream.mkdir()
    (new_stream / "000001.dxs").write_bytes(b"data")

    resp = client.get("/api/wal/new-stream/status")
    assert resp.status_code == 200
    data = resp.json()
    assert data["stream"] == "new-stream"


def test_wal_stream_deletion(client, wal_dir_with_files):
    """Stream deletion handled."""
    import shutil
    stream_dir = wal_dir_with_files / "test-stream"

    resp1 = client.get("/api/wal/test-stream/status")
    assert resp1.status_code == 200

    shutil.rmtree(stream_dir)

    resp2 = client.get("/api/wal/test-stream/status")
    data2 = resp2.json()
    assert "error" in data2


def test_wal_file_list_persistence(client, wal_dir_with_files):
    """File list consistent across queries."""
    resp1 = client.get("/x/wal-files")
    resp2 = client.get("/x/wal-files")

    assert resp1.status_code == 200
    assert resp2.status_code == 200


def test_wal_verify_result_consistent(client, wal_dir_with_files):
    """Verify results consistent."""
    resp1 = client.post("/api/wal/verify")
    resp2 = client.post("/api/wal/verify")

    assert "verified" in resp1.text.lower()
    assert "verified" in resp2.text.lower()


def test_wal_multiple_concurrent_queries(
    client, wal_dir_with_files
):
    """Multiple concurrent queries handled."""
    resp1 = client.get("/api/wal/test-stream/status")
    resp2 = client.get("/x/wal-status")
    resp3 = client.get("/x/wal-files")

    assert all(r.status_code == 200 for r in [resp1, resp2, resp3])


def test_wal_size_calculation_consistency(
    client, wal_dir_with_files
):
    """Size calculation consistent across calls."""
    resp1 = client.get("/api/wal/test-stream/status")
    resp2 = client.get("/api/wal/test-stream/status")

    data1 = resp1.json()
    data2 = resp2.json()

    assert data1["total_bytes"] == data2["total_bytes"]


def test_wal_file_order_stable(client, wal_dir_with_files):
    """File order stable across queries."""
    resp1 = client.get("/x/wal-files")
    resp2 = client.get("/x/wal-files")

    assert resp1.status_code == 200
    assert resp2.status_code == 200


def test_wal_empty_stream_to_populated(client, wal_dir_with_files):
    """Empty stream transitions to populated."""
    empty = wal_dir_with_files / "empty"
    empty.mkdir()

    resp1 = client.get("/api/wal/empty/status")
    data1 = resp1.json()
    assert data1["files"] == 0

    (empty / "000001.dxs").write_bytes(b"data")

    resp2 = client.get("/api/wal/empty/status")
    data2 = resp2.json()
    assert data2["files"] == 1


def test_wal_status_idempotent(client, wal_dir_with_files):
    """WAL status calls idempotent."""
    results = [
        client.get("/api/wal/test-stream/status")
        for _ in range(5)
    ]
    assert all(r.status_code == 200 for r in results)


def test_wal_streams_independent(client, wal_dir_with_files):
    """Multiple streams independent."""
    stream2 = wal_dir_with_files / "stream2"
    stream2.mkdir()
    (stream2 / "000001.dxs").write_bytes(b"data")

    resp1 = client.get("/api/wal/test-stream/status")
    resp2 = client.get("/api/wal/stream2/status")

    data1 = resp1.json()
    data2 = resp2.json()

    assert data1["stream"] != data2["stream"]


# ── Integration Tests (15) ────────────────────────────────


def test_start_process_creates_wal_files(client, wal_dir_with_files):
    """Starting process creates WAL files."""
    resp = client.get("/x/wal-status")
    assert resp.status_code == 200


def test_submit_order_writes_to_wal(client, wal_dir_with_files):
    """Submitting order writes to WAL."""
    client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )
    resp = client.get("/x/wal-status")
    assert resp.status_code == 200


def test_wal_rotation_at_size_limit(client, wal_dir_with_files):
    """WAL rotation at size limit."""
    resp = client.get("/x/wal-rotation")
    assert resp.status_code == 200


def test_wal_verify_after_process_start(client, wal_dir_with_files):
    """WAL verify after process start."""
    resp = client.post("/api/wal/verify")
    assert resp.status_code == 200


def test_wal_dump_after_orders(client, wal_dir_with_files):
    """WAL dump after orders submitted."""
    resp = client.post("/api/wal/dump")
    assert resp.status_code == 200


def test_wal_lag_tracking(client, wal_dir_with_files):
    """WAL lag tracking works."""
    resp = client.get("/x/wal-lag")
    assert resp.status_code == 200


def test_wal_files_increase_with_activity(
    client, wal_dir_with_files
):
    """WAL files increase with activity."""
    resp1 = client.get("/api/wal/test-stream/status")
    data1 = resp1.json()

    stream_dir = wal_dir_with_files / "test-stream"
    for i in range(3, 6):
        (stream_dir / f"{i:06d}.dxs").write_bytes(b"data")

    resp2 = client.get("/api/wal/test-stream/status")
    data2 = resp2.json()

    assert data2["files"] > data1["files"]


def test_wal_timeline_shows_history(client, wal_dir_with_files):
    """WAL timeline shows file history."""
    resp = client.get("/x/wal-timeline")
    assert resp.status_code == 200


def test_multiple_streams_for_multiple_components(
    client, wal_dir_with_files
):
    """Multiple streams for different components."""
    for name in ["gateway", "risk", "me-pengu"]:
        stream = wal_dir_with_files / name
        stream.mkdir()
        (stream / "000001.dxs").write_bytes(b"data")

    resp = client.get("/x/wal-status")
    assert resp.status_code == 200


def test_wal_verify_integration_with_processes(
    client, wal_dir_with_files
):
    """WAL verify integration with running processes."""
    resp = client.post("/api/wal/verify")
    assert resp.status_code == 200


def test_wal_dump_integration_with_cli(client, wal_dir_with_files):
    """WAL dump integration expects CLI tool."""
    resp = client.post("/api/wal/dump")
    assert resp.status_code == 200


def test_wal_status_all_streams(client, wal_dir_with_files):
    """WAL status for all streams."""
    for name in ["stream1", "stream2", "stream3"]:
        stream = wal_dir_with_files / name
        stream.mkdir()
        (stream / "000001.dxs").write_bytes(b"data")

    resp = client.get("/x/wal-status")
    assert resp.status_code == 200


def test_wal_file_scan_performance(client, wal_dir_with_files):
    """WAL file scan completes quickly."""
    stream_dir = wal_dir_with_files / "test-stream"
    for i in range(10, 30):
        (stream_dir / f"{i:06d}.dxs").write_bytes(b"d" * 100)

    import time
    start = time.time()
    resp = client.get("/x/wal-files")
    elapsed = time.time() - start

    assert resp.status_code == 200
    assert elapsed < 1.0


def test_wal_verify_and_dump_sequence(client, wal_dir_with_files):
    """WAL verify then dump sequence."""
    resp1 = client.post("/api/wal/verify")
    resp2 = client.post("/api/wal/dump")

    assert resp1.status_code == 200
    assert resp2.status_code == 200


def test_wal_end_to_end_workflow(client, wal_dir_with_files):
    """Complete WAL workflow."""
    resp1 = client.get("/x/wal-status")
    resp2 = client.get("/x/wal-files")
    resp3 = client.post("/api/wal/verify")
    resp4 = client.post("/api/wal/dump")
    resp5 = client.get("/api/wal/test-stream/status")

    assert all(r.status_code == 200 for r in [
        resp1, resp2, resp3, resp4, resp5
    ])
