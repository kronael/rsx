# pyright: basic
"""Tests for market maker API: /api/maker/start, stop, restart, status,
/x/maker-status.

The maker is a process of the cluster scenario. These tests run the server's
routes over the in-memory cluster (`fake_cluster`) and check what the routes
ask of `rsx-cli cluster` and what they show.
"""

import json

import pytest
from fastapi.testclient import TestClient

import server
from server import MAKER_NAME, app


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
    assert "stopped" in resp.text.lower()


def test_maker_status_html_is_html(client):
    """GET /x/maker-status returns HTML content type."""
    resp = client.get("/x/maker-status")
    assert "text/html" in resp.headers["content-type"]


def test_maker_stop_when_not_running(client):
    """POST /api/maker/stop when not running returns graceful message."""
    resp = client.post("/api/maker/stop")
    assert resp.status_code == 200
    assert "not running" in resp.text.lower()


# ── Start ──────────────────────────────────────────────────


def test_maker_start_asks_the_cluster_for_the_maker(client, fake_cluster):
    """POST /api/maker/start runs `cluster up --only maker`."""
    resp = client.post("/api/maker/start")
    assert resp.status_code == 200
    assert ("up", "minimal", (MAKER_NAME,), False) in fake_cluster.calls
    pid = fake_cluster.running[MAKER_NAME]
    assert f"maker started (pid {pid})" in resp.text


def test_maker_start_failure_shows_the_reason(client, fake_cluster):
    """A maker that cannot start answers 200 with the CLI's reason."""
    fake_cluster.fail("maker exited during startup (pid 7)")
    resp = client.post("/api/maker/start")
    assert resp.status_code == 200
    assert "maker failed to start" in resp.text
    assert "exited during startup" in resp.text
    assert MAKER_NAME not in fake_cluster.running


def test_maker_double_start_returns_already_running(client, fake_cluster):
    """A second start does not ask the cluster again."""
    fake_cluster.started(MAKER_NAME)
    resp = client.post("/api/maker/start")
    assert resp.status_code == 200
    assert "already running" in resp.text.lower()
    assert not [call for call in fake_cluster.calls if call[0] == "up"]


# ── Status while running ───────────────────────────────────


def test_maker_status_json_running(client, fake_cluster):
    """GET /api/maker/status returns the maker's pid from the cluster."""
    fake_cluster.started(MAKER_NAME)
    resp = client.get("/api/maker/status")
    assert resp.status_code == 200
    data = resp.json()
    assert data["running"] is True
    assert data["pid"] == fake_cluster.running[MAKER_NAME]
    assert data["name"] == MAKER_NAME


def test_maker_status_html_contains_pid(client, fake_cluster):
    """GET /x/maker-status HTML includes the PID when running."""
    fake_cluster.started(MAKER_NAME)
    resp = client.get("/x/maker-status")
    assert resp.status_code == 200
    assert "running" in resp.text.lower()
    assert str(fake_cluster.running[MAKER_NAME]) in resp.text


def book(bids, asks):
    return {
        "bids": [{"px": 30000 - n * 10, "qty": 5} for n in range(bids)],
        "asks": [{"px": 30010 + n * 10, "qty": 5} for n in range(asks)],
    }


def test_maker_levels_come_from_the_book(client, fake_cluster, monkeypatch):
    """A 5x5 book reports 5 levels on /api/maker/status and /api/status."""
    monkeypatch.setattr(server.md_records, "books", {10: book(5, 5)})
    fake_cluster.started(MAKER_NAME)
    assert client.get("/api/maker/status").json()["levels"] == 5
    assert client.get("/api/status").json()["maker"]["levels"] == 5


def test_maker_levels_are_the_thinner_side(client, fake_cluster, monkeypatch):
    monkeypatch.setattr(
        server.md_records, "books", {10: book(5, 5), 3: book(4, 2)})
    fake_cluster.started(MAKER_NAME)
    assert client.get("/api/maker/status").json()["levels"] == 2


def test_a_stopped_maker_has_no_levels(client, monkeypatch):
    """The book outlives the maker's process; the status does not show it."""
    monkeypatch.setattr(server.md_records, "books", {10: book(5, 5)})
    data = client.get("/api/maker/status").json()
    assert data["running"] is False
    assert data["levels"] == 0
    assert client.get("/api/status").json()["maker"]["levels"] == 0


def test_the_maker_page_shows_the_book(client, fake_cluster, monkeypatch):
    """/x/maker-live and /x/maker-status show levels, mid and spread."""
    monkeypatch.setattr(server.md_records, "books", {10: book(5, 5)})
    fake_cluster.started(MAKER_NAME, "gw-0", "me-pengu")
    for route in "/x/maker-live", "/x/maker-status":
        text = client.get(route).text
        assert "5 bid / 5 ask" in text
        assert "spread 3.3 bps" in text
    topology = client.get("/x/topology/maker").text
    assert "sym10=5/5" in topology


def test_a_crashed_maker_is_not_running(client, fake_cluster):
    """An exited record reads as stopped, not running."""
    fake_cluster.started(MAKER_NAME)
    fake_cluster.crash(MAKER_NAME)
    assert client.get("/api/maker/status").json()["running"] is False


# ── Stop and restart ───────────────────────────────────────


def test_maker_stop_running_stops_only_the_maker(client, fake_cluster):
    """POST /api/maker/stop runs `cluster down --only maker`."""
    fake_cluster.started(MAKER_NAME, "gw-0")
    resp = client.post("/api/maker/stop")
    assert resp.status_code == 200
    assert "stopped" in resp.text.lower()
    assert ("down", (MAKER_NAME,)) in fake_cluster.calls
    assert "gw-0" in fake_cluster.running
    assert MAKER_NAME not in fake_cluster.running


def test_maker_stop_then_status_not_running(client, fake_cluster):
    """After stop, /api/maker/status returns running=False."""
    fake_cluster.started(MAKER_NAME)
    client.post("/api/maker/stop")
    data = client.get("/api/maker/status").json()
    assert data["running"] is False
    assert data["pid"] is None
    assert "stopped" in client.get("/x/maker-status").text.lower()


def test_maker_stop_is_not_a_crash_for_the_watcher(client, fake_cluster):
    """A stopped maker leaves no exited record for the watcher."""
    fake_cluster.started(MAKER_NAME)
    client.post("/api/maker/stop")
    assert MAKER_NAME not in fake_cluster.exited


def test_maker_restart_starts_a_new_process(client, fake_cluster):
    """POST /api/maker/restart runs `cluster restart maker`."""
    fake_cluster.started(MAKER_NAME)
    before = fake_cluster.running[MAKER_NAME]
    resp = client.post("/api/maker/restart")
    assert resp.status_code == 200
    assert ("restart", MAKER_NAME, "minimal") in fake_cluster.calls
    assert fake_cluster.running[MAKER_NAME] != before
    assert "maker restarted" in resp.text


def test_maker_config_save_restarts_a_running_maker(
        client, fake_cluster, tmp_path, monkeypatch):
    """Saving the maker config restarts the maker so it reads the file."""
    config = tmp_path / "maker-config.json"
    monkeypatch.setattr(server, "MAKER_CONFIG", config)
    fake_cluster.started(MAKER_NAME)
    resp = client.post("/api/maker/config", data={"spread_bps": 30})
    assert resp.status_code == 200
    assert json.loads(config.read_text())["spread_bps"] == 30
    assert ("restart", MAKER_NAME, "minimal") in fake_cluster.calls
    assert "maker restarted" in resp.text


def test_maker_config_save_without_a_maker_only_saves(
        client, fake_cluster, tmp_path, monkeypatch):
    config = tmp_path / "maker-config.json"
    monkeypatch.setattr(server, "MAKER_CONFIG", config)
    resp = client.post("/api/maker/config", data={"spread_bps": 30})
    assert "maker not running" in resp.text
    assert not [call for call in fake_cluster.calls if call[0] == "restart"]


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
    assert "<" in resp.text
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


def test_scan_processes_includes_maker_when_running(client, fake_cluster):
    """/api/processes lists the maker with its pid when it runs."""
    fake_cluster.started(MAKER_NAME)
    resp = client.get("/api/processes")
    assert resp.status_code == 200
    maker = next(p for p in resp.json() if p["name"] == MAKER_NAME)
    assert maker["state"] == "running"
    assert maker["pid"] == fake_cluster.running[MAKER_NAME]


def test_scan_processes_maker_not_present_when_stopped(client):
    """/api/processes does not list the maker without a cluster."""
    resp = client.get("/api/processes")
    assert resp.status_code == 200
    names = [p["name"] for p in resp.json()]
    assert MAKER_NAME not in names


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
