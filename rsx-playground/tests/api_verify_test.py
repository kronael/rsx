# pyright: basic
"""Verify page glue over `rsx-cli verify --json`.

The checks themselves are tested in rsx-cli; these tests cover what the
playground adds: running the real binary over its WAL directories, passing
the Postgres url, rendering pass/fail/skip, and surfacing errors.

Run with: cd rsx-playground && uv run pytest tests/api_verify_test.py -v
"""

import time

import pytest

import invariants
import pages
import rsx_cli
import server
from invariants import Check
from invariants import Report
from invariants import Status
from test_utils import seq_frame
from test_utils import write_wal_stream

DOWN_DB = "postgres://rsx:rsx@127.0.0.1:1/rsx"


@pytest.fixture
def wal_dir(tmp_path, monkeypatch):
    write_wal_stream(tmp_path, "pengu", [seq_frame(n) for n in (1, 2, 3)])
    monkeypatch.setattr(server, "WAL_DIR", tmp_path)
    monkeypatch.setattr(server, "PG_URL", DOWN_DB)
    return tmp_path


def check_named(body, name):
    return next(check for check in body["checks"] if check["name"] == name)


def make_check(status, violations=(), violation_count=0, invariant=1):
    return Check(
        invariant=invariant,
        name=f"check {status.value}",
        status=status,
        detail="the detail",
        checked=5,
        violation_count=violation_count,
        violations=list(violations),
    )


def make_report(*checks, status=Status.PASS, error=""):
    return Report(status, list(checks), int(time.time() * 1000), error)


# ── run-json ──────────────────────────────────────────────


def test_run_json_carries_the_cli_checks(client, wal_dir):
    before_ms = time.time_ns() // 1_000_000
    resp = client.post("/api/verify/run-json")

    assert resp.status_code == 200
    body = resp.json()
    assert {check["invariant"] for check in body["checks"]} >= set(range(1, 11))
    assert set(body["checks"][0]) == {
        "invariant", "name", "status", "detail",
        "checked", "violation_count", "violations",
    }
    assert body["run_at_ms"] >= before_ms


def test_run_json_readiness_follows_the_failed_checks(client, wal_dir):
    body = client.post("/api/verify/run-json").json()

    assert body["failed"] == sum(
        check["status"] == "fail" for check in body["checks"])
    assert body["failed"] > 0
    assert body["ready"] is False
    assert body["status"] == "fail"


def test_a_check_with_nothing_to_examine_is_skip_not_pass(client, wal_dir):
    body = client.post("/api/verify/run-json").json()

    assert check_named(body, "WAL records are intact")["status"] == "pass"
    assert check_named(body, "Fills precede ORDER_DONE")["status"] == "skip"
    assert check_named(body, "SPSC rings preserve FIFO")["status"] == "skip"


def test_down_postgres_fails_its_check_with_the_reason(client, wal_dir):
    body = client.post("/api/verify/run-json").json()

    postgres = check_named(body, "Postgres is reachable")
    assert postgres["status"] == "fail"
    assert "failed to connect" in postgres["detail"]


def test_a_flipped_byte_fails_the_intact_check_at_its_stream(
        client, wal_dir):
    active = wal_dir / "pengu" / "1" / "1_active.wal"
    data = bytearray(active.read_bytes())
    data[32 + 16 + 3] ^= 0xFF
    active.write_bytes(bytes(data))

    body = client.post("/api/verify/run-json").json()

    intact = check_named(body, "WAL records are intact")
    assert intact["status"] == "fail"
    violation = intact["violations"][0]
    assert violation["stream_id"] == 1
    assert violation["wal_dir"] == str(wal_dir / "pengu")


# ── HTML rendering ────────────────────────────────────────


def test_run_renders_one_badge_per_check_in_its_status(client, wal_dir):
    body = client.post("/api/verify/run-json").json()
    resp = client.post("/api/verify/run")

    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]
    for status in Status:
        expected = sum(
            check["status"] == status.value for check in body["checks"])
        assert resp.text.count(f">{status.value.upper()}</span>") == expected


def test_run_shows_a_failure_with_its_location(client, wal_dir):
    active = wal_dir / "pengu" / "1" / "1_active.wal"
    data = bytearray(active.read_bytes())
    data[32 + 16 + 3] ^= 0xFF
    active.write_bytes(bytes(data))

    text = client.post("/api/verify/run").text

    assert "WAL records are intact" in text
    assert "crc error" in text
    assert "stream_id 1" in text


def test_the_mark_wal_dir_is_not_verified(client, wal_dir):
    write_wal_stream(wal_dir, "mark", [seq_frame(n) for n in (5, 1)], 100)

    body = client.post("/api/verify/run-json").json()

    intact = check_named(body, "WAL records are intact")
    assert intact["status"] == "pass"
    assert intact["checked"] == 3


def test_x_verify_before_a_run_prompts(client):
    text = client.get("/x/verify").text

    assert 'click "Run All Checks"' in text


def test_x_verify_shows_the_last_report(client, wal_dir):
    client.post("/api/verify/run")
    text = client.get("/x/verify").text

    assert "Run All Checks" not in text
    assert "WAL records are intact" in text
    assert "processes running" in text


def test_render_shows_only_the_first_violations(client):
    violations = [{"message": f"broken {n}"} for n in range(10)]
    failing = make_check(Status.FAIL, violations, violation_count=13)

    text = pages.render_verify(make_report(failing, status=Status.FAIL))

    assert "broken 0" in text and "broken 2" in text
    assert "broken 3" not in text
    assert "and 10 more" in text


# ── errors ────────────────────────────────────────────────


@pytest.mark.allow_5xx
def test_a_missing_wal_dir_surfaces_the_cli_reason(
        client, tmp_path, monkeypatch):
    monkeypatch.setattr(server, "WAL_DIR", tmp_path / "absent")

    resp = client.post("/api/verify/run-json")
    assert resp.status_code == 502
    assert "failed to read" in resp.json()["error"]

    assert "Failed: failed to read" in client.post("/api/verify/run").text
    assert "Failed: failed to read" in client.get("/x/verify").text
    assert "Failed: failed to read" in client.get("/x/invariant-status").text


@pytest.mark.allow_5xx
def test_a_missing_binary_surfaces_why(client, tmp_path, monkeypatch):
    monkeypatch.setattr(rsx_cli, "BINARY", tmp_path / "no-rsx-cli")

    resp = client.post("/api/verify/run-json")

    assert resp.status_code == 502
    assert "cannot start" in resp.json()["error"]


def test_an_unexpected_report_is_an_error():
    with pytest.raises(rsx_cli.CliError, match="unexpected rsx-cli verify"):
        invariants.parse_report('{"status": "pass", "checks": [{"name": "x"}]}')


# ── invariant status card ─────────────────────────────────


def test_status_card_is_pending_before_any_report():
    assert "check pending" in pages.render_invariant_status(None)


def test_status_card_counts_skips_apart_from_passes():
    text = pages.render_invariant_status(
        make_report(make_check(Status.PASS), make_check(Status.SKIP)))

    assert "1 passing" in text
    assert "1 skipped" in text
    assert "All passing" not in text


def test_status_card_with_only_skips_says_nothing_was_checked():
    text = pages.render_invariant_status(
        make_report(make_check(Status.SKIP), status=Status.SKIP))

    assert "nothing checked" in text
    assert "passing" not in text


def test_status_card_names_the_failing_checks_count():
    text = pages.render_invariant_status(
        make_report(
            make_check(Status.PASS), make_check(Status.FAIL),
            status=Status.FAIL))

    assert "1 check(s) failing" in text


def test_status_card_shows_the_run_error():
    text = pages.render_invariant_status(
        invariants.failed_report("cannot start rsx-cli"))

    assert "Failed: cannot start rsx-cli" in text


def test_status_card_runs_once_per_five_seconds(client, wal_dir):
    client.get("/x/invariant-status")
    first = server.verifier.report

    client.get("/x/invariant-status")

    assert server.verifier.report is first


def test_status_card_polls_during_a_run_do_not_start_another(
        client, wal_dir, monkeypatch):
    monkeypatch.setattr(server.verifier, "running", True)

    text = client.get("/x/invariant-status").text

    assert "check pending" in text
    assert server.verifier.report is None
