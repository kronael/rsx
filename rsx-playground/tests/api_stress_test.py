# pyright: basic
"""Stress run and probes through `rsx-cli stress`.

Run with: cd rsx-playground && uv run pytest tests/api_stress_test.py -v

The child is a shell script that answers as `rsx-cli stress` does: it
writes a report to `-o`, exits with an error line, or waits for SIGTERM.
The real CLI against a real cluster is exercised by the Playwright suite.
"""

import json
import socket
import stat
import textwrap

import pytest
from fastapi.testclient import TestClient

import rsx_cli
import server
import stress_cli

LEGS = (
    "internal",
    "gateway_to_risk",
    "risk_to_me",
    "matching",
    "match_done_to_gateway",
)
TARGET = stress_cli.Target("ws://127.0.0.1:8088", "ws://127.0.0.1:8180", "postgres://x")


def summary(**values):
    base = {
        "n": 100,
        "min": 80,
        "p50": 120,
        "p95": 300,
        "p99": 450,
        "p99_9": 900,
        "max": 1200,
    }
    return {**base, **values}


def cli_report(**overrides):
    report = {
        "mode": "load",
        "status": "passed",
        "failures": [],
        "interrupted": False,
        "config": {
            "gateway": "ws://127.0.0.1:8088",
            "symbols": [10],
            "users": 10,
            "connections": 10,
            "target_rate": 100,
            "duration_s": 10,
            "count": 0,
            "warmup": 0,
            "timeout_ms": 2000,
            "measures": "accepted",
        },
        "offered": 1000,
        "submitted": 1000,
        "send_errors": 0,
        "accepted": 985,
        "rejected": 15,
        "timeouts": 0,
        "errors": 0,
        "completed": 1000,
        "fills": 985,
        "filled_orders": 985,
        "rejected_by_reason": {"E1005": 15},
        "errors_by_reason": {},
        "timestamp_errors": {},
        "elapsed_ms": 10100,
        "latency_us": summary(),
        "send_lag_us": summary(),
        "server_latency_ns": {leg: summary() for leg in LEGS},
        "per_symbol": {},
    }
    report.update(overrides)
    return report


@pytest.fixture
def fake_cli(tmp_path, monkeypatch):
    """A `rsx-cli` stand-in; `behave(kind)` selects what it does."""
    report_src = tmp_path / "report.json"
    report_src.write_text(json.dumps(cli_report()))
    args_file = tmp_path / "args"
    script = tmp_path / "rsx-cli"

    def behave(kind, stdout=""):
        body = {
            "report": 'cp "$REPORT_SRC" "$out"',
            "wait": textwrap.dedent("""\
                trap 'cp "$REPORT_SRC" "$out"; exit 0' TERM
                while :; do sleep 0.1; done"""),
            "fail": 'echo \'{"error": "token refused"}\' >&2; exit 2',
            "stdout": f"printf '%s' '{stdout}'",
            "stdout_exit1": f"printf '%s' '{stdout}'; exit 1",
        }[kind]
        script.write_text(
            textwrap.dedent(f"""\
            #!/bin/sh
            export REPORT_SRC='{report_src}'
            echo "$@" > '{args_file}'
            while [ $# -gt 0 ]; do
              if [ "$1" = "-o" ]; then out=$2; fi
              shift
            done
            """)
            + body
            + "\n"
        )
        script.chmod(script.stat().st_mode | stat.S_IEXEC)

    monkeypatch.setattr(rsx_cli, "BINARY", script)
    monkeypatch.setattr(stress_cli, "PID_FILE", tmp_path / "stress.pid")
    behave.args = lambda: args_file.read_text().split()
    return behave


@pytest.fixture
def stress_run(tmp_path, monkeypatch):
    run = stress_cli.Stress(tmp_path / "reports", tmp_path / "log")
    monkeypatch.setattr(server, "stress_run", run)
    monkeypatch.setattr(server, "STRESS_REPORTS_DIR", run.report_dir)
    run.report_dir.mkdir()
    return run


@pytest.fixture
def client():
    return TestClient(server.app)


@pytest.fixture
def open_gateway():
    """A TCP listener standing in for the gateway the route probes."""
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen()
    yield f"ws://127.0.0.1:{listener.getsockname()[1]}"
    listener.close()


# ── the child ──────────────────────────────────────────────


@pytest.mark.asyncio
async def test_start_passes_the_load_and_target_to_the_cli(fake_cli, stress_run):
    fake_cli("report")
    await stress_run.start(stress_cli.Load(rate=50, duration_s=3, users=4), TARGET)
    await stress_run.task
    args = fake_cli.args()
    assert args[:1] == ["stress"]
    for flag, value in (
        ("-m", "load"),
        ("-r", "50"),
        ("-d", "3"),
        ("-u", "4"),
        ("-g", TARGET.gateway),
        ("--md", TARGET.marketdata),
        ("--db", TARGET.database_url),
        ("-o", str(stress_run.report)),
    ):
        assert args[args.index(flag) + 1] == value
    assert stress_run.report.exists()
    assert stress_run.error is None
    assert not stress_run.is_running()


@pytest.mark.asyncio
async def test_stop_terminates_the_child_and_its_report_is_kept(fake_cli, stress_run):
    fake_cli("wait")
    await stress_run.start(stress_cli.Load(10, 60), TARGET)
    assert stress_run.is_running()
    assert stress_run.pid is not None
    await stress_run.stop()
    assert not stress_run.is_running()
    assert stress_run.report.exists()


@pytest.mark.asyncio
async def test_a_child_that_fails_before_a_report_raises_its_error_line(
    fake_cli, stress_run
):
    fake_cli("fail")
    with pytest.raises(rsx_cli.CliError, match="token refused"):
        await stress_run.start(stress_cli.Load(10, 1), TARGET)
    assert stress_run.error == "token refused"
    assert not stress_run.report.exists()


@pytest.mark.asyncio
async def test_a_second_start_while_running_is_refused(fake_cli, stress_run):
    fake_cli("wait")
    await stress_run.start(stress_cli.Load(10, 60), TARGET)
    try:
        with pytest.raises(rsx_cli.CliError, match="already running"):
            await stress_run.start(stress_cli.Load(10, 60), TARGET)
    finally:
        await stress_run.stop()


@pytest.mark.asyncio
async def test_a_missing_binary_is_a_failed_start(stress_run, tmp_path, monkeypatch):
    monkeypatch.setattr(rsx_cli, "BINARY", tmp_path / "absent")
    with pytest.raises(rsx_cli.CliError, match="cannot start"):
        await stress_run.start(stress_cli.Load(10, 1), TARGET)


# ── probes ─────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_probe_reports_the_median_of_the_one_sample(fake_cli):
    fake_cli(
        "stdout",
        json.dumps(cli_report(latency_us=summary(n=1, p50=77), rejected_by_reason={})),
    )
    probe = await stress_cli.probe(TARGET, "probe", 10)
    assert probe.error is None
    assert probe.elapsed_us == 77
    args = fake_cli.args()
    assert args[args.index("-m") + 1] == "probe"
    assert args[args.index("-s") + 1] == "10"
    assert args[args.index("-n") + 1] == "1"


@pytest.mark.asyncio
async def test_reject_probe_carries_the_rejection_reasons(fake_cli):
    fake_cli(
        "stdout",
        json.dumps(
            cli_report(
                mode="reject",
                latency_us=summary(n=1, p50=31),
                rejected_by_reason={"E1007": 1},
            )
        ),
    )
    probe = await stress_cli.probe(TARGET, "reject")
    assert probe.elapsed_us == 31
    assert probe.reasons == {"E1007": 1}
    assert "-s" not in fake_cli.args()


@pytest.mark.asyncio
async def test_probe_without_a_sample_states_the_failures(fake_cli):
    fake_cli(
        "stdout_exit1",
        json.dumps(
            cli_report(
                status="failed",
                failures=["zero accepted latency samples"],
                latency_us=summary(n=0, p50=None),
            )
        ),
    )
    probe = await stress_cli.probe(TARGET, "probe", 10)
    assert probe.elapsed_us is None
    assert probe.error == "zero accepted latency samples"


@pytest.mark.asyncio
async def test_probe_failure_carries_the_cli_error_line(fake_cli):
    fake_cli("fail")
    probe = await stress_cli.probe(TARGET, "probe", 10)
    assert probe.elapsed_us is None
    assert probe.error == "token refused"


def test_latency_probe_route_records_the_sample(client, fake_cli, monkeypatch):
    fake_cli("stdout", json.dumps(cli_report(latency_us=summary(n=1, p50=61))))
    monkeypatch.setattr(server, "e2e_latencies", [])
    body = client.post("/api/latency-probe?symbol_id=10").json()
    assert body == {"ok": True, "symbol_id": 10, "elapsed_us": 61}
    assert server.e2e_latencies == [61]


def test_latency_probe_route_surfaces_the_cli_error(client, fake_cli):
    fake_cli("fail")
    body = client.post("/api/latency-probe?symbol_id=10").json()
    assert body["ok"] is False
    assert body["error"] == "token refused"


def test_gateway_only_probe_route_records_the_sample(client, fake_cli, monkeypatch):
    fake_cli(
        "stdout",
        json.dumps(
            cli_report(
                mode="reject",
                latency_us=summary(n=1, p50=29),
                rejected_by_reason={"E1007": 1},
            )
        ),
    )
    monkeypatch.setattr(server, "gw_only_latencies", [])
    body = client.post("/api/latency-probe-gw").json()
    assert body["ok"] is True
    assert body["elapsed_us"] == 29
    assert body["reasons"] == {"E1007": 1}
    assert server.gw_only_latencies == [29]


# ── routes ─────────────────────────────────────────────────


@pytest.mark.allow_5xx
def test_run_with_the_gateway_down_is_502(client, stress_run):
    resp = client.post(
        "/api/stress/run",
        data={"rate": "10", "duration": "1", "gateway_url": "ws://localhost:1"},
    )
    assert resp.status_code == 502
    assert resp.json()["code"] == "GATEWAY_UNREACHABLE"
    assert not stress_run.is_running()


def test_run_needs_a_loopback_caller_or_the_admin_token(stress_run, monkeypatch):
    monkeypatch.setattr(server, "PLAYGROUND_ADMIN_TOKEN", "")
    outsider = TestClient(server.app, client=("203.0.113.9", 5000))
    resp = outsider.post("/api/stress/run", data={"rate": "10", "duration": "1"})
    assert resp.status_code == 403
    assert not stress_run.is_running()


@pytest.mark.parametrize(
    "form",
    [
        {"rate": "0", "duration": "1"},
        {"rate": "10", "duration": "0"},
    ],
)
def test_run_with_a_non_positive_rate_or_duration_is_400(client, stress_run, form):
    resp = client.post("/api/stress/run", data=form)
    assert resp.status_code == 400
    assert resp.json()["code"] == "BAD_REQUEST"


def test_run_starts_the_cli_and_its_report_is_listed(
    client, fake_cli, stress_run, open_gateway
):
    fake_cli("report")
    resp = client.post(
        "/api/stress/run",
        data={"rate": "100", "duration": "10", "gateway_url": open_gateway},
    )
    assert resp.status_code == 200
    assert resp.json()["context"]["started"] is True
    assert fake_cli.args()[fake_cli.args().index("-g") + 1] == open_gateway
    rows = client.get("/api/stress/reports").json()
    assert len(rows) == 1
    assert rows[0]["rate"] == 100
    assert rows[0]["submitted"] == 1000
    assert rows[0]["accept_rate"] == 98.5
    assert rows[0]["p99_latency"] == 450
    assert rows[0]["status"] == "passed"


@pytest.mark.xfail(
    strict=True,
    reason="PG-CLI-STRESS-IGNORES-RATE-AND-DURATION: the script sends them "
    "as query parameters, the route reads form fields",
)
def test_script_stress_runs_at_the_asked_rate(
    client, fake_cli, stress_run, open_gateway, playground_cli, monkeypatch
):
    import httpx

    def post(url, **kwargs):
        kwargs.pop("timeout", None)
        return client.post(url.removeprefix(playground_cli.API_BASE), **kwargs)

    fake_cli("report")
    monkeypatch.setattr(httpx, "post", post)
    monkeypatch.setattr(server, "GATEWAY_URL", open_gateway)
    monkeypatch.setattr(playground_cli, "ensure_server", lambda: True)

    assert playground_cli.stress_test(5, 2) == 0
    args = fake_cli.args()
    assert args[args.index("-r") + 1] == "5"
    assert args[args.index("-d") + 1] == "2"


@pytest.mark.allow_5xx
def test_run_surfaces_the_cli_error_when_the_child_fails(
    client, fake_cli, stress_run, open_gateway
):
    fake_cli("fail")
    resp = client.post(
        "/api/stress/run",
        data={"rate": "10", "duration": "1", "gateway_url": open_gateway},
    )
    assert resp.status_code == 500
    assert resp.json()["code"] == "STRESS_FAILED"
    assert resp.json()["message"] == "token refused"
    htmx = client.post(
        "/api/stress/run",
        data={"rate": "10", "duration": "1", "gateway_url": open_gateway},
        headers={"hx-request": "true"},
    )
    assert htmx.status_code == 200
    assert "Failed: token refused" in htmx.text


def test_start_reads_form_values_from_the_overview_buttons(
    client, fake_cli, stress_run
):
    fake_cli("report")
    resp = client.post("/api/stress/start", data={"rate": "10", "duration": "60"})
    assert resp.status_code == 200
    args = fake_cli.args()
    assert args[args.index("-r") + 1] == "10"
    assert args[args.index("-d") + 1] == "60"


def test_start_rejects_a_non_numeric_rate(client, stress_run):
    resp = client.post("/api/stress/start", json={"rate": "fast"})
    assert resp.status_code == 400


def test_status_of_an_idle_server(client, stress_run):
    body = client.get("/api/stress/status").json()
    assert body == {
        "running": False,
        "pid": None,
        "name": "stress",
        "report": None,
        "error": None,
    }


def test_stop_when_idle_says_so(client, stress_run):
    resp = client.post("/api/stress/stop")
    assert "stress not running" in resp.text


# ── reports ────────────────────────────────────────────────


def write_report(stress_run, report_id, report):
    path = stress_run.report_dir / f"stress-{report_id}.json"
    path.write_text(json.dumps(report))
    return path


def test_reports_list_shows_a_cli_report_as_a_row(client, stress_run):
    write_report(stress_run, "20261027-120000", cli_report())
    resp = client.get("/x/stress-reports-list")
    assert resp.status_code == 200
    assert "2026-10-27 12:00:00" in resp.text
    assert "98.5%" in resp.text
    assert "450us" in resp.text


def test_reports_list_surfaces_corrupt_and_foreign_files(client, stress_run):
    (stress_run.report_dir / "stress-broken.json").write_text('{"config": ')
    write_report(stress_run, "old", {"config": {"target_rate": 1}})
    rows = {row["id"]: row for row in client.get("/api/stress/reports").json()}
    assert rows["broken"]["corrupt"] is True
    assert rows["old"]["corrupt"] is True
    assert "KeyError" in rows["old"]["error"]


def test_report_json_is_the_cli_object(client, stress_run):
    report = cli_report()
    write_report(stress_run, "r1", report)
    assert client.get("/api/stress/reports/r1").json() == report
    assert client.get("/api/stress/reports/nope").status_code == 404


def test_report_page_shows_counts_latency_legs_and_status(client, stress_run):
    write_report(stress_run, "20261027-120000", cli_report())
    resp = client.get("/stress/20261027-120000")
    assert resp.status_code == 200
    assert "985" in resp.text
    assert "450µs" in resp.text
    assert "match_done_to_gateway" in resp.text
    assert "PASS" in resp.text


def test_report_page_of_a_failed_run_lists_its_failures(client, stress_run):
    write_report(
        stress_run,
        "bad",
        cli_report(
            status="failed", failures=["accepted crossing orders did not all fill"]
        ),
    )
    resp = client.get("/stress/bad")
    assert "FAIL" in resp.text
    assert "accepted crossing orders did not all fill" in resp.text


def test_report_page_of_an_unreadable_file_is_422(client, stress_run):
    write_report(stress_run, "old", {"config": {}})
    assert client.get("/stress/old").status_code == 422
    assert client.get("/stress/absent").status_code == 404
