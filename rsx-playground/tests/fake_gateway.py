# pyright: basic
"""A gateway on a loopback port for tests that run the real rsx-cli binary.

It serves `GET /v1/symbols` and the order WebSocket; `answer(index, frame)`
returns the frames sent back for the index-th order or cancel frame.
"""

import asyncio
import json
import threading
from collections.abc import Callable

from aiohttp import WSMsgType
from aiohttp import web

SYMBOLS = {
    "symbols": [
        {
            "id": 10,
            "tick_size": 1,
            "lot_size": 100000,
            "price_decimals": 6,
            "qty_decimals": 4,
        }
    ]
}
OID = "0190000000007000800000000000000a"
RESTING_QTY = 300000


def update(status, filled=0, remaining=0, reason=0, oid=OID):
    return {"U": [oid, status, filled, remaining, reason, 0, 0, 0, 0, 0]}


def fill(price, qty, taker=OID):
    return {"F": [taker, "m" * 32, price, qty, 5000, 0, 0, 0, 0, 0, 0]}


def resting(index, frame):
    return [update(1, 0, RESTING_QTY)]


class FakeGateway:
    def __init__(
        self, answer: Callable[[int, dict], list[dict]] = resting, symbols=SYMBOLS
    ):
        self.answer = answer
        self.symbol_catalog = symbols
        self.frames: list[dict] = []
        self.authorizations: list[str] = []
        self.loop = asyncio.new_event_loop()
        self.thread = threading.Thread(target=self.loop.run_forever, daemon=True)
        self.thread.start()
        self.runner = None
        self.url = asyncio.run_coroutine_threadsafe(self.serve(), self.loop).result(10)

    async def serve(self) -> str:
        app = web.Application()
        app.router.add_get("/v1/symbols", self.symbols)
        app.router.add_get("/{tail:.*}", self.socket)
        self.runner = web.AppRunner(app)
        await self.runner.setup()
        site = web.TCPSite(self.runner, "127.0.0.1", 0)
        await site.start()
        return f"ws://127.0.0.1:{self.runner.addresses[0][1]}"

    async def symbols(self, request):
        return web.json_response(self.symbol_catalog)

    async def socket(self, request):
        self.authorizations.append(request.headers.get("Authorization", ""))
        ws = web.WebSocketResponse()
        await ws.prepare(request)
        await ws.send_str(json.dumps({"H": [1]}))
        async for message in ws:
            if message.type != WSMsgType.TEXT:
                continue
            frame = json.loads(message.data)
            if "H" in frame:
                continue
            index = len(self.frames)
            self.frames.append(frame)
            for reply in self.answer(index, frame):
                await ws.send_str(json.dumps(reply))
        return ws

    def stop(self) -> None:
        asyncio.run_coroutine_threadsafe(self.runner.cleanup(), self.loop).result(10)
        self.loop.call_soon_threadsafe(self.loop.stop)
        self.thread.join(timeout=5)
        self.loop.close()


class ScriptedGateway:
    """Stands in for `order_cli.Gateway`: each call takes the next scripted
    outcome, or raises it when it is an exception."""

    def __init__(self, scripted):
        self.scripted = list(scripted)
        self.placed = []
        self.cancelled = []

    def next(self):
        item = self.scripted.pop(0)
        if isinstance(item, Exception):
            raise item
        return item

    async def place(self, order):
        self.placed.append(order)
        return self.next()

    async def cancel(self, user_id, key):
        self.cancelled.append((user_id, key))
        return self.next()
