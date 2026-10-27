"""The marketdata book and trades, read from one `rsx-cli md follow` child.

`MdFeed` follows the catalogued symbols over a single long-lived child and
folds its frames into `books` and `trades`. A child that exits (marketdata
restarted or stopped, a bad frame) is restarted with backoff and the books
are dropped until its snapshots arrive; a symbol list that changes restarts
the child. The playground decodes no marketdata bytes: a frame is the CLI's
`kind`-tagged JSON object.
"""

import asyncio
import logging
import time
from typing import Any

import rsx_cli

log = logging.getLogger(__name__)

PID_FILE = rsx_cli.PID_DIR / "md-follow.pid"
BACKOFF_S = (0.5, 1.0, 2.0, 5.0)
HEALTHY_RUN_S = 30.0
KEPT_TRADES = 200
YIELD_EVERY = 500
LINE_LIMIT = 1 << 24
BID = "bids"
ASK = "asks"

Level = dict[str, int]
Book = dict[str, list[Level]]


def set_level(book: Book, side: str, px: int, qty: int) -> None:
    levels = book[side]
    if qty == 0:
        book[side] = [level for level in levels if level["px"] != px]
        return
    for level in levels:
        if level["px"] == px:
            level["qty"] = qty
            break
    else:
        levels.append({"px": px, "qty": qty})
    levels.sort(key=lambda level: level["px"], reverse=side == BID)


class MdFeed:
    def __init__(self, url: str) -> None:
        self.url = url
        self.symbol_ids: list[int] = []
        self.books: dict[int, Book] = {}
        self.trades: list[dict[str, int]] = []
        self.task: asyncio.Task[None] | None = None
        self.pid: int | None = None
        self.restarts = 0
        self.error = ""

    async def sync(self, symbol_ids: list[int]) -> None:
        """Follows exactly `symbol_ids`: starts the child, restarts it when
        the list changed, ends it when the list is empty."""
        symbol_ids = sorted(symbol_ids)
        is_following = self.task is not None and not self.task.done()
        if symbol_ids == self.symbol_ids and is_following == bool(symbol_ids):
            return
        await self.stop()
        self.symbol_ids = symbol_ids
        if symbol_ids:
            self.task = asyncio.create_task(
                self.follow_loop(symbol_ids), name="md-follow"
            )

    async def stop(self) -> None:
        task = self.task
        self.task = None
        self.symbol_ids = []
        if task is not None:
            task.cancel()
            await asyncio.gather(task, return_exceptions=True)
        self.pid = None
        self.books.clear()

    def status(self) -> str:
        if self.pid is not None:
            return f"following pid {self.pid}"
        if self.error:
            return f"down: {self.error}"
        return "not following"

    async def follow_loop(self, symbol_ids: list[int]) -> None:
        failures = 0
        while True:
            started = time.monotonic()
            try:
                await self.do_follow(symbol_ids)
            except rsx_cli.CliError as err:
                self.error = str(err)
            except (KeyError, TypeError, ValueError) as err:
                self.error = f"unexpected frame: {type(err).__name__}: {err}"
            self.pid = None
            self.books.clear()
            self.restarts += 1
            lived_s = time.monotonic() - started
            failures = 1 if lived_s > HEALTHY_RUN_S else failures + 1
            delay = BACKOFF_S[min(failures, len(BACKOFF_S)) - 1]
            if failures == 1:
                log.warning(
                    "md follow: %s; restarting in %.1fs", self.error, delay
                )
            await asyncio.sleep(delay)

    async def do_follow(self, symbol_ids: list[int]) -> None:
        args = ("md", "follow", *map(str, symbol_ids), "-u", self.url, "--json")
        async with rsx_cli.follow(
            *args, pid_file=PID_FILE, line_limit=LINE_LIMIT
        ) as child:
            self.pid = child.pid
            count = 0
            async for frame in child.lines():
                self.error = ""
                self.ingest(frame)
                count += 1
                if count % YIELD_EVERY == 0:
                    await asyncio.sleep(0)
            error = await child.exit_error()
        if error.code == 0:
            raise rsx_cli.CliError("rsx-cli exited", 0)
        raise error

    def ingest(self, frame: dict[str, Any]) -> None:
        kind = frame["kind"]
        if kind == "l2_snapshot":
            self.books[frame["symbol_id"]] = {
                BID: [{"px": px, "qty": qty} for px, qty, _ in frame["bids"]],
                ASK: [{"px": px, "qty": qty} for px, qty, _ in frame["asks"]],
            }
        elif kind == "l2_delta":
            book = self.book_of(frame["symbol_id"])
            side = BID if frame["side"] == 0 else ASK
            set_level(book, side, frame["px"], frame["qty"])
        elif kind == "bbo":
            self.reconcile_touch(frame)
        elif kind == "trade":
            self.trades.append(
                {
                    "symbol_id": frame["symbol_id"],
                    "price": frame["px"],
                    "qty": frame["qty"],
                    "taker_side": frame["taker_side"],
                    "seq": frame["seq"],
                }
            )
            if len(self.trades) > KEPT_TRADES:
                del self.trades[: KEPT_TRADES // 2]

    def book_of(self, symbol_id: int) -> Book:
        return self.books.setdefault(symbol_id, {BID: [], ASK: []})

    def reconcile_touch(self, frame: dict[str, Any]) -> None:
        """The BBO is the authoritative touch: a dropped depth delta leaves a
        stale level that would cross it, so levels through the touch go and
        the top of each side is set."""
        book = self.book_of(frame["symbol_id"])
        bid_px = frame["bid_px"]
        ask_px = frame["ask_px"]
        if ask_px:
            book[BID] = [level for level in book[BID] if level["px"] < ask_px]
        if bid_px:
            book[ASK] = [level for level in book[ASK] if level["px"] > bid_px]
        if bid_px:
            set_level(book, BID, bid_px, frame["bid_qty"])
        if ask_px:
            set_level(book, ASK, ask_px, frame["ask_qty"])
