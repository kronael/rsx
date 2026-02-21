"""Dummy market maker for RSX playground.

Places limit orders on both sides at configurable spread
around a mid price. Reads BBO from marketdata WS when
available, otherwise uses a default mid.

Mid override precedence (highest to lowest):
  1. tmp/maker-config.json `mid_override` key (polled each cycle)
  2. RSX_MAKER_MID_OVERRIDE env var (integer raw price units)
  3. Live BBO from marketdata WS
  4. Built-in defaults per symbol
"""

import asyncio
import json
import os
import time
import random
from pathlib import Path

import aiohttp

_STATUS_FILE = (
    Path(__file__).resolve().parent.parent / "tmp" / "maker-status.json"
)
_CONFIG_FILE = (
    Path(__file__).resolve().parent.parent / "tmp" / "maker-config.json"
)


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
        tick_sizes=None,
    ):
        self.gateway_url = gateway_url
        self.marketdata_ws = marketdata_ws
        self.symbol_ids = symbol_ids or [10]
        # tick_sizes / lot_sizes: {symbol_id: size}; defaults to 1
        self.tick_sizes = tick_sizes or {}
        self.lot_sizes: dict[int, int] = {}
        self.spread_bps = spread_bps
        self.qty_per_level = qty_per_level
        self.num_levels = num_levels
        self.refresh_sec = refresh_sec
        self.user_id = user_id

        self._task = None
        self._md_task = None
        self._running = False
        self._order_counter = 0

        # env-var mid override (set once at construction)
        _env = os.environ.get("RSX_MAKER_MID_OVERRIDE", "").strip()
        self._env_mid_override: int | None = (
            int(_env) if _env else None
        )

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

    def _read_config_mid(self) -> int | None:
        """Poll tmp/maker-config.json for mid_override."""
        try:
            text = _CONFIG_FILE.read_text()
            data = json.loads(text)
            val = data.get("mid_override")
            if val is not None:
                return int(val)
        except (FileNotFoundError, json.JSONDecodeError, ValueError):
            pass
        return None

    def _effective_mid_override(self) -> int | None:
        """Return active mid override or None."""
        cfg = self._read_config_mid()
        if cfg is not None:
            return cfg
        return self._env_mid_override

    def start(self):
        if self._running:
            return
        self._running = True
        self._task = asyncio.create_task(self._run())
        # skip MD subscription when env override is set;
        # config-file override is checked per cycle so still
        # need to know if env disables MD entirely.
        if self._env_mid_override is None:
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

    def _write_status(self):
        """Write status dict to tmp/maker-status.json."""
        try:
            _STATUS_FILE.parent.mkdir(parents=True, exist_ok=True)
            data = self.status()
            data["pid"] = os.getpid()
            tmp = _STATUS_FILE.with_suffix(".tmp")
            tmp.write_text(json.dumps(data))
            tmp.replace(_STATUS_FILE)
        except Exception:
            pass

    async def _fetch_tick_sizes(self, base_url: str):
        """Fetch tick+lot sizes from RSX_SYMBOLS_URL or /v1/symbols."""
        import os
        symbols_url = os.environ.get("RSX_SYMBOLS_URL")
        if not symbols_url:
            from urllib.parse import urlparse
            url = base_url.replace(
                "ws://", "http://").replace(
                "wss://", "https://")
            parsed = urlparse(url)
            symbols_url = (
                f"{parsed.scheme}://{parsed.netloc}/v1/symbols"
            )
        try:
            async with aiohttp.ClientSession() as session:
                async with session.get(
                    symbols_url,
                    timeout=aiohttp.ClientTimeout(total=3),
                ) as resp:
                    if resp.status == 200:
                        data = await resp.json()
                        # M format: [[id, tick, lot, name], ...]
                        rows = data.get("M") or []
                        for row in rows:
                            if len(row) >= 3:
                                sid, tick, lot = (
                                    int(row[0]), int(row[1]),
                                    int(row[2]),
                                )
                                if tick:
                                    self.tick_sizes[sid] = tick
                                if lot:
                                    self.lot_sizes[sid] = lot
        except Exception as e:
            self.errors.append(f"tick fetch: {e}")

    async def _run(self):
        """Main quoting loop."""
        # fetch tick sizes from server before quoting
        await self._fetch_tick_sizes(self.gateway_url)

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
            self._write_status()
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
                        "C": [cid],
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
                mid_override = self._effective_mid_override()
                for sid in self.symbol_ids:
                    mid = (
                        mid_override
                        if mid_override is not None
                        else self.mid_prices.get(sid, 50000)
                    )
                    half_spread = max(
                        1,
                        mid * self.spread_bps // 10000,
                    )
                    level_step = max(1, half_spread // 2)

                    tick = self.tick_sizes.get(sid, 1) or 1
                    lot = self.lot_sizes.get(sid, 1) or 1
                    # base qty: at least 1 lot, scaled by qty_per_level
                    base_qty = max(lot, self.qty_per_level * lot)
                    for i in range(self.num_levels):
                        offset = half_spread + i * level_step
                        qty = base_qty + random.randint(
                            0, base_qty // 2) // lot * lot

                        # bid — round down to tick boundary
                        bid_cid = self._next_cid()
                        bid_px = (mid - offset) // tick * tick
                        await ws.send_str(json.dumps({
                            "N": [sid, 0, bid_px, qty, bid_cid, 0],
                        }))
                        self.active_cids.add(bid_cid)
                        self.orders_placed += 1

                        # ask — round up to tick boundary
                        ask_cid = self._next_cid()
                        raw_ask = mid + offset
                        ask_px = (
                            (raw_ask + tick - 1) // tick * tick
                        )
                        await ws.send_str(json.dumps({
                            "N": [sid, 1, ask_px, qty, ask_cid, 0],
                        }))
                        self.active_cids.add(ask_cid)
                        self.orders_placed += 1

                        # drain responses; evict filled cids
                        for _ in range(2):
                            try:
                                resp = await asyncio.wait_for(
                                    ws.receive(), timeout=0.2)
                                if resp.type == aiohttp.WSMsgType.TEXT:
                                    data = json.loads(resp.data)
                                    # evict filled/done orders
                                    if "F" in data or "D" in data:
                                        cid = (data.get("F") or
                                               data.get("D", [None]))[0]
                                        self.active_cids.discard(cid)
                                    if "E" in data:
                                        self.errors.append(
                                            f"order: {data['E']}")
                                        if len(self.errors) > 20:
                                            del self.errors[:10]
                                        # evict both cids for
                                        # this level; we can't
                                        # tell which was rejected
                                        self.active_cids.discard(
                                            bid_cid)
                                        self.active_cids.discard(
                                            ask_cid)
                            except asyncio.TimeoutError:
                                pass


if __name__ == "__main__":
    import os
    import signal

    maker = DummyMarketMaker(
        gateway_url=os.environ.get(
            "GATEWAY_URL", "ws://localhost:8080"),
        marketdata_ws=os.environ.get(
            "MARKETDATA_WS", "ws://localhost:8180"),
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
