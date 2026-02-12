"""Pytest configuration for rsx-playground tests."""

import os
import signal
import sys
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock, patch

import psutil
import pytest
from fastapi.testclient import TestClient

sys.path.insert(0, str(Path(__file__).parent.parent))

import server


@pytest.fixture
def client():
    """Create TestClient for server app."""
    return TestClient(server.app)


@pytest.fixture(autouse=True)
def cleanup_state():
    """Clear in-memory state before and after each test."""
    # Before test: clear state
    server.recent_orders.clear()
    server.verify_results.clear()
    server.managed.clear()

    yield

    # After test: cleanup everything
    server.recent_orders.clear()
    server.verify_results.clear()

    # Kill any managed processes
    for name, info in list(server.managed.items()):
        proc = info.get("proc")
        if proc and hasattr(proc, "pid"):
            try:
                os.kill(proc.pid, signal.SIGTERM)
                # Wait for process to terminate before continuing
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    # Force kill if graceful shutdown times out
                    try:
                        os.kill(proc.pid, signal.SIGKILL)
                        proc.wait(timeout=1)
                    except (ProcessLookupError, OSError):
                        pass
            except (ProcessLookupError, OSError):
                pass
    server.managed.clear()


@pytest.fixture(scope="session", autouse=True)
def cleanup_session():
    """Cleanup at session start and end."""
    # Before session: kill any stray RSX processes
    _kill_stray_processes()

    yield

    # After session: full cleanup
    _kill_stray_processes()
    _cleanup_test_files()


def _kill_stray_processes():
    """Kill any stray RSX processes from previous runs."""
    rsx_binaries = [
        "rsx-matching", "rsx-gateway", "rsx-risk",
        "rsx-marketdata", "rsx-mark", "rsx-recorder"
    ]

    for proc in psutil.process_iter(['pid', 'name', 'cmdline']):
        try:
            cmdline = proc.info.get('cmdline') or []
            cmdline_str = ' '.join(cmdline)

            if any(binary in cmdline_str for binary in rsx_binaries):
                print(f"Killing stray process: {proc.pid} {cmdline_str}")
                proc.kill()
                proc.wait(timeout=5)
        except (psutil.NoSuchProcess, psutil.AccessDenied, psutil.TimeoutExpired):
            pass


def _cleanup_test_files():
    """Clean up test-generated files."""
    import shutil

    root = Path(__file__).parent.parent.parent

    # Clean WAL files in tmp/wal/
    wal_dir = root / "tmp" / "wal"
    if wal_dir.exists():
        for item in wal_dir.iterdir():
            if item.is_dir():
                shutil.rmtree(item, ignore_errors=True)

    # Clean PID files in tmp/pids/
    pid_dir = root / "tmp" / "pids"
    if pid_dir.exists():
        for pid_file in pid_dir.glob("*.pid"):
            pid_file.unlink(missing_ok=True)

    # Clean test log files (keep production logs)
    log_dir = root / "log"
    if log_dir.exists():
        for log_file in log_dir.glob("*.log"):
            # Only clean if file is recent (test-generated)
            if log_file.stat().st_mtime > (os.path.getmtime(log_dir) - 3600):
                log_file.unlink(missing_ok=True)


@pytest.fixture
def mock_postgres_down():
    """Mock postgres as unavailable."""
    original_pool = server.pg_pool
    server.pg_pool = None
    yield
    server.pg_pool = original_pool


@pytest.fixture
def mock_postgres_connected():
    """Mock postgres as connected with a pool."""
    original_pool = server.pg_pool
    server.pg_pool = MagicMock()
    yield
    server.pg_pool = original_pool


@pytest.fixture
async def mock_pg_query_success():
    """Mock successful postgres query."""
    async def _query(*args):
        return [{"user_id": 1, "balance": 10000}]

    with patch.object(server, 'pg_query', side_effect=_query):
        yield


@pytest.fixture
async def mock_pg_query_error():
    """Mock postgres query error."""
    async def _query(*args):
        return {"error": "connection failed"}

    with patch.object(server, 'pg_query', side_effect=_query):
        yield


@pytest.fixture
def running_process():
    """Mock a running process in managed dict."""
    proc = AsyncMock()
    proc.pid = 12345
    proc.returncode = None

    server.managed["test-process"] = {
        "proc": proc,
        "binary": "./target/debug/test-binary",
        "env": {"TEST_ENV": "1"},
    }

    yield proc

    if "test-process" in server.managed:
        del server.managed["test-process"]


@pytest.fixture
def wal_dir_with_files(tmp_path):
    """Create temporary WAL directory with test files."""
    wal_dir = tmp_path / "wal"
    wal_dir.mkdir()

    stream_dir = wal_dir / "test-stream"
    stream_dir.mkdir()

    (stream_dir / "000001.dxs").write_bytes(b"test data" * 100)
    (stream_dir / "000002.dxs").write_bytes(b"test data" * 50)

    original_wal = server.WAL_DIR
    server.WAL_DIR = wal_dir

    yield wal_dir

    server.WAL_DIR = original_wal


@pytest.fixture
def log_dir_with_files(tmp_path):
    """Create temporary log directory with test files."""
    log_dir = tmp_path / "log"
    log_dir.mkdir()

    (log_dir / "gateway.log").write_text(
        "INFO: gateway started\n"
        "ERROR: connection failed\n"
        "WARN: retry attempt 1\n"
    )

    (log_dir / "risk.log").write_text(
        "INFO: risk engine ready\n"
        "DEBUG: position updated\n"
    )

    original_log = server.LOG_DIR
    server.LOG_DIR = log_dir

    yield log_dir

    server.LOG_DIR = original_log
