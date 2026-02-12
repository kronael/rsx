"""Pytest configuration for rsx-playground tests."""

import sys
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock, patch

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
    """Clear in-memory state before each test."""
    server.recent_orders.clear()
    server.verify_results.clear()
    server.managed.clear()
    yield
    server.recent_orders.clear()
    server.verify_results.clear()
    server.managed.clear()


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
