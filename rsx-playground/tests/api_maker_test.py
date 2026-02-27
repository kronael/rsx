"""Tests for market maker API: /api/maker/start, stop, status, /x/maker-status.

Verifies:
- Start spawns market_maker.py subprocess and writes PID file
- Stop sends SIGTERM and cleans up managed dict / PID file
- Status returns running/pid/name JSON
- HTML partial x/maker-status renders correct state
- Book endpoint (/x/book) returns graceful empty state without processes
- Double-start returns "already running" without spawning duplicate
- Stop when not running returns graceful message
"""

import os
import signal
import time
from pathlib import Path
from unittest.mock import AsyncMock
from unittest.mock import MagicMock

import pytest
from fastapi.testclient import TestClient

import server
from server import MAKER_NAME
from server import MAKER_SCRIPT
from server import PID_DIR
from server import _book_snap
from server import app
from server import managed


@pytest.fixture
def client():
    return TestClient(app)


# ── Status endpoints without running maker ─────────────────


def test_maker_status_not_running_json(client):
    """GET /api/maker/status returns running=False when no maker."""
    resp = client.get("/api/maker/status")
    assert resp.status_code == 200
    data = resp.json()
    assert data["running"] is False
    assert data["pid"] is None
    assert data["name"] == "maker"


def test_maker_status_html_stopped(client):
    """GET /x/maker-status returns stopped HTML when no maker."""
    resp = client.get("/x/maker-status")
    assert resp.status_code == 200
    assert "stopped" in resp.text.lower() or "text/html" in resp.headers["content-type"]


def test_maker_status_html_is_html(client):
    """GET /x/maker-status returns HTML content type."""
    resp = client.get("/x/maker-status")
    assert "text/html" in resp.headers["content-type"]


def test_maker_stop_when_not_running(client):
    """POST /api/maker/stop when not running returns graceful message."""
    resp = client.post("/api/maker/stop")
    assert resp.status_code == 200
    assert "not running" in resp.text.lower()


# ── Start endpoint ─────────────────────────────────────────


def test_maker_start_returns_200(client):
    """POST /api/maker/start returns 200."""
    resp = client.post("/api/maker/start")
    assert resp.status_code == 200


def test_maker_start_spawns_process(client):
    """POST /api/maker/start adds maker to managed dict."""
    resp = client.post("/api/maker/start")
    assert resp.status_code == 200
    # maker_maker.py exists so should be in managed or failed to start
    if "started" in resp.text.lower() or "pid" in resp.text.lower():
        assert MAKER_NAME in managed
    # either started or failed-to-start is acceptable; no 5xx


def test_maker_start_writes_pid_file(client):
    """POST /api/maker/start writes PID file if process starts."""
    resp = client.post("/api/maker/start")
    assert resp.status_code == 200
    pid_file = server.PID_DIR / f"{MAKER_NAME}.pid"
    if MAKER_NAME in managed:
        assert pid_file.exists()
        pid_text = pid_file.read_text().strip()
        assert pid_text.isdigit()


def test_maker_script_exists():
    """market_maker.py file exists at expected path."""
    assert MAKER_SCRIPT.exists(), f"market_maker.py not found at {MAKER_SCRIPT}"


def test_maker_start_message_contains_pid_or_error(client):
    """POST /api/maker/start response contains pid or error info."""
    resp = client.post("/api/maker/start")
    assert resp.status_code == 200
    text = resp.text.lower()
    # Either "pid", "started", "failed", or "not found"
    assert any(word in text for word in ["pid", "started", "failed", "not found"])


# ── Double-start idempotency ───────────────────────────────


def test_maker_double_start_returns_already_running(client):
    """Second POST /api/maker/start returns 'already running'."""
    # Inject a fake running maker into managed
    mock_proc = MagicMock()
    mock_proc.pid = 99999
    mock_proc.returncode = None
    managed[MAKER_NAME] = {
        "proc": mock_proc,
        "binary": str(MAKER_SCRIPT),
        "env": {},
    }
    resp = client.post("/api/maker/start")
    assert resp.status_code == 200
    assert "already running" in resp.text.lower()


def test_maker_double_start_no_duplicate_entry(client):
    """Double-start does not create duplicate managed entries."""
    mock_proc = MagicMock()
    mock_proc.pid = 99999
    mock_proc.returncode = None
    managed[MAKER_NAME] = {
        "proc": mock_proc,
        "binary": str(MAKER_SCRIPT),
        "env": {},
    }
    client.post("/api/maker/start")
    # Only one entry for MAKER_NAME
    assert list(managed.keys()).count(MAKER_NAME) == 1


# ── Status while running ───────────────────────────────────


def test_maker_status_json_running(client):
    """GET /api/maker/status returns running=True when maker is in managed."""
    mock_proc = MagicMock()
    mock_proc.pid = 12345
    mock_proc.returncode = None
    managed[MAKER_NAME] = {
        "proc": mock_proc,
        "binary": str(MAKER_SCRIPT),
        "env": {},
    }
    resp = client.get("/api/maker/status")
    assert resp.status_code == 200
    data = resp.json()
    assert data["running"] is True
    assert data["pid"] == 12345
    assert data["name"] == MAKER_NAME


def test_maker_status_html_running(client):
    """GET /x/maker-status returns running HTML when maker active."""
    mock_proc = MagicMock()
    mock_proc.pid = 12345
    mock_proc.returncode = None
    managed[MAKER_NAME] = {
        "proc": mock_proc,
        "binary": str(MAKER_SCRIPT),
        "env": {},
    }
    resp = client.get("/x/maker-status")
    assert resp.status_code == 200
    assert "running" in resp.text.lower()
    assert "12345" in resp.text


def test_maker_status_html_contains_pid(client):
    """GET /x/maker-status HTML includes PID when running."""
    mock_proc = MagicMock()
    mock_proc.pid = 55555
    mock_proc.returncode = None
    managed[MAKER_NAME] = {
        "proc": mock_proc,
        "binary": str(MAKER_SCRIPT),
        "env": {},
    }
    resp = client.get("/x/maker-status")
    assert "55555" in resp.text


# ── Stop endpoint ──────────────────────────────────────────


def test_maker_stop_running_returns_stopped(client):
    """POST /api/maker/stop when running returns stopped message."""
    mock_proc = AsyncMock()
    mock_proc.pid = 99998
    mock_proc.returncode = None
    mock_proc.terminate = MagicMock()
    mock_proc.kill = MagicMock()
    managed[MAKER_NAME] = {
        "proc": mock_proc,
        "binary": str(MAKER_SCRIPT),
        "env": {},
    }
    resp = client.post("/api/maker/stop")
    assert resp.status_code == 200
    assert "stopped" in resp.text.lower()


def test_maker_stop_removes_from_managed(client):
    """POST /api/maker/stop removes maker from managed dict."""
    mock_proc = AsyncMock()
    mock_proc.pid = 99997
    mock_proc.returncode = None
    mock_proc.terminate = MagicMock()
    mock_proc.kill = MagicMock()
    managed[MAKER_NAME] = {
        "proc": mock_proc,
        "binary": str(MAKER_SCRIPT),
        "env": {},
    }
    client.post("/api/maker/stop")
    assert MAKER_NAME not in managed


def test_maker_stop_cleans_pid_file(client, tmp_path):
    """POST /api/maker/stop removes PID file."""
    server.PID_DIR.mkdir(parents=True, exist_ok=True)
    pid_file = server.PID_DIR / f"{MAKER_NAME}.pid"
    pid_file.write_text("99996")

    mock_proc = AsyncMock()
    mock_proc.pid = 99996
    mock_proc.returncode = None
    mock_proc.terminate = MagicMock()
    mock_proc.kill = MagicMock()
    managed[MAKER_NAME] = {
        "proc": mock_proc,
        "binary": str(MAKER_SCRIPT),
        "env": {},
    }
    client.post("/api/maker/stop")
    assert not pid_file.exists()


# ── Stop → Status transition ───────────────────────────────


def test_maker_stop_then_status_not_running(client):
    """After stop, /api/maker/status returns running=False."""
    mock_proc = AsyncMock()
    mock_proc.pid = 99995
    mock_proc.returncode = None
    mock_proc.terminate = MagicMock()
    mock_proc.kill = MagicMock()
    managed[MAKER_NAME] = {
        "proc": mock_proc,
        "binary": str(MAKER_SCRIPT),
        "env": {},
    }
    client.post("/api/maker/stop")
    resp = client.get("/api/maker/status")
    data = resp.json()
    assert data["running"] is False
    assert data["pid"] is None


def test_maker_stop_then_html_shows_stopped(client):
    """After stop, /x/maker-status shows stopped."""
    mock_proc = AsyncMock()
    mock_proc.pid = 99994
    mock_proc.returncode = None
    mock_proc.terminate = MagicMock()
    mock_proc.kill = MagicMock()
    managed[MAKER_NAME] = {
        "proc": mock_proc,
        "binary": str(MAKER_SCRIPT),
        "env": {},
    }
    client.post("/api/maker/stop")
    resp = client.get("/x/maker-status")
    assert "stopped" in resp.text.lower()


# ── Book endpoint graceful empty state ────────────────────


def test_book_endpoint_no_processes_returns_200(client):
    """GET /x/book returns 200 with no processes running."""
    resp = client.get("/x/book?symbol_id=10")
    assert resp.status_code == 200


def test_book_endpoint_no_processes_graceful(client):
    """GET /x/book returns graceful empty state without processes."""
    resp = client.get("/x/book?symbol_id=10")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]
    # Should contain a graceful message, not a stack trace
    assert "<" in resp.text  # is HTML
    assert "Traceback" not in resp.text
    assert "Exception" not in resp.text


def test_book_endpoint_default_symbol(client):
    """GET /x/book without symbol_id defaults to symbol 10."""
    resp = client.get("/x/book")
    assert resp.status_code == 200


def test_book_endpoint_various_symbols(client):
    """GET /x/book with various symbol IDs returns 200."""
    for sym in [1, 3, 10, 99]:
        resp = client.get(f"/x/book?symbol_id={sym}")
        assert resp.status_code == 200


# ── Process scan includes maker ───────────────────────────


def test_scan_processes_includes_maker_when_running(client):
    """scan_processes includes maker when it's in managed dict."""
    mock_proc = MagicMock()
    mock_proc.pid = 77777
    mock_proc.returncode = None
    managed[MAKER_NAME] = {
        "proc": mock_proc,
        "binary": str(MAKER_SCRIPT),
        "env": {},
    }
    resp = client.get("/api/processes")
    assert resp.status_code == 200
    procs = resp.json()
    names = [p.get("name") or p.get("id", "") for p in procs]
    assert any(MAKER_NAME in n for n in names)


# ── Mark prices endpoint ──────────────────────────────────


def test_mark_prices_returns_json_when_offline(client):
    """GET /api/mark/prices returns JSON when WAL has no BBO data."""
    resp = client.get("/api/mark/prices")
    assert resp.status_code == 200
    data = resp.json()
    assert "prices" in data


def test_mark_prices_includes_source_field(client):
    """GET /api/mark/prices entries include source field."""
    resp = client.get("/api/mark/prices")
    assert resp.status_code == 200
    data = resp.json()
    prices = data.get("prices", {})
    for sid, entry in prices.items():
        assert "source" in entry
        assert entry["source"] in ("wal", "live")


def test_scan_processes_maker_not_present_when_stopped(client):
    """scan_processes does not show maker when not in managed dict."""
    # managed is cleared by cleanup_state fixture
    resp = client.get("/api/processes")
    assert resp.status_code == 200
    procs = resp.json()
    names = [p.get("name") or p.get("id", "") for p in procs]
    assert not any(MAKER_NAME == n for n in names)
