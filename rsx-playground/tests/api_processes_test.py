# pyright: basic
"""API tests for the process endpoints and the glue to `rsx-cli cluster`.

Run with: cd rsx-playground && uv run pytest tests/api_processes_test.py -v

The routes run over the in-memory cluster (`fake_cluster`, which answers as
`rsx-cli cluster … --json` does). The real CLI is exercised by the read-only
calls at the end and by the Playwright suite, which starts the real cluster.
"""

import asyncio
import os
import signal
from unittest.mock import AsyncMock

import pytest
from fastapi.testclient import TestClient

import cluster_cli
import fault_cli
import risk_cli
import rsx_cli
import server
from server import app

CONFIRM = {"x-confirm": "yes"}


@pytest.fixture
def client():
    return TestClient(app)


def post(client, path, **kwargs):
    return client.post(path, headers=CONFIRM, **kwargs)


# ── Start, stop, list ──────────────────────────────────────


def test_start_all_minimal_scenario(client, fake_cluster):
    """Start-all brings up the scenario's processes with a build."""
    resp = post(client, "/api/processes/all/start?scenario=minimal")
    assert resp.status_code == 200
    assert "started 7 processes" in resp.text
    assert ("up", "minimal", (), True) in fake_cluster.calls
    assert fake_cluster.scenario == "minimal"
    assert server.current_scenario == "minimal"


def test_start_all_twice_reports_the_running_ones(client, fake_cluster):
    post(client, "/api/processes/all/start?scenario=minimal")
    resp = post(client, "/api/processes/all/start?scenario=minimal")
    assert "started 0 processes (7 already running)" in resp.text


def test_start_all_of_another_scenario_stops_the_running_one(
        client, fake_cluster):
    post(client, "/api/processes/all/start?scenario=minimal")
    resp = post(client, "/api/processes/all/start?scenario=duo")
    assert "started 8 processes" in resp.text
    assert ("down", ()) in fake_cluster.calls
    assert "me-sol" in fake_cluster.running


def test_start_all_failure_is_shown_as_text(client, fake_cluster):
    fake_cluster.fail("ports already in use: 8088")
    resp = post(client, "/api/processes/all/start?scenario=minimal")
    assert resp.status_code == 200
    assert "ports already in use: 8088" in resp.text
    assert "text-red-400" in resp.text


def test_start_all_seeds_the_accounts_first(client, fake_cluster, monkeypatch):
    order = []

    async def seed(database_url):
        order.append("seed")
        return risk_cli.Seeded(1001, 10**17)

    original_up = fake_cluster.up
    original_down = fake_cluster.down

    async def up(*args, **kwargs):
        order.append("up")
        return await original_up(*args, **kwargs)

    async def down(*args, **kwargs):
        order.append("down")
        return await original_down(*args, **kwargs)

    monkeypatch.setattr(risk_cli, "account_seed", seed)
    monkeypatch.setattr(cluster_cli, "up", up)
    monkeypatch.setattr(cluster_cli, "down", down)
    post(client, "/api/processes/all/start?scenario=minimal")
    assert order == ["seed", "up"]
    order.clear()
    post(client, "/api/processes/all/start?scenario=duo")
    assert order == ["down", "seed", "up"]


def test_start_all_seeds_against_the_postgres_url(
        client, fake_cluster, monkeypatch):
    seed = AsyncMock(return_value=risk_cli.Seeded(1001, 10**17))
    monkeypatch.setattr(risk_cli, "account_seed", seed)
    post(client, "/api/processes/all/start?scenario=minimal")
    seed.assert_awaited_once_with(server.PG_URL)


def test_start_all_seed_failure_is_shown_and_nothing_starts(
        client, fake_cluster, monkeypatch):
    monkeypatch.setattr(
        risk_cli, "account_seed",
        AsyncMock(side_effect=rsx_cli.CliError("connection refused", 1)))
    resp = post(client, "/api/processes/all/start?scenario=minimal")
    assert resp.status_code == 200
    assert "connection refused" in resp.text
    assert "text-red-400" in resp.text
    assert not [call for call in fake_cluster.calls if call[0] == "up"]
    assert not fake_cluster.running


def test_start_all_needs_confirmation(client, fake_cluster):
    resp = client.post("/api/processes/all/start?scenario=minimal")
    assert resp.status_code == 400
    assert not fake_cluster.calls


def test_stop_all_stops_the_cluster(client, fake_cluster):
    post(client, "/api/processes/all/start?scenario=minimal")
    resp = post(client, "/api/processes/all/stop")
    assert resp.status_code == 200
    assert "stopped 7 processes" in resp.text
    assert not fake_cluster.running


def test_stop_all_with_nothing_running(client, fake_cluster):
    resp = post(client, "/api/processes/all/stop")
    assert resp.status_code == 200
    assert "stopped 0 processes" in resp.text


def test_get_processes_lists_the_cluster(client, fake_cluster):
    post(client, "/api/processes/all/start?scenario=minimal")
    resp = client.get("/api/processes")
    assert resp.status_code == 200
    procs = resp.json()
    assert [p["name"] for p in procs] == sorted(p["name"] for p in procs)
    assert {p["state"] for p in procs} == {"running"}
    gateway = next(p for p in procs if p["name"] == "gw-0")
    assert gateway["pid"] == fake_cluster.running["gw-0"]
    for key in ("state", "cpu", "mem", "uptime", "total_restarts"):
        assert key in gateway


def test_stopped_process_has_no_pid(client, fake_cluster):
    post(client, "/api/processes/all/start?scenario=minimal")
    post(client, "/api/processes/gw-0/stop")
    procs = client.get("/api/processes").json()
    gateway = next(p for p in procs if p["name"] == "gw-0")
    assert gateway["state"] == "stopped"
    assert gateway["pid"] == "-"


def test_plan_processes_show_as_stopped_before_the_first_start(
        client, fake_cluster):
    procs = client.get("/api/processes").json()
    assert {p["name"] for p in procs} == {
        "gw-0", "marketdata", "mark", "me-pengu", "recorder", "risk-0",
        "maker"}
    assert {p["state"] for p in procs} == {"stopped"}


def test_get_scenarios_includes_minimal(client, fake_cluster):
    resp = client.get("/api/scenarios")
    assert resp.status_code == 200
    assert resp.json() == ["minimal", "duo", "stress-low"]


def test_metrics_counts_running_processes(client, fake_cluster):
    post(client, "/api/processes/all/start?scenario=minimal")
    data = client.get("/api/metrics").json()
    assert data["processes"] == 7
    assert data["running"] == 7


def test_removed_build_routes(client):
    assert client.post("/api/build").status_code in (404, 405)
    assert client.get("/api/build-log").status_code == 404


# ── Single process actions ─────────────────────────────────


def test_start_individual_process(client, fake_cluster):
    resp = post(client, "/api/processes/gw-0/start")
    assert resp.status_code == 200
    assert ("up", "minimal", ("gw-0",), True) in fake_cluster.calls
    assert f"started gw-0 (pid {fake_cluster.running['gw-0']})" in resp.text


def test_start_a_running_process_is_refused(client, fake_cluster):
    fake_cluster.started("gw-0")
    resp = post(client, "/api/processes/gw-0/start")
    assert "gw-0 already running" in resp.text
    assert not [call for call in fake_cluster.calls if call[0] == "up"]


def test_start_unknown_process_returns_error(client, fake_cluster):
    resp = post(client, "/api/processes/invalid-proc-999/start")
    assert resp.status_code == 200
    assert "unknown process: invalid-proc-999" in resp.text


def test_start_failure_shows_the_cli_error(client, fake_cluster):
    fake_cluster.fail("binary not found: rsx-gateway")
    resp = post(client, "/api/processes/gw-0/start")
    assert "Failed: binary not found: rsx-gateway" in resp.text


def test_stop_individual_process(client, fake_cluster):
    fake_cluster.started("gw-0", "me-pengu")
    resp = post(client, "/api/processes/gw-0/stop")
    assert resp.status_code == 200
    assert "stopped gw-0" in resp.text
    assert ("down", ("gw-0",)) in fake_cluster.calls
    assert "me-pengu" in fake_cluster.running


def test_stop_unknown_or_stopped_process(client, fake_cluster):
    resp = post(client, "/api/processes/unknown-999/stop")
    assert resp.status_code == 200
    assert "unknown-999 not running" in resp.text


def test_stop_marks_the_exit_intentional(client, fake_cluster):
    fake_cluster.started("gw-0")
    post(client, "/api/processes/gw-0/stop")
    assert server._restart_state["gw-0"]["intentional"] is True


def test_restart_process(client, fake_cluster):
    fake_cluster.started("gw-0")
    before = fake_cluster.running["gw-0"]
    resp = post(client, "/api/processes/gw-0/restart")
    assert resp.status_code == 200
    assert ("restart", "gw-0", "minimal") in fake_cluster.calls
    assert fake_cluster.running["gw-0"] != before
    assert f"restarted gw-0 (pid {fake_cluster.running['gw-0']})" in resp.text


def test_restart_when_not_running_starts_it(client, fake_cluster):
    resp = post(client, "/api/processes/gw-0/restart")
    assert "restarted gw-0" in resp.text
    assert "gw-0" in fake_cluster.running


def test_restart_resets_the_circuit(client, fake_cluster):
    entry = server._restart_entry("gw-0")
    entry.update(restarts=6, blocked=True, next_restart_at=1e12)
    post(client, "/api/processes/gw-0/restart")
    assert entry["blocked"] is False
    assert entry["restarts"] == 0
    assert entry["next_restart_at"] == 0.0
    assert entry["restarting"] is False


def test_restart_unknown_process(client, fake_cluster):
    resp = post(client, "/api/processes/unknown-999/restart")
    assert resp.status_code == 200
    assert "unknown process: unknown-999" in resp.text


def test_kill_process(client, fake_cluster, monkeypatch):
    fake_cluster.started("gw-0")
    pid = fake_cluster.running["gw-0"]
    signals = []
    monkeypatch.setattr(os, "kill", lambda target, sig: signals.append(
        (target, sig)))
    resp = post(client, "/api/processes/gw-0/kill")
    assert resp.status_code == 200
    assert "killed gw-0" in resp.text
    assert (pid, signal.SIGKILL) in signals
    assert ("down", ("gw-0",)) in fake_cluster.calls
    assert server._restart_state["gw-0"]["intentional"] is True


def test_kill_not_running_process(client, fake_cluster):
    resp = post(client, "/api/processes/unknown-999/kill")
    assert resp.status_code == 200
    assert "unknown-999 not running" in resp.text


def test_invalid_action_returns_400(client):
    resp = client.post("/api/processes/gw-0/invalid-action")
    assert resp.status_code == 400
    assert "error" in resp.json()


# ── Fault: a crash the watcher restarts ────────────────────


@pytest.fixture
def fault_kill(fake_cluster, monkeypatch):
    """`fault_cli.kill` over the in-memory cluster: the process exits."""
    calls = []

    async def kill(name):
        calls.append(name)
        pid = fake_cluster.running[name]
        fake_cluster.crash(name)
        return fault_cli.Killed(name, pid)

    monkeypatch.setattr(server.fault_cli, "kill", kill)
    return calls


def test_fault_kill_leaves_the_exit_to_the_watcher(
        client, fake_cluster, fault_kill):
    fake_cluster.started("me-pengu")
    pid = fake_cluster.running["me-pengu"]
    resp = client.post("/api/fault/kill/me-pengu")
    assert resp.status_code == 200
    assert fault_kill == ["me-pengu"]
    assert f"crashed me-pengu (pid {pid})" in resp.text
    assert not server._restart_state.get("me-pengu", {}).get("intentional")
    assert ("down", ("me-pengu",)) not in fake_cluster.calls


def test_fault_kill_not_running(client, fake_cluster, fault_kill):
    resp = client.post("/api/fault/kill/me-pengu")
    assert "me-pengu not running" in resp.text
    assert fault_kill == []


def test_fault_kill_cli_refusal_is_shown(
        client, fake_cluster, monkeypatch):
    fake_cluster.started("me-pengu")

    async def refuse(name):
        raise rsx_cli.CliError("me-pengu: no pid record", 1)

    monkeypatch.setattr(server.fault_cli, "kill", refuse)
    resp = client.post("/api/fault/kill/me-pengu")
    assert "Kill failed: me-pengu: no pid record" in resp.text
    assert "text-red-400" in resp.text


# ── Reset ──────────────────────────────────────────────────


def test_reset_stops_and_wipes_through_the_cli(client, fake_cluster):
    post(client, "/api/processes/all/start?scenario=minimal")
    resp = post(client, "/api/reset")
    assert resp.status_code == 200
    assert ("reset",) in fake_cluster.calls
    assert not fake_cluster.running
    assert "stopped 7 processes" in resp.text
    assert "cleared risk-state, wal, tips" in resp.text


def test_reset_failure_is_shown_and_nothing_is_claimed(
        client, fake_cluster):
    fake_cluster.fail("postgres is not reachable at postgres://rsx@x")
    resp = post(client, "/api/reset")
    assert resp.status_code == 200
    assert "Failed: postgres is not reachable" in resp.text
    assert "reset done" not in resp.text


def test_reset_needs_confirmation(client, fake_cluster):
    resp = client.post("/api/reset")
    assert resp.status_code == 400
    assert not fake_cluster.calls


# ── Scenario switch ────────────────────────────────────────


def test_scenario_switch_restarts_the_cluster(client, fake_cluster):
    post(client, "/api/processes/all/start?scenario=minimal")
    resp = post(
        client, "/api/scenario/switch", data={"scenario-select": "duo"})
    assert resp.status_code == 200
    assert "switched to duo and restarted 8 processes" in resp.text
    assert fake_cluster.scenario == "duo"
    assert client.get("/x/current-scenario").text == "duo"


def test_scenario_switch_to_the_running_scenario_restarts_it(
        client, fake_cluster):
    post(client, "/api/processes/all/start?scenario=minimal")
    before = dict(fake_cluster.running)
    resp = post(
        client, "/api/scenario/switch", data={"scenario-select": "minimal"})
    assert "switched to minimal and restarted 7 processes" in resp.text
    assert set(fake_cluster.running) == set(before)
    assert all(fake_cluster.running[n] != before[n] for n in before)


def test_scenario_switch_unknown_scenario(client, fake_cluster):
    resp = post(
        client, "/api/scenario/switch",
        data={"scenario-select": "unknown-999"})
    assert resp.status_code == 200
    assert "unknown scenario: unknown-999" in resp.text
    assert not fake_cluster.running


def test_scenario_switch_reports_a_failed_start(client, fake_cluster):
    fake_cluster.fail("not ready after 120 s: risk-0 (warm_catchup)")
    resp = post(
        client, "/api/scenario/switch", data={"scenario-select": "duo"})
    assert "switched to duo (restart failed: not ready after 120 s" in (
        resp.text)


def test_scenario_survives_a_server_restart(client, fake_cluster):
    """The scenario is the CLI's state, not the server's memory."""
    post(client, "/api/processes/all/start?scenario=duo")
    server.current_scenario = "minimal"
    assert client.get("/x/current-scenario").text == "duo"


# ── The watcher ────────────────────────────────────────────


def exited_view(fake_cluster, *names):
    fake_cluster.started(*names)
    fake_cluster.names = list(names)
    for name in names:
        fake_cluster.crash(name)
    return asyncio.run(fake_cluster.ps())


def run_watch(view):
    async def watch():
        server.watch_crashes(view)
        await asyncio.gather(*list(server._restart_tasks))

    asyncio.run(watch())


def test_watcher_restarts_an_exited_process(fake_cluster):
    run_watch(exited_view(fake_cluster, "me-pengu"))
    assert ("restart", "me-pengu", "minimal") in fake_cluster.calls
    assert "me-pengu" in fake_cluster.running
    entry = server._restart_state["me-pengu"]
    assert entry["total_restarts"] == 1
    assert entry["restarting"] is False


def test_watcher_ignores_running_and_absent_processes(fake_cluster):
    fake_cluster.names = ["gw-0", "mark"]
    fake_cluster.started("gw-0")
    run_watch(asyncio.run(fake_cluster.ps()))
    assert not [c for c in fake_cluster.calls if c[0] == "restart"]


def test_watcher_ignores_an_intentional_exit(fake_cluster):
    view = exited_view(fake_cluster, "me-pengu")
    server._restart_entry("me-pengu")["intentional"] = True
    run_watch(view)
    assert not [c for c in fake_cluster.calls if c[0] == "restart"]


def test_watcher_clears_intentional_once_the_record_is_gone(fake_cluster):
    server._restart_entry("me-pengu")["intentional"] = True
    fake_cluster.names = ["me-pengu"]
    run_watch(asyncio.run(fake_cluster.ps()))
    assert server._restart_state["me-pengu"]["intentional"] is False


def test_watcher_backs_off_between_restarts(fake_cluster):
    view = exited_view(fake_cluster, "me-pengu")
    run_watch(view)
    fake_cluster.crash("me-pengu")
    run_watch(view)
    restarts = [c for c in fake_cluster.calls if c[0] == "restart"]
    assert len(restarts) == 1
    assert server._restart_state["me-pengu"]["next_restart_at"] > 0


def test_watcher_opens_the_circuit_after_repeated_crashes(fake_cluster):
    view = exited_view(fake_cluster, "me-pengu")
    for _ in range(server._RESTART_MAX + 2):
        server._restart_entry("me-pengu")["next_restart_at"] = 0.0
        run_watch(view)
        if "me-pengu" in fake_cluster.running:
            fake_cluster.crash("me-pengu")
    restarts = [c for c in fake_cluster.calls if c[0] == "restart"]
    assert len(restarts) == server._RESTART_MAX
    assert server._restart_state["me-pengu"]["blocked"] is True
    procs = asyncio.run(server.scan_processes())
    assert next(p for p in procs if p["name"] == "me-pengu")["state"] == (
        "blocked")


def test_watcher_failed_restart_is_logged_and_retried_later(
        fake_cluster, caplog):
    view = exited_view(fake_cluster, "me-pengu")
    fake_cluster.fail("binary not found")
    run_watch(view)
    assert "restart of me-pengu failed: binary not found" in caplog.text
    assert server._restart_state["me-pengu"]["restarting"] is False


# ── The real CLI, read only ────────────────────────────────


def test_real_cli_lists_scenarios_and_an_empty_cluster(tmp_path):
    async def ask():
        scenarios = await cluster_cli.scenarios()
        data = await rsx_cli.run(
            "cluster", "ps", "-C", str(tmp_path), "--json")
        return scenarios, data

    scenarios, data = asyncio.run(ask())
    assert "minimal" in [s.name for s in scenarios]
    assert data == {"scenario": None, "processes": []}


def test_real_cli_rejects_an_unknown_scenario_with_code_2():
    with pytest.raises(rsx_cli.CliError) as raised:
        asyncio.run(cluster_cli.plan("no-such-scenario"))
    assert raised.value.code == cluster_cli.EXIT_MISUSE
