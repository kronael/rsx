# pyright: basic
"""Faults through `rsx-cli fault`: the argument lists, the routes' text and
the live-stream choice of the WAL corruption.

Run with: cd rsx-playground && uv run pytest tests/api_fault_test.py -v

The argument tests run a shell script that answers as the CLI does. The WAL
test runs the real `target/debug/rsx-cli` over a fixture directory.
"""

import json
import os
import stat
import subprocess
import textwrap
import time

import pytest
from fastapi.testclient import TestClient

import cluster_cli
import fault_cli
import rsx_cli
import server
from test_utils import seq_frame
from test_utils import write_wal_stream


@pytest.fixture
def client():
    return TestClient(server.app)


@pytest.fixture
def fake_cli(tmp_path, monkeypatch):
    """A `rsx-cli` stand-in printing `stdout`, or the error line when set."""
    args_file = tmp_path / "args"
    script = tmp_path / "rsx-cli"

    def behave(stdout="", error=None):
        body = f"printf '%s' '{stdout}'"
        if error is not None:
            body = f"echo '{json.dumps({'error': error})}' >&2; exit 1"
        script.write_text(
            textwrap.dedent(f"""\
            #!/bin/sh
            echo "$@" > '{args_file}'
            """)
            + body
            + "\n"
        )
        script.chmod(script.stat().st_mode | stat.S_IEXEC)

    monkeypatch.setattr(rsx_cli, "BINARY", script)
    behave.args = lambda: args_file.read_text().split()
    return behave


# ── the argument lists ─────────────────────────────────────


@pytest.mark.asyncio
async def test_kill_names_the_process_and_the_root(fake_cli):
    fake_cli('{"name":"me-pengu","pid":4242,"signal":"SIGKILL"}')
    killed = await fault_cli.kill("me-pengu")
    assert killed == fault_cli.Killed("me-pengu", 4242)
    assert fake_cli.args() == [
        "fault",
        "kill",
        "me-pengu",
        "-C",
        str(fault_cli.ROOT),
        "--json",
    ]


@pytest.mark.asyncio
async def test_net_apply_passes_the_delay_and_loss_the_button_names(fake_cli):
    fake_cli('{"iface":"lo","delay_ms":50,"loss_pct":10.0,"active":true}')
    fault = await fault_cli.net(is_clear=False)
    assert fault == fault_cli.Net("lo", 50, 10.0, True)
    assert fake_cli.args() == [
        "fault",
        "net",
        "--delay-ms",
        "50",
        "--loss-pct",
        "10",
        "--json",
    ]


@pytest.mark.asyncio
async def test_net_clear_passes_clear_alone(fake_cli):
    fake_cli('{"iface":"lo","delay_ms":null,"loss_pct":null,"active":false}')
    fault = await fault_cli.net(is_clear=True)
    assert fault == fault_cli.Net("lo", None, None, False)
    assert fake_cli.args() == ["fault", "net", "--clear", "--json"]


@pytest.mark.asyncio
async def test_wal_corrupt_names_the_stream_and_its_directory(fake_cli, tmp_path):
    fake_cli(
        '{"stream_id":10,"file":"/w/pengu/10/10_active.wal",'
        '"offset":24,"old_byte":1,"new_byte":254}'
    )
    flip = await fault_cli.wal_corrupt(10, tmp_path / "pengu")
    assert flip.offset == 24 and flip.file.name == "10_active.wal"
    assert fake_cli.args() == [
        "fault",
        "wal-corrupt",
        "10",
        "--wal-dir",
        str(tmp_path / "pengu"),
        "-C",
        str(fault_cli.ROOT),
        "--json",
    ]


@pytest.mark.asyncio
@pytest.mark.parametrize("release, flag", [("1", ["-r"]), ("", [])])
async def test_dup_order_follows_the_release_switch(
    fake_cli, monkeypatch, release, flag
):
    monkeypatch.setenv("RSX_PLAYGROUND_RELEASE", release)
    fake_cli('{"sent":2,"order_id":"ab","records":[{"type":"ORDER_FAILED"}]}')
    demo = await fault_cli.dup_order()
    assert demo == fault_cli.DupOrder("ab", [{"type": "ORDER_FAILED"}])
    assert fake_cli.args() == [
        "fault",
        "dup-order",
        "-C",
        str(fault_cli.ROOT),
        *flag,
        "--json",
    ]
    assert cluster_cli.release_args() == flag


@pytest.mark.asyncio
async def test_the_cli_error_line_is_the_message(fake_cli):
    fake_cli(error="sudo -n tc qdisc add dev lo failed: RTNETLINK answers")
    with pytest.raises(rsx_cli.CliError, match="RTNETLINK answers"):
        await fault_cli.net(is_clear=False)


# ── /api/fault/net ─────────────────────────────────────────


def stub_net(monkeypatch, fault=None, error=None):
    seen = []

    async def net(is_clear):
        seen.append(is_clear)
        if error is not None:
            raise rsx_cli.CliError(error, 1)
        return fault

    monkeypatch.setattr(server.fault_cli, "net", net)
    return seen


def test_net_apply_says_what_was_added(client, monkeypatch):
    seen = stub_net(monkeypatch, fault_cli.Net("lo", 50, 10.0, True))
    text = client.post("/api/fault/net", data={"action": "apply"}).text
    assert seen == [False]
    assert "lo: +50ms delay, 10% loss" in text


def test_net_clear_says_it_healed(client, monkeypatch):
    seen = stub_net(monkeypatch, fault_cli.Net("lo", None, None, False))
    text = client.post("/api/fault/net", data={"action": "clear"}).text
    assert seen == [True]
    assert "lo netem cleared" in text


def test_net_failure_shows_the_tc_message(client, monkeypatch):
    stub_net(monkeypatch, error="sudo: a password is required")
    text = client.post("/api/fault/net", data={"action": "apply"}).text
    assert "Failed: sudo: a password is required" in text
    assert "text-red-400" in text


def test_net_is_admin_only(client):
    remote = TestClient(server.app, client=("203.0.113.9", 5000))
    assert remote.post("/api/fault/net").status_code == 403


# ── /api/fault/wal-corrupt: the real CLI over a fixture ────


def verify_status(wal_dir):
    out = subprocess.run(
        [str(rsx_cli.BINARY), "wal", "verify", str(wal_dir), "--json"],
        check=False,
        capture_output=True,
        text=True,
    )
    return json.loads(out.stdout)["status"]


@pytest.fixture
def live_streams(tmp_path, monkeypatch):
    """An ME stream, a mark stream written later and a recorder archive
    written last of all: the archive is the newest file by mtime."""
    frames = [seq_frame(n) for n in (1, 2, 3)]
    me = write_wal_stream(tmp_path, "pengu", frames, stream_id=10)
    mark = write_wal_stream(tmp_path, "mark", frames, stream_id=100)
    archive = write_wal_stream(tmp_path / "archive", "10", frames, stream_id=10)
    now = time.time()
    for age, path in ((30, me), (20, mark), (10, archive)):
        os.utime(path, (now - age, now - age))
    monkeypatch.setattr(server, "WAL_DIR", tmp_path)
    return me, mark, archive


def test_wal_corrupt_flips_the_newest_live_stream_not_the_archive(client, live_streams):
    me, mark, archive = live_streams
    before = {path: path.read_bytes() for path in live_streams}
    text = client.post("/api/fault/wal-corrupt").text
    assert "flipped payload byte @ offset 24 in mark/100/100_active.wal" in text
    assert mark.read_bytes() != before[mark]
    assert me.read_bytes() == before[me]
    assert archive.read_bytes() == before[archive]
    assert verify_status(mark.parent.parent) == "corrupt"
    assert verify_status(me.parent.parent) == "ok"


def test_wal_corrupt_without_any_stream_says_so(client, tmp_path, monkeypatch):
    monkeypatch.setattr(server, "WAL_DIR", tmp_path / "missing")
    text = client.post("/api/fault/wal-corrupt").text
    assert "no WAL files found" in text


def test_wal_corrupt_with_an_unreadable_record_shows_the_cli_error(
    client, tmp_path, monkeypatch
):
    stream = write_wal_stream(tmp_path, "pengu", [], stream_id=10)
    stream.write_bytes(b"\0" * 8)
    monkeypatch.setattr(server, "WAL_DIR", tmp_path)
    text = client.post("/api/fault/wal-corrupt").text
    assert "Failed:" in text
    assert stream.read_bytes() == b"\0" * 8
