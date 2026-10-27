# pyright: basic
"""The marketdata book and trades from one `rsx-cli md follow` child.

Run with: cd rsx-playground && uv run pytest tests/api_md_feed_test.py -v

The child is a shell script that prints the frames in `frames.jl`, as the
CLI prints them (`rsx-cli/README.md` § md follow), and then waits for
SIGTERM. The real CLI against a real marketdata runs in the live check.
"""

import json
import os
import stat
import time

import pytest
from conftest import FeedLoop

import cluster_cli
import md_feed
import rsx_cli
import server


def wait_for(condition, what, timeout=10.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if condition():
            return
        time.sleep(0.02)
    raise AssertionError(f"timed out waiting for {what}")


def is_gone(pid):
    return not os.path.exists(f"/proc/{pid}")


def snapshot(symbol_id, bids, asks):
    return {
        "kind": "l2_snapshot",
        "symbol_id": symbol_id,
        "bids": [[px, qty, 1] for px, qty in bids],
        "asks": [[px, qty, 1] for px, qty in asks],
        "ts_ns": 1,
        "seq": 1,
    }


def delta(symbol_id, side, px, qty):
    return {
        "kind": "l2_delta",
        "symbol_id": symbol_id,
        "side": side,
        "px": px,
        "qty": qty,
        "count": 1,
        "ts_ns": 2,
        "seq": 2,
    }


def bbo(symbol_id, bid, ask):
    return {
        "kind": "bbo",
        "symbol_id": symbol_id,
        "bid_px": bid[0],
        "bid_qty": bid[1],
        "bid_count": 1,
        "ask_px": ask[0],
        "ask_qty": ask[1],
        "ask_count": 1,
        "ts_ns": 3,
        "seq": 3,
    }


def trade(symbol_id, px, qty, seq=4):
    return {
        "kind": "trade",
        "symbol_id": symbol_id,
        "px": px,
        "qty": qty,
        "taker_side": 1,
        "ts_ns": 4,
        "seq": seq,
    }


class FakeFollow:
    def __init__(self, tmp_path, monkeypatch):
        self.frames = tmp_path / "frames.jl"
        self.args = tmp_path / "args"
        self.script = tmp_path / "rsx-cli"
        self.frames.write_text("")
        self.script.write_text(
            "#!/bin/sh\n"
            f"echo \"$@\" > '{self.args}'\n"
            f"cat '{self.frames}'\n"
            "exec sleep 60\n"
        )
        self.script.chmod(self.script.stat().st_mode | stat.S_IEXEC)
        monkeypatch.setattr(rsx_cli, "BINARY", self.script)
        monkeypatch.setattr(rsx_cli, "PID_DIR", tmp_path / "cli-pids")
        monkeypatch.setattr(md_feed, "PID_FILE", tmp_path / "cli-pids" / "md.pid")
        monkeypatch.setattr(md_feed, "BACKOFF_S", (0.05, 0.1))

    def prints(self, *frames):
        self.frames.write_text("".join(json.dumps(f) + "\n" for f in frames))

    def printed_args(self):
        return self.args.read_text().split()


@pytest.fixture
def fake_follow(tmp_path, monkeypatch):
    return FakeFollow(tmp_path, monkeypatch)


class Feed:
    def __init__(self):
        self.loop = FeedLoop()
        self.feed = md_feed.MdFeed("ws://md.test:1")

    def sync(self, *symbol_ids):
        self.loop.run(self.feed.sync(list(symbol_ids)))

    def close(self):
        self.loop.run(self.feed.stop())
        self.loop.close()


@pytest.fixture
def feed():
    feeding = Feed()
    yield feeding
    feeding.close()


def levels(book, side):
    return [(level["px"], level["qty"]) for level in book.get(side, [])]


def test_the_child_follows_the_symbols_at_the_url(fake_follow, feed):
    fake_follow.prints(snapshot(1, [(100, 5)], [(101, 7)]))
    feed.sync(10, 1)
    wait_for(lambda: feed.feed.books, "the snapshot")
    assert fake_follow.printed_args() == [
        "md", "follow", "1", "10", "-u", "ws://md.test:1", "--json"]
    assert feed.feed.status() == f"following pid {feed.feed.pid}"


def test_a_snapshot_is_the_book(fake_follow, feed):
    fake_follow.prints(
        snapshot(1, [(100, 5), (99, 3)], [(101, 7)]))
    feed.sync(1)
    wait_for(lambda: feed.feed.books, "the snapshot")
    book = feed.feed.books[1]
    assert levels(book, "bids") == [(100, 5), (99, 3)]
    assert levels(book, "asks") == [(101, 7)]


def test_deltas_set_and_remove_levels_best_first(fake_follow, feed):
    fake_follow.prints(
        snapshot(1, [(100, 5)], [(102, 7)]),
        delta(1, 0, 101, 4),
        delta(1, 1, 103, 2),
        delta(1, 1, 102, 0),
        delta(1, 0, 100, 9),
    )
    feed.sync(1)
    wait_for(lambda: levels(feed.feed.books.get(1, {}), "bids") == [(101, 4), (100, 9)],
             "the deltas")
    assert levels(feed.feed.books.get(1, {}), "asks") == [(103, 2)]


def test_a_delta_before_any_snapshot_starts_a_book(fake_follow, feed):
    fake_follow.prints(delta(7, 0, 50, 3))
    feed.sync(7)
    wait_for(lambda: 7 in feed.feed.books, "the delta")
    assert levels(feed.feed.books[7], "bids") == [(50, 3)]


def test_the_bbo_drops_levels_that_cross_the_touch(fake_follow, feed):
    fake_follow.prints(
        snapshot(1, [(103, 1), (100, 5), (98, 2)], [(98, 1), (101, 7), (103, 2)]),
        bbo(1, (100, 4), (101, 6)),
    )
    feed.sync(1)
    wait_for(lambda: levels(feed.feed.books.get(1, {}), "asks") == [(101, 6), (103, 2)],
             "the touch")
    assert levels(feed.feed.books[1], "bids") == [(100, 4), (98, 2)]


def test_an_empty_bbo_side_changes_nothing_on_that_side(fake_follow, feed):
    fake_follow.prints(
        snapshot(1, [(100, 5)], [(101, 7)]),
        bbo(1, (100, 8), (0, 0)),
    )
    feed.sync(1)
    wait_for(lambda: levels(feed.feed.books.get(1, {}), "bids") == [(100, 8)],
             "the touch")
    assert levels(feed.feed.books.get(1, {}), "asks") == [(101, 7)]


def test_trades_are_kept_with_the_dashboard_keys(fake_follow, feed):
    fake_follow.prints(trade(1, 300, 25, seq=66))
    feed.sync(1)
    wait_for(lambda: feed.feed.trades, "the trade")
    assert feed.feed.trades == [{
        "symbol_id": 1, "price": 300, "qty": 25, "taker_side": 1, "seq": 66}]


def test_the_trade_buffer_is_capped(fake_follow, feed, monkeypatch):
    monkeypatch.setattr(md_feed, "KEPT_TRADES", 10)
    fake_follow.prints(*[trade(1, 300, n, seq=n) for n in range(1, 26)])
    feed.sync(1)
    wait_for(lambda: feed.feed.trades and feed.feed.trades[-1]["seq"] == 25,
             "the last trade")
    assert 5 < len(feed.feed.trades) <= 10


def test_heartbeats_are_ignored(fake_follow, feed):
    fake_follow.prints(
        {"kind": "heartbeat", "ts_ms": 1}, snapshot(1, [(100, 5)], []))
    feed.sync(1)
    wait_for(lambda: 1 in feed.feed.books, "the snapshot")
    assert feed.feed.trades == []


def test_a_changed_symbol_list_restarts_the_child(fake_follow, feed):
    fake_follow.prints(snapshot(1, [(100, 5)], []))
    feed.sync(1)
    wait_for(lambda: feed.feed.pid, "the first child")
    first_pid = feed.feed.pid
    feed.sync(1, 3)
    wait_for(lambda: feed.feed.pid not in (None, first_pid), "a second child")
    wait_for(lambda: is_gone(first_pid), "the first child to exit")
    wait_for(lambda: fake_follow.printed_args()[2:4] == ["1", "3"], "new args")


def test_the_same_symbol_list_keeps_the_child(fake_follow, feed):
    fake_follow.prints(snapshot(1, [(100, 5)], []))
    feed.sync(3, 1)
    wait_for(lambda: feed.feed.pid, "the child")
    pid = feed.feed.pid
    feed.sync(1, 3)
    assert feed.feed.pid == pid
    assert feed.feed.restarts == 0


def test_an_empty_symbol_list_ends_the_child_and_the_book(fake_follow, feed):
    fake_follow.prints(snapshot(1, [(100, 5)], []))
    feed.sync(1)
    wait_for(lambda: feed.feed.books, "the snapshot")
    pid = feed.feed.pid
    feed.sync()
    wait_for(lambda: is_gone(pid), "the child to exit")
    assert feed.feed.pid is None
    assert feed.feed.books == {}
    assert feed.feed.status() == "not following"


def test_a_killed_child_is_restarted_and_the_book_comes_back(fake_follow, feed):
    fake_follow.prints(snapshot(1, [(100, 5)], []))
    feed.sync(1)
    wait_for(lambda: feed.feed.books, "the snapshot")
    first_pid = feed.feed.pid
    fake_follow.prints(snapshot(1, [(200, 9)], []))
    os.kill(first_pid, 9)
    wait_for(
        lambda: feed.feed.pid not in (None, first_pid)
        and feed.feed.books.get(1)
        and levels(feed.feed.books.get(1, {}), "bids") == [(200, 9)],
        "the restarted child's snapshot")
    assert feed.feed.restarts == 1


def test_the_book_is_dropped_while_the_child_is_down(fake_follow, feed):
    fake_follow.prints(snapshot(1, [(100, 5)], []))
    feed.sync(1)
    wait_for(lambda: feed.feed.books, "the snapshot")
    fake_follow.script.write_text(
        '#!/bin/sh\necho \'{"error": "failed to connect"}\' >&2\nexit 1\n')
    os.kill(feed.feed.pid, 9)
    wait_for(lambda: feed.feed.error.startswith("failed to connect"),
             "the failure")
    assert feed.feed.books == {}
    assert feed.feed.status() == "down: failed to connect"


def test_a_failing_child_backs_off(fake_follow, feed, monkeypatch):
    monkeypatch.setattr(md_feed, "BACKOFF_S", (0.2, 0.4))
    fake_follow.script.write_text(
        '#!/bin/sh\necho \'{"error": "refused"}\' >&2\nexit 1\n')
    feed.sync(1)
    time.sleep(1.0)
    assert 1 <= feed.feed.restarts <= 4


def test_a_frame_without_a_kind_is_an_error_not_a_crash(fake_follow, feed):
    fake_follow.prints({"symbol_id": 1})
    feed.sync(1)
    wait_for(lambda: "unexpected frame" in feed.feed.error, "the error")


def test_a_missing_binary_is_an_error_not_a_fallback(feed, tmp_path, monkeypatch):
    monkeypatch.setattr(rsx_cli, "BINARY", tmp_path / "no-such-cli")
    feed.sync(1)
    wait_for(lambda: "cannot start" in feed.feed.error, "the error")


def test_a_line_over_the_pipe_default_is_read(fake_follow, feed):
    wide = [(1000 - n, 1) for n in range(6000)]
    fake_follow.prints(snapshot(1, wide, []))
    assert len(json.dumps(snapshot(1, wide, []))) > 1 << 16
    feed.sync(1)
    wait_for(lambda: feed.feed.books, "the wide snapshot")
    assert len(feed.feed.books[1]["bids"]) == 6000


# ── the server's use of the feed ──────────────────────────


def view(*running):
    rows = [
        cluster_cli.Row(name, 100 + n, "running", 1, f"{name}.log")
        for n, name in enumerate(running)
    ]
    return cluster_cli.View("minimal", rows)


def test_the_server_follows_every_catalogued_symbol_while_marketdata_runs(
        fake_follow, feed, monkeypatch):
    monkeypatch.setattr(server, "md_records", feed.feed)
    feed.loop.run(server.sync_md_feed(view("marketdata", "maker")))
    assert feed.feed.symbol_ids == sorted(
        symbol.id for symbol in server.symbols.table.values())
    assert feed.feed.symbol_ids


def test_the_server_stops_following_when_marketdata_is_not_running(
        fake_follow, feed, monkeypatch):
    monkeypatch.setattr(server, "md_records", feed.feed)
    feed.loop.run(server.sync_md_feed(view("marketdata")))
    wait_for(lambda: feed.feed.pid, "the child")
    pid = feed.feed.pid
    feed.loop.run(server.sync_md_feed(view("maker")))
    wait_for(lambda: is_gone(pid), "the child to exit")
    assert feed.feed.symbol_ids == []


def test_the_book_routes_read_the_followed_book(
        client, fake_follow, feed, monkeypatch):
    monkeypatch.setattr(server, "md_records", feed.feed)
    fake_follow.prints(snapshot(10, [(30000, 5), (29900, 3)], [(30100, 7)]))
    feed.sync(10)
    wait_for(lambda: feed.feed.books, "the snapshot")
    book = client.get("/api/book/10").json()
    assert book["source"] == "live"
    assert book["bids"] == [
        {"px": 30000, "qty": 5}, {"px": 29900, "qty": 3}]
    assert book["asks"] == [{"px": 30100, "qty": 7}]
    assert client.get("/api/bbo/10").json()["ask_px"] == 30100
    assert "30100" in client.get("/x/book?symbol_id=10").text.replace(
        ".", "").replace(",", "")


def test_the_topology_shows_the_feed(feed, monkeypatch):
    monkeypatch.setattr(server, "md_records", feed.feed)
    rows = server._topo_marketdata([])["rows"]
    assert ("md follow", "not following") in rows
