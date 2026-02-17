"""Dummy market maker for RSX playground.

Places limit orders on both sides at configurable spread
around a mid price. Reads BBO from marketdata WS when
available, otherwise uses a default mid.
"""

import asyncio
import json
import time
import random

import aiohttp


class DummyMarketMaker:
    """Simple market maker that quotes around mid price."""

    def __init__(
        self,
        gateway_url="ws://localhost:8080",
        marketdata_ws="ws://localhost:8081",
        symbol_ids=None,
        spread_bps=10,
        qty_per_level=10,
        num_levels=5,
        refresh_sec=2.0,
        user_id=99,
    ):
        self.gateway_url = gateway_url
        self.marketdata_ws = marketdata_ws
        self.symbol_ids = symbol_ids or [10]
        self.spread_bps = spread_bps
        self.qty_per_level = qty_per_level
        self.num_levels = num_levels
        self.refresh_sec = refresh_sec
        self.user_id = user_id

        self._task = None
        self._md_task = None
        self._running = False
        self._order_counter = 0

        # state
        self.mid_prices: dict[int, int] = {}
        self.orders_placed = 0
        self.cancels_sent = 0
        self.errors: list[str] = []
        self.active_cids: set[str] = set()

    @property
    def running(self):
        return self._running

    def status(self):
        return {
            "running": self._running,
            "symbol_ids": self.symbol_ids,
            "mid_prices": {
                str(k): v for k, v in self.mid_prices.items()
            },
            "orders_placed": self.orders_placed,
            "cancels_sent": self.cancels_sent,
            "active_orders": len(self.active_cids),
            "errors": self.errors[-5:],
            "spread_bps": self.spread_bps,
            "num_levels": self.num_levels,
            "refresh_sec": self.refresh_sec,
        }

    def start(self):
        if self._running:
            return
        self._running = True
        self._task = asyncio.create_task(self._run())
        self._md_task = asyncio.create_task(
            self._read_marketdata()
        )

    async def stop(self):
        self._running = False
        if self._md_task:
            self._md_task.cancel()
            try:
                await self._md_task
            except (asyncio.CancelledError, Exception):
                pass
            self._md_task = None
        if self._task:
            self._task.cancel()
            try:
                await self._task
            except (asyncio.CancelledError, Exception):
                pass
            self._task = None
        self.active_cids.clear()

    def _next_cid(self):
        self._order_counter += 1
        raw = f"mm{self._order_counter}"
        return raw[:20].ljust(20, "0")

    async def _read_marketdata(self):
        """Subscribe to BBO and track mid prices."""
        while self._running:
            try:
                async with aiohttp.ClientSession() as session:
                    async with session.ws_connect(
                        self.marketdata_ws,
                        timeout=aiohttp.ClientTimeout(
                            total=5),
                    ) as ws:
                        for sid in self.symbol_ids:
                            await ws.send_str(json.dumps({
                                "sub": {
                                    "sym": sid,
                                    "ch": ["BBO"],
                                },
                            }))
                        async for msg in ws:
                            if not self._running:
                                break
                            if msg.type != aiohttp.WSMsgType.TEXT:
                                continue
                            data = json.loads(msg.data)
                            if "BBO" in data:
                                bbo = data["BBO"]
                                sym = bbo[0]
                                bid_px = bbo[1]
                                ask_px = bbo[4]
                                if bid_px > 0 and ask_px > 0:
                                    self.mid_prices[sym] = (
                                        (bid_px + ask_px) // 2
                                    )
            except (
                ConnectionRefusedError,
                OSError,
                asyncio.CancelledError,
            ):
                pass
            except Exception as e:
                self.errors.append(f"md: {e}")
                if len(self.errors) > 20:
                    del self.errors[:10]
            if self._running:
                await asyncio.sleep(2.0)

    async def _run(self):
        """Main quoting loop."""
        # default mid prices for symbols without live data
        defaults = {10: 50000, 1: 30000, 2: 2000, 3: 100}
        for sid in self.symbol_ids:
            if sid not in self.mid_prices:
                self.mid_prices[sid] = defaults.get(
                    sid, 50000)

        while self._running:
            try:
                await self._quote_cycle()
            except asyncio.CancelledError:
                break
            except (ConnectionRefusedError, OSError) as e:
                self.errors.append(f"gw: {e}")
                if len(self.errors) > 20:
                    del self.errors[:10]
            except Exception as e:
                self.errors.append(f"run: {e}")
                if len(self.errors) > 20:
                    del self.errors[:10]
            if self._running:
                await asyncio.sleep(self.refresh_sec)

    async def _quote_cycle(self):
        """Cancel stale orders and place new quotes."""
        headers = {"x-user-id": str(self.user_id)}
        async with aiohttp.ClientSession() as session:
            async with session.ws_connect(
                self.gateway_url,
                headers=headers,
                timeout=aiohttp.ClientTimeout(total=5),
            ) as ws:
                # cancel existing orders
                for cid in list(self.active_cids):
                    await ws.send_str(json.dumps({
                        "X": [cid],
                    }))
                    self.cancels_sent += 1
                    # drain response
                    try:
                        await asyncio.wait_for(
                            ws.receive(), timeout=0.1)
                    except asyncio.TimeoutError:
                        pass
                self.active_cids.clear()

                # place new orders
                for sid in self.symbol_ids:
                    mid = self.mid_prices.get(sid, 50000)
                    half_spread = max(
                        1,
                        mid * self.spread_bps // 10000,
                    )
                    level_step = max(1, half_spread // 2)

                    for i in range(self.num_levels):
                        offset = half_spread + i * level_step
                        qty = self.qty_per_level + random.randint(
                            0, self.qty_per_level // 2)

                        # bid
                        bid_cid = self._next_cid()
                        bid_px = mid - offset
                        await ws.send_str(json.dumps({
                            "N": [sid, 0, bid_px, qty, bid_cid, 0],
                        }))
                        self.active_cids.add(bid_cid)
                        self.orders_placed += 1

                        # ask
                        ask_cid = self._next_cid()
                        ask_px = mid + offset
                        await ws.send_str(json.dumps({
                            "N": [sid, 1, ask_px, qty, ask_cid, 0],
                        }))
                        self.active_cids.add(ask_cid)
                        self.orders_placed += 1

                        # drain responses
                        for _ in range(2):
                            try:
                                resp = await asyncio.wait_for(
                                    ws.receive(), timeout=0.2)
                                if resp.type == aiohttp.WSMsgType.TEXT:
                                    data = json.loads(resp.data)
                                    if "E" in data:
                                        self.errors.append(
                                            f"order: {data['E']}")
                                        if len(self.errors) > 20:
                                            del self.errors[:10]
                            except asyncio.TimeoutError:
                                pass


if __name__ == "__main__":
    import os
    import signal

    maker = DummyMarketMaker(
        gateway_url=os.environ.get(
            "GATEWAY_URL", "ws://localhost:8080"),
        marketdata_ws=os.environ.get(
            "MARKETDATA_WS", "ws://localhost:8081"),
    )

    async def main():
        maker.start()
        stop_event = asyncio.Event()

        def _handle_sig(*_):
            stop_event.set()

        loop = asyncio.get_running_loop()
        loop.add_signal_handler(signal.SIGTERM, _handle_sig)
        loop.add_signal_handler(signal.SIGINT, _handle_sig)
        await stop_event.wait()
        await maker.stop()

    asyncio.run(main())
