# pyright: basic
"""WAL views over rsx-cli: verify, the followed record cache, the
follow children's lifecycle and the errors surfaced to the UI.

Run with: cd rsx-playground && uv run pytest tests/api_wal_feed_test.py -v
"""

import os
import signal
import time

import pytest

import rsx_cli
import server
from test_utils import seq_frame
from test_utils import write_wal_stream


def wait_for(condition, what, timeout=15.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if condition():
            return
        time.sleep(0.05)
    raise AssertionError(f"timed out waiting for {what}")


def cached_seqs():
    return [rec["seq"] for rec in server.wal_records.records("unknown")]


def only_follower():
    followers = list(server.wal_records.followers.values())
    assert len(followers) == 1
    return followers[0]


def is_gone(pid):
    return not os.path.exists(f"/proc/{pid}")


@pytest.fixture
def stream_dir(tmp_path, monkeypatch):
    write_wal_stream(
        tmp_path, "pengu", [seq_frame(n) for n in (1, 2, 3)])
    monkeypatch.setattr(server, "WAL_DIR", tmp_path)
    return tmp_path


def append_frame(wal_dir, seq):
    active = wal_dir / "pengu" / "1" / "1_active.wal"
    with active.open("ab") as wal:
        wal.write(seq_frame(seq))


# ── wal verify ────────────────────────────────────────────


def test_verify_reports_the_records_it_checked(client, stream_dir):
    text = client.post("/api/wal/verify").text
    assert "verified 1 streams, 1 files, 3 records" in text


def test_verify_fails_on_a_flipped_byte(client, stream_dir):
    active = stream_dir / "pengu" / "1" / "1_active.wal"
    data = bytearray(active.read_bytes())
    data[32 + 16 + 3] ^= 0xFF
    active.write_bytes(bytes(data))
    text = client.post("/api/wal/verify").text
    assert "Failed: WAL corrupt" in text
    assert "pengu/1: 1 crc errors" in text
    assert "verified" not in text


def test_verify_reports_a_sequence_gap(client, tmp_path, monkeypatch):
    monkeypatch.setattr(server, "WAL_DIR", tmp_path)
    write_wal_stream(
        tmp_path, "pengu", [seq_frame(n) for n in (1, 2, 5)])
    text = client.post("/api/wal/verify").text
    assert "seq gap 3-4" in text


def test_dump_renders_the_cli_records_of_the_newest_file(
        client, stream_dir):
    text = client.post("/api/wal/dump").text
    assert "1_active.wal" in text
    assert text.count("UNKNOWN") == 3


# ── followed record cache ─────────────────────────────────


def test_followed_records_are_decoded_by_the_cli(
        client, stream_dir, follow_wal):
    follow_wal(stream_dir, 3)
    assert cached_seqs() == [1, 2, 3]
    record = server.wal_records.records("unknown")[0]
    assert record["stream"] == "pengu"
    assert record["ts_ns"] == 1000
    assert "crc32" not in record


def test_an_appended_record_arrives_once(
        client, stream_dir, follow_wal):
    follow_wal(stream_dir, 3)
    append_frame(stream_dir, 4)
    wait_for(lambda: cached_seqs() == [1, 2, 3, 4], "the appended record")
    time.sleep(0.5)
    assert cached_seqs() == [1, 2, 3, 4]


def test_rotation_is_followed_into_the_next_segment(
        client, stream_dir, follow_wal):
    follow_wal(stream_dir, 3)
    stream = stream_dir / "pengu" / "1"
    (stream / "1_active.wal").rename(stream / "1_1_3.wal")
    (stream / "1_active.wal").write_bytes(seq_frame(4) + seq_frame(5))
    wait_for(lambda: cached_seqs() == [1, 2, 3, 4, 5], "both segments")


def test_the_follow_window_starts_near_the_end_of_a_long_wal(
        client, tmp_path, follow_wal, monkeypatch):
    monkeypatch.setattr(server.wal_feed, "START_WINDOW_SEQS", 4)
    write_wal_stream(
        tmp_path, "pengu", [seq_frame(n) for n in range(1, 11)])
    follow_wal(tmp_path, 4)
    assert cached_seqs() == [7, 8, 9, 10]


def test_a_killed_follower_restarts_without_repeating_records(
        client, stream_dir, follow_wal):
    follow_wal(stream_dir, 3)
    follower = only_follower()
    first_pid = follower.pid
    os.kill(first_pid, signal.SIGKILL)
    wait_for(
        lambda: follower.pid not in (None, first_pid),
        "a restarted child")
    assert follower.restarts == 1
    append_frame(stream_dir, 4)
    wait_for(lambda: cached_seqs() == [1, 2, 3, 4], "record after restart")
    time.sleep(0.5)
    assert cached_seqs() == [1, 2, 3, 4]


def test_stopping_the_feed_kills_the_children(
        client, stream_dir, follow_wal):
    follow_wal(stream_dir, 3)
    pid = only_follower().pid
    assert not is_gone(pid)
    follow_wal.stop()
    wait_for(lambda: is_gone(pid), "the child to exit")
    assert server.wal_records.followers == {}
    assert cached_seqs() == []


def test_a_wiped_wal_is_followed_from_scratch(
        client, stream_dir, follow_wal):
    follow_wal(stream_dir, 3)
    (stream_dir / "pengu" / "1" / "1_active.wal").write_bytes(
        seq_frame(1) + seq_frame(2))
    follow_wal.refresh(stream_dir)
    wait_for(lambda: cached_seqs() == [1, 2], "the replaced WAL")


def test_a_restarted_seq_is_not_taken_for_a_wipe(
        client, stream_dir, follow_wal):
    follow_wal(stream_dir, 3)
    follower = only_follower()
    append_frame(stream_dir, 1)
    append_frame(stream_dir, 2)
    follow_wal.refresh(stream_dir)
    assert only_follower() is follower
    assert follower.restarts == 0
    assert cached_seqs() == [1, 2, 3]


def test_a_removed_stream_resets_the_cache(
        client, stream_dir, follow_wal):
    follow_wal(stream_dir, 3)
    (stream_dir / "pengu" / "1" / "1_active.wal").unlink()
    (stream_dir / "pengu" / "1").rmdir()
    follow_wal.refresh(stream_dir)
    assert cached_seqs() == []
    assert server.wal_records.followers == {}


def test_a_new_stream_is_followed_on_the_next_refresh(
        client, stream_dir, follow_wal):
    follow_wal(stream_dir, 3)
    write_wal_stream(stream_dir, "sol", [seq_frame(1)], stream_id=3)
    follow_wal.refresh(stream_dir)
    wait_for(lambda: len(server.wal_records.followers) == 2, "sol")
    wait_for(lambda: len(cached_seqs()) == 4, "the new stream's record")


# ── errors surface ────────────────────────────────────────


def corrupt_second_record(wal_dir):
    active = wal_dir / "pengu" / "1" / "1_active.wal"
    data = bytearray(active.read_bytes())
    data[32 + 16 + 3] ^= 0xFF
    active.write_bytes(bytes(data))


def test_a_follower_that_hits_a_corrupt_record_shows_its_error(
        client, stream_dir, follow_wal):
    corrupt_second_record(stream_dir)
    follow_wal(stream_dir, 1)
    follower = only_follower()
    wait_for(lambda: follower.restarts >= 1, "the first failure")
    assert "wal read error" in follower.error
    text = client.get("/x/wal-followers").text
    assert "pengu/1" in text
    assert "wal read error" in text


def test_a_failing_follower_backs_off(
        client, stream_dir, follow_wal, monkeypatch):
    monkeypatch.setattr(server.wal_feed, "BACKOFF_S", (0.2, 0.4, 0.8))
    corrupt_second_record(stream_dir)
    follow_wal(stream_dir, 1)
    follower = only_follower()
    wait_for(lambda: follower.restarts >= 3, "three failures", 20)
    assert follower.restarts <= 6


def test_followers_partial_lists_a_running_child(
        client, stream_dir, follow_wal):
    follow_wal(stream_dir, 3)
    text = client.get("/x/wal-followers").text
    assert "pengu/1" in text
    assert "running" in text
    assert str(only_follower().pid) in text


def test_followers_partial_without_streams_says_so(client):
    assert "no WAL streams followed" in client.get(
        "/x/wal-followers").text


@pytest.mark.allow_5xx
def test_a_missing_binary_is_an_error_not_a_fallback(
        client, stream_dir, monkeypatch):
    monkeypatch.setattr(rsx_cli, "BINARY", stream_dir / "no-such-cli")
    page = client.get("/x/wal-status")
    assert page.status_code == 200
    assert "Failed: rsx-cli: cannot start" in page.text
    api = client.get("/api/wal/pengu/status")
    assert api.status_code == 502
    assert "cannot start" in api.json()["error"]


def test_a_discovery_failure_is_kept_for_the_ui(
        client, stream_dir, follow_wal, monkeypatch):
    monkeypatch.setattr(rsx_cli, "BINARY", stream_dir / "no-such-cli")
    follow_wal.feed_loop.run(server.wal_records.refresh(stream_dir))
    assert "cannot start" in server.wal_records.refresh_error
    text = client.get("/x/wal-followers").text
    assert "Failed: stream discovery" in text


def test_unreadable_cli_output_is_an_error():
    with pytest.raises(rsx_cli.CliError, match="unparseable"):
        rsx_cli.parse_line("not json")
