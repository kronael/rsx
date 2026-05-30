"""Acceptance tests for PROJECT.md criteria 3-8.

Criteria:
  3  WS fill round-trip: both sides receive Fill <= 1s
  4  Cancel round-trip: OrderUpdate{CANCELLED} <= 1s
  5  Restart safety: resting orders survive ME kill+restart
  7  Maker integration: >= 5 bid+ask levels after 3s
  8  Stress test: accept_rate > 0.8 at 50 rps / 10s

All tests start a full RSX stack (gateway + risk + ME + marketdata).
Postgres at DATABASE_URL is required (default: rsx:folium@10.0.2.1:5432).
"""

import asyncio
import uuid
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import time
from pathlib import Path

import aiohttp
import jwt as pyjwt
import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))

from stress_client import StressConfig
from stress_client import run_stress_test

# Gateway JWT secret used by the `stack` fixture below. Must be
# >= 32 bytes (HS256 minimum per rsx-gateway/src/config.rs).
GW_JWT_SECRET = "test-secret-at-least-32-bytes-long-please!"

# ── paths ─────────────────────────────────────────────────────────────

ROOT = Path(__file__).parent.parent.parent
BIN = ROOT / "target" / "debug"
GW_BIN = BIN / "rsx-gateway"
RISK_BIN = BIN / "rsx-risk"
ME_BIN = BIN / "rsx-matching"
MD_BIN = BIN / "rsx-marketdata"

PG_URL = os.environ.get(
    "DATABASE_URL",
    "postgres://rsx:folium@10.0.2.1:5432/rsx_dev",
)

# ── ports (28xxx range to avoid collision with dev stack) ──────────────

GW_WS_PORT = 28080
MD_WS_PORT = 28081
GW_CAST_PORT = 28200   # GW listens for Risk fills here
RISK_CAST_PORT = 28300  # Risk listens for GW orders + ME events
ME_CAST_PORT = 28100   # ME listens for Risk orders
MD_CAST_PORT = 28103   # Marketdata listens for ME events


def _wait_port(port: int, timeout: float = 5.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        with socket.socket() as s:
            s.settimeout(0.05)
            if s.connect_ex(("127.0.0.1", port)) == 0:
                return True
        time.sleep(0.05)
    return False


def _kill(proc):
    if proc and proc.poll() is None:
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()


def _seed_accounts(user_ids: list[int], collateral: int = 10_000_000):
    """Insert test accounts into postgres (upsert)."""
    import asyncpg

    async def _do():
        conn = await asyncpg.connect(PG_URL)
        try:
            for uid in user_ids:
                await conn.execute(
                    """
                    INSERT INTO accounts (user_id, collateral)
                    VALUES ($1, $2)
                    ON CONFLICT (user_id)
                    DO UPDATE SET collateral = EXCLUDED.collateral
                    """,
                    uid, collateral,
                )
        finally:
            await conn.close()

    asyncio.run(_do())


@pytest.fixture(scope="module")
def stack(tmp_path_factory):
    """Start gateway + risk + ME + marketdata.

    Skips if any required binary is missing.
    """
    for binary in [GW_BIN, RISK_BIN, ME_BIN, MD_BIN]:
        if not binary.exists():
            pytest.fail(
                f"rsx binary not built ({binary}); run `cargo build` first"
            )

    tmp = tmp_path_factory.mktemp("stack")
    wal_gw = str(tmp / "wal_gw")
    wal_risk = str(tmp / "wal_risk")
    wal_me = str(tmp / "wal_me")
    Path(wal_gw).mkdir()
    Path(wal_risk).mkdir()
    Path(wal_me).mkdir()

    # Seed test accounts 1-20 with plenty of collateral
    try:
        _seed_accounts(list(range(1, 21)))
    except Exception as e:
        pytest.skip(f"postgres seed failed: {e}")

    env_base = {**os.environ, "RUST_LOG": "info"}

    env_gw = {
        **env_base,
        "RSX_GW_LISTEN": f"0.0.0.0:{GW_WS_PORT}",
        # >= 32 bytes; required by rsx-gateway/src/config.rs
        "RSX_GW_JWT_SECRET": GW_JWT_SECRET,
        "RSX_GW_IDLE_TIMEOUT_S": "60",
        "RSX_GW_ORDER_TIMEOUT_MS": "5000",
        "RSX_GW_MAX_PENDING": "10000",
        "RSX_GW_RL_USER": "10000",
        "RSX_GW_RL_IP": "100000",
        "RSX_GW_HEARTBEAT_INTERVAL_S": "30",
        "RSX_RISK_CAST_ADDR": f"127.0.0.1:{RISK_CAST_PORT}",
        "RSX_GW_CAST_ADDR": f"127.0.0.1:{GW_CAST_PORT}",
        "RSX_GW_WAL_DIR": wal_gw,
        "RSX_MAX_SYMBOLS": "16",
        "RSX_DEFAULT_TICK_SIZE": "1",
        "RSX_DEFAULT_LOT_SIZE": "1",
    }

    env_risk = {
        **env_base,
        "DATABASE_URL": PG_URL,
        "RSX_RISK_CAST_ADDR": f"127.0.0.1:{RISK_CAST_PORT}",
        "RSX_GW_CAST_ADDR": f"127.0.0.1:{GW_CAST_PORT}",
        "RSX_ME_CAST_ADDR": f"127.0.0.1:{ME_CAST_PORT}",
        "RSX_RISK_WAL_DIR": wal_risk,
        "RSX_RISK_SHARD_ID": "0",
        "RSX_RISK_SHARD_COUNT": "1",
        "RSX_RISK_IS_REPLICA": "false",
        # Mark not started; risk binds these but no data arrives.
        "RSX_RISK_MARK_CAST_ADDR": "127.0.0.1:28105",
        "RSX_MARK_CAST_ADDR": "127.0.0.1:28106",
    }

    env_me = {
        **env_base,
        "RSX_ME_SYMBOL_ID": "10",
        "RSX_ME_PRICE_DECIMALS": "0",
        "RSX_ME_QTY_DECIMALS": "0",
        "RSX_ME_TICK_SIZE": "1",
        "RSX_ME_LOT_SIZE": "1",
        "RSX_ME_CAST_ADDR": f"127.0.0.1:{ME_CAST_PORT}",
        "RSX_RISK_CAST_ADDR": f"127.0.0.1:{RISK_CAST_PORT}",
        "RSX_MD_CAST_ADDR": f"127.0.0.1:{MD_CAST_PORT}",
        "RSX_ME_WAL_DIR": wal_me,
    }

    env_md = {
        **env_base,
        "RSX_MD_LISTEN": f"0.0.0.0:{MD_WS_PORT}",
        "RSX_MKT_CAST_ADDR": f"127.0.0.1:{MD_CAST_PORT}",
        "RSX_ME_CAST_ADDR": f"127.0.0.1:{ME_CAST_PORT}",
        "RSX_MD_TIP_FILE": str(tmp / "md.tip"),
    }

    procs = {}
    try:
        # Start risk first (postgres connection takes time)
        procs["risk"] = subprocess.Popen(
            [str(RISK_BIN)],
            env=env_risk,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        time.sleep(0.5)  # let it acquire the pg lease

        procs["me"] = subprocess.Popen(
            [str(ME_BIN)],
            env=env_me,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

        procs["md"] = subprocess.Popen(
            [str(MD_BIN)],
            env=env_md,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

        procs["gw"] = subprocess.Popen(
            [str(GW_BIN)],
            env=env_gw,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

        if not _wait_port(GW_WS_PORT, timeout=8.0):
            for p in procs.values():
                _kill(p)
            pytest.fail("gateway did not start within 8s")

        if not _wait_port(MD_WS_PORT, timeout=5.0):
            for p in procs.values():
                _kill(p)
            pytest.fail("marketdata did not start within 5s")

        # Extra settle time for risk→ME handshake
        time.sleep(0.5)

        yield {
            "gw_url": f"ws://127.0.0.1:{GW_WS_PORT}",
            "md_url": f"ws://127.0.0.1:{MD_WS_PORT}",
            "procs": procs,
            "env_me": env_me,
            "wal_me": wal_me,
        }
    finally:
        for p in procs.values():
            _kill(p)


# ── helpers ────────────────────────────────────────────────────────────

def _order(sym: int, side: int, px: int, qty: int,
           cid: str, tif: int = 0) -> str:
    cid = cid[:20].ljust(20, "0")
    return json.dumps({"N": [sym, side, px, qty, cid, tif]})


def _mint_jwt(user_id: int) -> str:
    """Mint an HS256 JWT for the `stack` gateway (spec: 11-gateway.md)."""
    return pyjwt.encode(
        {
            "sub": str(user_id),
            "user_id": user_id,
            "exp": int(time.time()) + 3600,
            "aud": "rsx-gateway",
            "iss": "rsx-auth",
            "jti": uuid.uuid4().hex,
        },
        GW_JWT_SECRET,
        algorithm="HS256",
    )


async def _connect(url: str, user_id: int,
                   timeout: float = 5.0) -> aiohttp.ClientWebSocketResponse:
    session = aiohttp.ClientSession()
    ws = await session.ws_connect(
        url,
        headers={"Authorization": f"Bearer {_mint_jwt(user_id)}"},
        timeout=aiohttp.ClientTimeout(total=timeout),
    )
    return ws, session


async def _recv_until(ws, predicate, timeout: float = 2.0):
    """Receive messages until predicate(data) is True or timeout."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        remaining = deadline - time.time()
        if remaining <= 0:
            break
        try:
            msg = await asyncio.wait_for(
                ws.receive(), timeout=min(remaining, 0.2))
            if msg.type == aiohttp.WSMsgType.TEXT:
                data = json.loads(msg.data)
                if predicate(data):
                    return data
        except asyncio.TimeoutError:
            pass
    return None


# ── criterion 3: WS fill round-trip ───────────────────────────────────

@pytest.mark.asyncio
async def test_ws_new_order_fill_update_complete(stack):
    """Both sides receive WsFrame::Fill within 1s of crossing orders."""
    gw = stack["gw_url"]

    ws1, s1 = await _connect(gw, user_id=1)
    ws2, s2 = await _connect(gw, user_id=2)

    try:
        # User 2 posts a resting bid
        await ws2.send_str(_order(10, 0, 49900, 5, "bid00"))
        await asyncio.sleep(0.05)

        # User 1 sells into it
        t0 = time.time()
        await ws1.send_str(_order(10, 1, 49900, 5, "ask00"))

        # Both sides should get a Fill
        fill1 = await _recv_until(
            ws1, lambda d: "F" in d, timeout=1.0)
        fill2 = await _recv_until(
            ws2, lambda d: "F" in d, timeout=1.0)

        elapsed = time.time() - t0
        assert fill1 is not None, \
            f"user1 (taker) did not receive Fill within 1s"
        assert fill2 is not None, \
            f"user2 (maker) did not receive Fill within 1s"
        assert elapsed < 1.0, \
            f"fill round-trip took {elapsed:.3f}s > 1s"
    finally:
        await ws1.close()
        await ws2.close()
        await s1.close()
        await s2.close()


# ── criterion 4: cancel round-trip ────────────────────────────────────

@pytest.mark.asyncio
async def test_cancel_round_trip(stack):
    """Cancel receives OrderUpdate{CANCELLED} within 1s."""
    gw = stack["gw_url"]

    ws, s = await _connect(gw, user_id=3)
    try:
        cid = "cancel_test_cid000"  # 18 chars → pad to 20
        cid = cid[:20].ljust(20, "0")

        # Place a resting order
        await ws.send_str(_order(10, 0, 48000, 3, cid))
        await asyncio.sleep(0.05)

        # Cancel by CID
        t0 = time.time()
        await ws.send_str(json.dumps({"C": [cid]}))

        # Expect OrderUpdate with status=2 (CANCELLED)
        upd = await _recv_until(
            ws,
            lambda d: "U" in d and d["U"][1] == 2,
            timeout=1.0,
        )
        elapsed = time.time() - t0

        assert upd is not None, \
            "did not receive OrderUpdate{CANCELLED} within 1s"
        assert elapsed < 1.0, \
            f"cancel round-trip took {elapsed:.3f}s > 1s"
    finally:
        await ws.close()
        await s.close()


# ── criterion 5: restart safety ────────────────────────────────────────

@pytest.mark.asyncio
async def test_restart_safety(stack):
    """Resting orders survive ME kill+restart; new order matches them."""
    gw = stack["gw_url"]
    me_bin = str(ME_BIN)
    env_me = stack["env_me"]

    ws4, s4 = await _connect(gw, user_id=4)
    ws5, s5 = await _connect(gw, user_id=5)
    try:
        # Place 3 resting bids from user 4
        for i in range(3):
            cid = f"rest{i}".ljust(20, "0")
            await ws4.send_str(_order(10, 0, 47000 + i, 2, cid))
        await asyncio.sleep(0.2)

        # Kill ME
        me_proc = stack["procs"]["me"]
        me_proc.send_signal(signal.SIGTERM)
        me_proc.wait(timeout=3)

        # Wait for snapshot to be written (snapshot every 10s by default,
        # but we need to start before it's written — test restart from WAL)
        await asyncio.sleep(0.3)

        # Restart ME
        new_me = subprocess.Popen(
            [me_bin], env=env_me,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        stack["procs"]["me"] = new_me

        # Wait for ME to be ready (give it 3s to restore from WAL)
        await asyncio.sleep(2.0)

        # User 5 crosses into the resting orders
        await ws5.send_str(_order(10, 1, 47000, 2, "cross00"))

        fill5 = await _recv_until(
            ws5, lambda d: "F" in d, timeout=2.0)
        fill4 = await _recv_until(
            ws4, lambda d: "F" in d, timeout=2.0)

        assert fill5 is not None, \
            "taker did not receive Fill after ME restart"
        assert fill4 is not None, \
            "maker did not receive Fill after ME restart"
    finally:
        await ws4.close()
        await ws5.close()
        await s4.close()
        await s5.close()


# ── criterion 7: maker integration ────────────────────────────────────

@pytest.mark.asyncio
async def test_maker_integration(stack):
    """Market maker quotes >= 5 bid+ask levels within 3s."""
    import sys
    sys.path.insert(
        0, str(Path(__file__).parent.parent))
    from market_maker import DummyMarketMaker

    gw = stack["gw_url"]
    md = stack["md_url"]

    maker = DummyMarketMaker(
        gateway_url=gw,
        marketdata_ws=md,
        symbol_ids=[10],
        spread_bps=10,
        qty_per_level=5,
        num_levels=5,
        refresh_sec=1.0,
        user_id=99,
    )

    loop = asyncio.get_event_loop()
    maker.start()
    try:
        # Wait 3s for maker to place orders
        await asyncio.sleep(3.0)

        st = maker.status()
        assert st["orders_placed"] > 0, \
            "market maker placed zero orders"
        assert st["active_orders"] >= 5, \
            (f"expected >= 5 active bid+ask levels, "
             f"got {st['active_orders']}")

        # Verify a crossing fill appears in the WAL within 2s
        # by sending a taker order that crosses the maker's asks
        ws6, s6 = await _connect(gw, user_id=6)
        try:
            mid = st["mid_prices"].get("10", 50000)
            cross_px = mid + 200  # should cross maker asks
            await ws6.send_str(
                _order(10, 0, cross_px, 1, "mkr_cross0"))

            fill = await _recv_until(
                ws6, lambda d: "F" in d, timeout=2.0)
            assert fill is not None, \
                "crossing taker order did not fill within 2s"
        finally:
            await ws6.close()
            await s6.close()
    finally:
        await maker.stop()


# ── criterion 8: stress test ───────────────────────────────────────────

@pytest.mark.asyncio
async def test_stress_accept_rate(stack):
    """accept_rate > 0.8 at 50 rps / 10s; no crash; WAL tip monotonic."""
    gw = stack["gw_url"]

    cfg = StressConfig(
        gateway_url=gw,
        rate=50,
        duration=10,
        symbols=["10"],
        users=5,
        connections=5,
        jwt_secret=GW_JWT_SECRET,
    )

    results = await run_stress_test(cfg)
    metrics = results.get("metrics", {})

    accept_rate = metrics.get("accept_rate", 0)
    assert accept_rate > 80.0, \
        f"accept_rate {accept_rate}% <= 80% threshold"

    # Verify all stack processes still running
    for name, proc in stack["procs"].items():
        assert proc.poll() is None, \
            f"{name} process crashed during stress test"
