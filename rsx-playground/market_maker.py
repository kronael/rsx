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
import logging
import os
import time
import random
import uuid
from pathlib import Path

logger = logging.getLogger(__name__)

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
        # unique 4-hex prefix per instance; prevents cid collisions
        # across restarts and concurrent workers.
        self._session_id = uuid.uuid4().hex[:4]

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
            # levels: approx per-side depth (bid + ask cids / 2)
            "levels": len(self.active_cids) // 2,
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

    async def _fetch_mark_prices(self) -> dict[int, int]:
        """Poll /api/mark/prices for index prices.

        Best-effort: returns empty dict on any failure.
        """
        port = os.environ.get(
            "RSX_PLAYGROUND_PORT", "49171"
        )
        url = f"http://localhost:{port}/api/mark/prices"
        try:
            async with aiohttp.ClientSession() as session:
                async with session.get(
                    url,
                    timeout=aiohttp.ClientTimeout(total=1),
                ) as resp:
                    if resp.status != 200:
                        return {}
                    data = await resp.json()
                    result: dict[int, int] = {}
                    for sid_str, entry in (
                        data.get("prices") or {}
                    ).items():
                        mark = entry.get("mark", 0)
                        if mark > 0:
                            result[int(sid_str)] = mark
                    return result
        except Exception:
            return {}

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

    async def _cancel_all(self):
        """Cancel all resting orders in the gateway before stopping."""
        if not self.active_cids:
            return
        headers = {"x-user-id": str(self.user_id)}
        try:
            async with aiohttp.ClientSession() as session:
                async with session.ws_connect(
                    self.gateway_url,
                    headers=headers,
                    timeout=aiohttp.ClientTimeout(total=3),
                ) as ws:
                    for cid in list(self.active_cids):
                        await ws.send_str(
                            json.dumps({"C": [cid]}))
                        self.cancels_sent += 1
        except Exception as e:
            self.errors.append(f"cancel_all: {e}")
        self.active_cids.clear()

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
        await self._cancel_all()

    def _next_cid(self):
        self._order_counter += 1
        # embed session_id so cids are unique across restarts
        raw = f"m{self._session_id}{self._order_counter}"
        return raw[:20].ljust(20, "0")

    async def _read_marketdata(self):
        """Subscribe to BBO and track mid prices.

        Reconnects with exponential backoff (1s→2s→…→16s).
        Circuit breaker trips after 8 consecutive infra failures.
        """
        delay = 1.0
        max_delay = 16.0
        circuit_at = 8
        consec_errors = 0

        while self._running:
            try:
                async with aiohttp.ClientSession() as session:
                    async with session.ws_connect(
                        self.marketdata_ws,
                        timeout=aiohttp.ClientTimeout(
                            total=5),
                    ) as ws:
                        for sid in self.symbol_ids:
                            # CHANNEL_BBO = 1 (server bitmask)
                            await ws.send_str(
                                json.dumps({"S": [sid, 1]})
                            )
                        consec_errors = 0  # connected; reset
                        delay = 1.0
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
            except asyncio.CancelledError:
                break
            except (ConnectionRefusedError, OSError):
                consec_errors += 1
            except Exception as e:
                consec_errors += 1
                self.errors.append(f"md: {e}")
                if len(self.errors) > 20:
                    del self.errors[:10]

            if consec_errors >= circuit_at:
                logger.warning(
                    "marketdata circuit breaker: %d consecutive "
                    "failures; stopping md subscription",
                    consec_errors,
                )
                break

            if self._running:
                await asyncio.sleep(delay)
                delay = min(delay * 2, max_delay)

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
                        # symbols format: [{"id", "tick_size", "lot_size"}]
                        rows = data.get("symbols") or []
                        for row in rows:
                            sid = int(row["id"])
                            tick = int(row.get("tick_size", 0))
                            lot = int(row.get("lot_size", 0))
                            if tick:
                                self.tick_sizes[sid] = tick
                            if lot:
                                self.lot_sizes[sid] = lot
        except Exception as e:
            self.errors.append(f"tick fetch: {e}")

    async def _preflight(self) -> bool:
        """Check gateway connectivity before quoting."""
        headers = {"x-user-id": str(self.user_id)}
        try:
            async with aiohttp.ClientSession() as session:
                async with session.ws_connect(
                    self.gateway_url,
                    headers=headers,
                    timeout=aiohttp.ClientTimeout(total=3),
                ) as ws:
                    # send a no-op heartbeat-ping to confirm the
                    # connection is live, not just TCP-accepted
                    await ws.ping()
                    return True
        except Exception as e:
            self.errors.append(f"preflight: {e}")
            if len(self.errors) > 20:
                del self.errors[:10]
            return False

    async def _run(self):
        """Main quoting loop."""
        # preflight: wait for gateway before placing any orders.
        # Exponential backoff 1s→2s→…→16s; circuit trips at 8
        # consecutive failures.
        delay = 1.0
        preflight_errors = 0
        preflight_circuit = 8
        while self._running:
            if await self._preflight():
                break
            preflight_errors += 1
            if preflight_errors >= preflight_circuit:
                logger.error(
                    "preflight circuit breaker: %d consecutive "
                    "failures; aborting maker",
                    preflight_errors,
                )
                self._running = False
                return
            logger.debug(
                "preflight failed (%d/%d); retrying in %.0fs",
                preflight_errors,
                preflight_circuit,
                delay,
            )
            await asyncio.sleep(delay)
            delay = min(delay * 2, 16.0)
        if not self._running:
            return

        # fetch tick sizes from server before quoting
        await self._fetch_tick_sizes(self.gateway_url)

        # default mid prices for symbols without live data;
        # prefer mark API prices over hardcoded defaults.
        defaults = {10: 50000, 1: 30000, 2: 2000, 3: 100}
        mark_prices = await self._fetch_mark_prices()
        for sid in self.symbol_ids:
            if sid not in self.mid_prices:
                if sid in mark_prices:
                    self.mid_prices[sid] = mark_prices[sid]
                else:
                    self.mid_prices[sid] = defaults.get(
                        sid, 50000)

        # Main quoting loop: circuit trips after 10 consecutive
        # infra errors to avoid thrashing on a broken gateway.
        quote_errors = 0
        quote_circuit = 10
        while self._running:
            # refresh mid from mark API for symbols
            # without live BBO
            try:
                mp = await self._fetch_mark_prices()
                for sid, px in mp.items():
                    if sid not in self.mid_prices or (
                        self.mid_prices[sid]
                        == defaults.get(sid, 50000)
                    ):
                        self.mid_prices[sid] = px
            except Exception:
                pass
            try:
                await self._quote_cycle()
                quote_errors = 0  # success resets counter
            except asyncio.CancelledError:
                break
            except (ConnectionRefusedError, OSError) as e:
                quote_errors += 1
                self.errors.append(f"gw: {e}")
                if len(self.errors) > 20:
                    del self.errors[:10]
            except Exception as e:
                quote_errors += 1
                self.errors.append(f"run: {e}")
                if len(self.errors) > 20:
                    del self.errors[:10]

            if quote_errors >= quote_circuit:
                logger.error(
                    "quote circuit breaker: %d consecutive "
                    "errors; aborting maker",
                    quote_errors,
                )
                self._running = False
                break

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
                        raw_qty = base_qty + random.randint(
                            0, base_qty // 2)
                        qty = raw_qty // lot * lot
                        if qty != raw_qty:
                            logger.debug(
                                "qty rounded %d -> %d for sym=%d",
                                raw_qty, qty, sid,
                            )

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
                        _sent = [bid_cid, ask_cid]
                        for _idx in range(2):
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
                                        self.active_cids.discard(
                                            _sent[_idx])
                            except asyncio.TimeoutError:
                                pass


if __name__ == "__main__":
    import os
    import signal

    def _env_int(key: str, default: int) -> int:
        try:
            return int(os.environ.get(key, ""))
        except (ValueError, TypeError):
            return default

    def _env_float(key: str, default: float) -> float:
        try:
            return float(os.environ.get(key, ""))
        except (ValueError, TypeError):
            return default

    maker = DummyMarketMaker(
        gateway_url=os.environ.get(
            "GATEWAY_URL", "ws://localhost:8080"),
        marketdata_ws=os.environ.get(
            "MARKETDATA_WS", "ws://localhost:8180"),
        spread_bps=_env_int("RSX_MAKER_SPREAD_BPS", 10),
        qty_per_level=_env_int("RSX_MAKER_QTY", 10),
        num_levels=_env_int("RSX_MAKER_LEVELS", 5),
        refresh_sec=_env_float("RSX_MAKER_REFRESH_MS", 2000) / 1000.0,
        symbol_ids=(
            [_env_int("RSX_MAKER_SYMBOL", 10)]
        ),
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
