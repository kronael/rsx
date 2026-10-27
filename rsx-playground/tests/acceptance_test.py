"""Acceptance tests for PROJECT.md criteria 3-5.

Criteria:
  3  Fill round-trip: the taker's order fills <= 1s, the maker's fill is
     in the ME WAL
  4  Cancel round-trip: OrderUpdate{CANCELLED} <= 1s
  5  Restart safety: resting orders survive ME kill+restart

All tests start a full RSX stack (gateway + risk + ME + marketdata).
Postgres at DATABASE_URL is required (default: rsx:folium@10.0.2.1:5432).
"""

import asyncio
import os
import shutil
import signal
import socket
import subprocess
import sys
import time
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))

import order_cli
import rsx_cli

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


def _seed_accounts(user_ids: list[int], collateral: int = 10**17):
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
        "RSX_GW_DATABASE_URL": PG_URL,
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

        with pytest.MonkeyPatch.context() as patch:
            patch.setenv("RSX_GW_JWT_SECRET", GW_JWT_SECRET)
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

SYMBOL_ID = 10
# Three lots of symbol 10's Postgres config: lot-aligned, above its minimum
# notional at the prices below.
QTY = 300_000


def _gateway(stack) -> order_cli.Gateway:
    return order_cli.Gateway(stack["gw_url"])


def _new_order(user_id: int, side: str, px: int, qty: int,
               cid: str) -> order_cli.NewOrder:
    return order_cli.NewOrder(
        user_id=user_id, symbol_id=SYMBOL_ID, side=side,
        px=px, qty_raw=qty, cid=cid)


async def _fills_of(wal_me: str, maker_user_id: int,
                    timeout: float = 2.0) -> list[dict]:
    """The ME WAL's FILL records with this maker, once any is written."""
    deadline = time.time() + timeout
    while True:
        records = await rsx_cli.run_lines(
            "wal", "dump", str(SYMBOL_ID), wal_me, "--type", "FILL", "--json")
        fills = [r for r in records if r["maker_user_id"] == maker_user_id]
        if fills or time.time() > deadline:
            return fills
        await asyncio.sleep(0.05)


# ── criterion 3: WS fill round-trip ───────────────────────────────────

@pytest.mark.asyncio
async def test_ws_new_order_fill_update_complete(stack):
    """The taker's order fills within 1s and the ME WAL holds the
    maker's fill."""
    gateway = _gateway(stack)

    resting = await gateway.place(_new_order(2, "buy", 49900, QTY, "bid00"))
    assert resting.status == "resting", resting.reason

    t0 = time.time()
    taker = await gateway.place(_new_order(1, "sell", 49900, QTY, "ask00"))
    elapsed = time.time() - t0

    assert taker.status == "filled", \
        f"user1 (taker) did not fill: {taker.status} {taker.reason}"
    assert [fill["qty"] for fill in taker.fills] == [QTY]
    assert elapsed < 1.0, \
        f"fill round-trip took {elapsed:.3f}s > 1s"
    assert await _fills_of(stack["wal_me"], maker_user_id=2), \
        "user2 (maker) has no FILL in the ME WAL"


# ── criterion 4: cancel round-trip ────────────────────────────────────

@pytest.mark.asyncio
async def test_cancel_round_trip(stack):
    """Cancel receives OrderUpdate{CANCELLED} within 1s."""
    gateway = _gateway(stack)

    resting = await gateway.place(_new_order(3, "buy", 48000, QTY, "cancel_test"))
    assert resting.status == "resting", resting.reason

    t0 = time.time()
    cancelled = await gateway.cancel(3, "cancel_test")
    elapsed = time.time() - t0

    assert cancelled.status == "cancelled", \
        f"cancel was not confirmed: {cancelled.status} {cancelled.reason}"
    assert elapsed < 1.0, \
        f"cancel round-trip took {elapsed:.3f}s > 1s"


# ── criterion 5: restart safety ────────────────────────────────────────

@pytest.mark.asyncio
async def test_restart_safety(stack):
    """Resting orders survive ME kill+restart; new order matches them."""
    gateway = _gateway(stack)
    me_bin = str(ME_BIN)
    env_me = stack["env_me"]

    for i in range(3):
        resting = await gateway.place(
            _new_order(4, "buy", 47000 + i, QTY, f"rest{i}"))
        assert resting.status == "resting", resting.reason
    await asyncio.sleep(0.2)

    me_proc = stack["procs"]["me"]
    me_proc.send_signal(signal.SIGTERM)
    me_proc.wait(timeout=3)

    # Restart from the WAL: the snapshot interval is longer than this test.
    await asyncio.sleep(0.3)
    new_me = subprocess.Popen(
        [me_bin], env=env_me,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    stack["procs"]["me"] = new_me

    await asyncio.sleep(2.0)

    taker = await gateway.place(_new_order(5, "sell", 47000, QTY, "cross00"))
    assert taker.status == "filled", \
        f"taker did not fill after ME restart: {taker.status} {taker.reason}"
    assert await _fills_of(stack["wal_me"], maker_user_id=4), \
        "maker has no FILL in the ME WAL after the restart"
