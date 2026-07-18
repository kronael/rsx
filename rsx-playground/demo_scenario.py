"""Demo scene driver: a dynamic, dramatic market for the terminal demo.

Drives the live trio's featured symbol (PENGU) through a scripted
calm -> wild pump -> partial-retrace price path by writing per-symbol
mid overrides (``mids``) into tmp/maker-config.json, which the Go maker
polls every quote cycle. Everything downstream is real: real maker
orders, real taker fills through GW -> Risk -> ME, a real risk-engine
liquidation, real marketdata. Nothing writes to the tape directly.

Cast (playground-seeded users):
  user 1  the winner — buys before the pump; its rsx-term position
          line ends around +10k unrealized PnL, computed by the
          terminal from its own fills.
  user 2  the victim — opens a leveraged short with deliberately small
          collateral (seeded in Postgres before risk boots, the same
          mechanism seed_accounts uses); the pump pushes equity under
          maintenance margin and the RISK ENGINE liquidates it: a real
          reduce-only IOC through the matching engine.
  user 3  background taker — small IOC prints so the tape breathes.

Stages (``./rsx-playground/playground demo-scene <stage>``):
  prep  boot a FRESH scene cluster: reset, server, shrink the victim's
        collateral, start-all trio, stop the mark process (risk then
        marks PENGU to the local BBO index — single-venue price
        discovery, so the scripted book move is the mark move), restart
        risk-0 while the book is still empty, start the maker with the
        "walls" depth profile, open the victim short. Prints the
        computed liquidation trigger.
  run   the timed path. Timeline (seconds): calm 0-18 (winner entry
        at 8), pump 18-48 (+45%, liquidation fires on the way up),
        retrace 48-70 (to +35%), then a gentle drift until killed.
  clean remove the scene's maker-config keys (mids, depth_profile).

Coordinate a recording against ``run``'s stage markers on stdout.
"""

import asyncio
import json
import math
import os
import random
import signal
import subprocess
import sys
import time
import uuid
from dataclasses import dataclass
from pathlib import Path

import aiohttp
import asyncpg
import jwt as pyjwt

ROOT = Path(__file__).resolve().parent.parent
MAKER_CONFIG = ROOT / "tmp" / "maker-config.json"

GATEWAY_URL = os.environ.get("GATEWAY_URL", "ws://127.0.0.1:8088")
API_BASE = (
    f"http://127.0.0.1:{os.environ.get('RSX_PLAYGROUND_PORT', '49171')}"
)
PG_URL = os.environ.get(
    "DATABASE_URL", "postgres://rsx:rsx@127.0.0.1:5432/rsx")
# runtime.py's dev-only gateway secret; the scene mints user JWTs the
# same way market_maker.py and the stress client do.
JWT_SECRET = os.environ.get(
    "RSX_GW_JWT_SECRET", "rsx-dev-secret-not-for-prod-padpad")

SYMBOL = 10  # PENGU-PERP: tick 1, lot 100000, price_dec 6, qty_dec 4
LOT = 100_000

# PENGU-only cluster. Two reasons, both learned in rehearsal:
# 1. risk has ONE gap-replay endpoint (the first symbol's ME); on the
#    trio a SOL-stream gap asks the PENGU ME, is refused, and risk
#    PANICS (failover.rs:102) mid-scene.
# 2. the cmp/event path wedges after enough events (BUGS.md
#    RISK-CMP-DESYNC-ORDER-EVENT-STARVE); one symbol at 1s refresh
#    keeps the rate ~6x below the trio's, buying the scene its window.
SCENARIO = "minimal"

WINNER_USER = 1
VICTIM_USER = 2
TAKER_USER = 3

# Winner: 1M PENGU long in 4 clips. At +35% off a 0.030 entry the
# terminal's uPnL line reads ~ +10,500 (qty_raw * dpx_raw / 1e10).
WINNER_QTY = 10_000_000_000
WINNER_CLIPS = 4

# Victim: 300k PENGU short. Collateral is sized so equity crosses
# maintenance margin (5%) about VICTIM_TRIGGER into the pump:
#   equity = C + q*(E - mark) < 0.05*q*mark  =>  mark > (C + qE)/(1.05q)
VICTIM_QTY = 3_000_000_000
VICTIM_TRIGGER = 0.08

# Maker profile for the scene: 20k lots/level base, "walls" shape.
# refresh 1s (not the default 500ms): halves the per-symbol ME event
# rate, which delays the known cmp-desync wedge (BUGS.md
# RISK-CMP-DESYNC-ORDER-EVENT-STARVE) — the whole scene must land
# before it starves the event path.
MAKER = {
    "spread_bps": 20, "qty": 20000, "symbol_id": SYMBOL,
    "refresh_ms": 1000, "levels": 5,
}

CALM_S = 12.0
ENTRY_AT = 4.0
PUMP_S = 25.0
PUMP_PCT = 0.45
RETRACE_S = 15.0
RETRACE_TO = 0.35
STEP_S = 0.5


def mark(msg: str) -> None:
    print(f"[scene +{time.monotonic() - T0:6.1f}s] {msg}", flush=True)


T0 = time.monotonic()


# ── playground API ──────────────────────────────────────────


async def api(
    session: aiohttp.ClientSession,
    method: str,
    path: str,
    **kwargs,
):
    headers = {"x-confirm": "yes", **kwargs.pop("headers", {})}
    for attempt in (1, 2):
        try:
            async with session.request(
                method, f"{API_BASE}{path}", headers=headers,
                timeout=aiohttp.ClientTimeout(total=90), **kwargs,
            ) as resp:
                body = await resp.text()
                if resp.status >= 400:
                    raise RuntimeError(
                        f"{method} {path} -> {resp.status}: {body[:200]}")
                try:
                    return json.loads(body)
                except json.JSONDecodeError:
                    return body
        except aiohttp.ServerDisconnectedError:
            # A keep-alive connection idled out during a long subprocess
            # step; retry once on a fresh connection.
            if attempt == 2:
                raise
            await asyncio.sleep(0.2)


async def bbo_mid(session: aiohttp.ClientSession, symbol: int) -> int:
    data = await api(session, "GET", f"/api/bbo/{symbol}")
    if not isinstance(data, dict):
        raise RuntimeError(f"unexpected BBO response for symbol {symbol}: {data}")
    bid = data.get("bid_px", 0)
    ask = data.get("ask_px", 0)
    if bid <= 0 or ask <= 0:
        raise RuntimeError(f"no live BBO for symbol {symbol}: {data}")
    return (bid + ask) // 2


async def wait_live_bbo(
    session: aiohttp.ClientSession, symbol: int, timeout: float = 30,
) -> int:
    deadline = time.monotonic() + timeout
    while True:
        try:
            return await bbo_mid(session, symbol)
        except (RuntimeError, aiohttp.ClientError):
            if time.monotonic() > deadline:
                raise
            await asyncio.sleep(1)


# ── maker config file (the price lever) ─────────────────────


def write_maker_config(cfg: dict) -> None:
    """Replace tmp/maker-config.json wholesale, atomically."""
    MAKER_CONFIG.parent.mkdir(parents=True, exist_ok=True)
    tmp = MAKER_CONFIG.with_suffix(".tmp")
    tmp.write_text(json.dumps(cfg))
    tmp.replace(MAKER_CONFIG)


def write_maker_keys(**keys) -> None:
    """Merge keys into tmp/maker-config.json atomically (None deletes)."""
    try:
        cfg = json.loads(MAKER_CONFIG.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        cfg = {}
    for k, v in keys.items():
        if v is None:
            cfg.pop(k, None)
        else:
            cfg[k] = v
    write_maker_config(cfg)


def set_mid(px: int) -> None:
    write_maker_keys(mids={str(SYMBOL): int(px)})


# ── gateway orders ──────────────────────────────────────────


def mint_jwt(user_id: int) -> str:
    return pyjwt.encode(
        {
            "sub": f"scene:{user_id}",
            "user_id": user_id,
            "aud": "rsx-gateway",
            "iss": "rsx-auth",
            "exp": int(time.time()) + 3600,
            "jti": uuid.uuid4().hex,
        },
        JWT_SECRET,
        algorithm="HS256",
    )


@dataclass
class FillReport:
    filled: int = 0
    notional: int = 0

    def avg_px(self) -> int:
        return self.notional // self.filled if self.filled else 0


async def ioc(
    session: aiohttp.ClientSession,
    user_id: int,
    side: int,
    px: int,
    qty: int,
    drain_s: float = 2.0,
) -> FillReport:
    """Submit one IOC as user_id and fold its fills from the ack stream."""
    headers = {"Authorization": f"Bearer {mint_jwt(user_id)}"}
    cid = f"sc{uuid.uuid4().hex[:14]}".ljust(20, "0")
    report = FillReport()
    async with session.ws_connect(GATEWAY_URL, headers=headers) as ws:
        await ws.send_str(json.dumps({"N": [SYMBOL, side, px, qty, cid, 1]}))
        deadline = time.monotonic() + drain_s
        while time.monotonic() < deadline:
            try:
                msg = await ws.receive(timeout=deadline - time.monotonic())
            except asyncio.TimeoutError:
                break
            if msg.type != aiohttp.WSMsgType.TEXT:
                continue
            frame = json.loads(msg.data)
            if "F" in frame:
                _, _, fpx, fqty = frame["F"][:4]
                report.filled += fqty
                report.notional += fpx * fqty
            elif "U" in frame and frame["U"][1] in (0, 2, 3):
                break
            elif "E" in frame:
                raise RuntimeError(f"user {user_id} order rejected: {frame}")
    return report


# ── prep ────────────────────────────────────────────────────


def _victim_collateral(entry_px: int) -> int:
    # C = q*E*(1.05*(1+trigger) - 1); IM at open is 10% of notional,
    # so this always clears the opening margin gate with headroom.
    factor = 1.05 * (1 + VICTIM_TRIGGER) - 1
    return int(VICTIM_QTY * entry_px * factor)


async def shrink_victim_collateral(entry_px: int) -> int:
    collateral = _victim_collateral(entry_px)
    conn = await asyncpg.connect(PG_URL)
    try:
        await conn.execute(
            "UPDATE accounts SET collateral = $1 WHERE user_id = $2",
            collateral, VICTIM_USER,
        )
        back = await conn.fetchval(
            "SELECT collateral FROM accounts WHERE user_id = $1",
            VICTIM_USER,
        )
        if back != collateral:
            raise RuntimeError(
                f"victim collateral readback {back} != {collateral}")
    finally:
        await conn.close()
    return collateral


def sh(cmd: list[str]) -> None:
    mark(f"$ {' '.join(cmd)}")
    res = subprocess.run(cmd, cwd=ROOT)
    if res.returncode != 0:
        raise RuntimeError(f"command failed ({res.returncode}): {cmd}")


async def wait_api(session: aiohttp.ClientSession, timeout: float = 60) -> None:
    deadline = time.monotonic() + timeout
    while True:
        try:
            await api(session, "GET", "/api/processes")
            return
        except (RuntimeError, aiohttp.ClientError):
            if time.monotonic() > deadline:
                raise
            await asyncio.sleep(1)


async def real_mark_anchor(session: aiohttp.ClientSession) -> int:
    """Best-effort real-world PENGU price from the mark WAL (the mark
    process gets a few seconds of Binance before prep silences it) —
    the scene's honest opening level. Falls back to the maker default."""
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        try:
            data = await api(session, "GET", "/api/mark/prices")
            prices = data.get("prices") if isinstance(data, dict) else None
            px = int((prices or {}).get(str(SYMBOL), {}).get("mark", 0))
            if px > 0:
                return px
        except (RuntimeError, aiohttp.ClientError, ValueError):
            pass
        await asyncio.sleep(1)
    return 30_000


async def prep() -> None:
    """Boot a fresh scene cluster and stage the accounts."""
    # A stale `run` stage from an earlier scene keeps writing mids and
    # taker orders forever (killing the CLI wrapper orphans the uv
    # child) — sweep any survivors before resetting the world.
    subprocess.run(["pkill", "-f", "demo_scenario.py run"], check=False)

    playground = str(ROOT / "rsx-playground" / "playground")
    sh([playground, "reset", "--yes"])
    sh([playground, "start"])

    async with aiohttp.ClientSession() as session:
        await wait_api(session)
        sh([playground, "start-all", SCENARIO])

        # Let the mark process pull a few seconds of real Binance PENGU
        # so the scene opens at the real-world price.
        anchor = await real_mark_anchor(session)
        mark(f"opening anchor {anchor} (real mark when available)")

        # Victim collateral BEFORE the risk restart below: risk
        # cold-loads accounts from Postgres, so the shrink lands then.
        collateral = await shrink_victim_collateral(anchor)
        mark(f"victim user {VICTIM_USER} collateral -> {collateral}")

        # Silence the mark feed, then restart risk while the books are
        # still empty: the fresh risk process sees mark[PENGU]==0 forever
        # and marks against the local BBO index — the scripted book move
        # becomes the margin price. Restarting risk this early means no
        # maker WAL state (reservations) exists to replay.
        await api(session, "POST", "/api/processes/mark/stop")
        mark("mark stopped (risk will mark PENGU to the local BBO index)")
        await api(session, "POST", "/api/processes/risk-0/restart")
        mark("risk-0 restarted clean")

        # Scene maker profile: bigger size + walls gradient, anchored at
        # the real-world level. Clean-slate write — a stale mids from an
        # earlier scene must not leak in.
        write_maker_config(
            {**MAKER, "depth_profile": "walls",
             "mids": {str(SYMBOL): anchor}})
        await api(session, "POST", "/api/maker/start")
        mid0 = await wait_live_bbo(session, SYMBOL)
        mark(f"books live, PENGU mid {mid0}")

        # Victim short: real IOC crossing the maker's bids.
        rep = await ioc(
            session, VICTIM_USER, side=1,
            px=int(mid0 * 0.98), qty=VICTIM_QTY,
        )
        if rep.filled < VICTIM_QTY:
            raise RuntimeError(
                f"victim short only filled {rep.filled}/{VICTIM_QTY}")
        avg_px = rep.avg_px()
        trigger = (collateral + VICTIM_QTY * avg_px) // int(1.05 * VICTIM_QTY)
        mark(
            f"victim SHORT {rep.filled} @ ~{avg_px} "
            f"(liquidates when index > ~{trigger}, "
            f"+{trigger / avg_px - 1:.1%})"
        )
        print("PREP OK")


# ── run ─────────────────────────────────────────────────────


def path_px(mid0: int, t: float) -> int:
    """The scripted mid at scene-time t: calm, pump, retrace, drift."""
    if t < CALM_S:
        pct = 0.002 * math.sin(t / 2.5)
    elif t < CALM_S + PUMP_S:
        u = (t - CALM_S) / PUMP_S
        pct = PUMP_PCT * u**1.6
    elif t < CALM_S + PUMP_S + RETRACE_S:
        u = (t - CALM_S - PUMP_S) / RETRACE_S
        pct = PUMP_PCT - (PUMP_PCT - RETRACE_TO) * u
    else:
        drift_t = t - CALM_S - PUMP_S - RETRACE_S
        pct = RETRACE_TO + 0.004 * math.sin(drift_t / 4)
    jitter = random.uniform(-0.0012, 0.0012)
    return max(1, int(mid0 * (1 + pct + jitter)))


async def winner_entry(session: aiohttp.ClientSession, mid0: int) -> None:
    total = FillReport()
    clip = WINNER_QTY // WINNER_CLIPS
    for i in range(WINNER_CLIPS):
        rep = await ioc(
            session, WINNER_USER, side=0,
            px=int(mid0 * 1.02), qty=clip,
        )
        total.filled += rep.filled
        total.notional += rep.notional
        mark(
            f"winner clip {i + 1}/{WINNER_CLIPS}: "
            f"+{rep.filled} @ ~{rep.avg_px()}")
        await asyncio.sleep(1.2)
    mark(f"winner LONG {total.filled} @ ~{total.avg_px()}")


async def taker_noise(
    session: aiohttp.ClientSession, mid0: int, run_t0: float,
) -> None:
    """Small user-3 IOC prints; buys lean with the pump."""
    while True:
        await asyncio.sleep(random.uniform(1.5, 3.5))
        t = time.monotonic() - run_t0
        pumping = CALM_S <= t < CALM_S + PUMP_S
        side = 0 if (pumping or random.random() < 0.5) else 1
        qty = random.randint(2, 10) * LOT * random.choice((1, 1, 5))
        px = path_px(mid0, t)
        px = int(px * 1.01) if side == 0 else int(px * 0.99)
        try:
            await ioc(session, TAKER_USER, side, px, qty, drain_s=1.0)
        except (RuntimeError, aiohttp.ClientError, asyncio.TimeoutError):
            pass  # background flavor only — never kill the scene


def liquidation_lines() -> list[str]:
    """Lines where the risk log states its liquidation ("liquidation
    order sent: user=..."). The authoritative evidence — the PG
    liquidations table the dashboard endpoint reads is not written on
    this path."""
    try:
        text = (ROOT / "log" / "risk-0.log").read_text()
    except OSError:
        return []
    return [ln for ln in text.splitlines() if "liquidation order sent" in ln]


async def liquidation_watch() -> None:
    """Announce the risk engine's liquidation the moment it lands."""
    while True:
        await asyncio.sleep(2)
        for line in liquidation_lines():
            mark(f"LIQUIDATION confirmed (risk log): {line.split('  ')[-1]}")
            return


async def run() -> None:
    async with aiohttp.ClientSession() as session:
        mid0 = await wait_live_bbo(session, SYMBOL)
        mark(f"run: mid0 {mid0}; calm {CALM_S:.0f}s, "
             f"pump +{PUMP_PCT:.0%} over {PUMP_S:.0f}s, "
             f"retrace to +{RETRACE_TO:.0%}")
        run_t0 = time.monotonic()
        noise = asyncio.create_task(taker_noise(session, mid0, run_t0))
        watch = asyncio.create_task(liquidation_watch())
        entry: asyncio.Task | None = None
        announced = set()
        try:
            while True:
                t = time.monotonic() - run_t0
                set_mid(path_px(mid0, t))
                if t >= ENTRY_AT and entry is None:
                    entry = asyncio.create_task(winner_entry(session, mid0))
                for at, what in (
                    (CALM_S, "PUMP begins"),
                    (CALM_S + PUMP_S, "retrace begins"),
                    (CALM_S + PUMP_S + RETRACE_S, "drift (hold for the shot)"),
                ):
                    if t >= at and what not in announced:
                        announced.add(what)
                        mark(what)
                await asyncio.sleep(STEP_S)
        finally:
            noise.cancel()
            watch.cancel()
            if entry is not None:
                entry.cancel()


async def report() -> None:
    """Print the liquidation evidence + both positions (verification)."""
    for line in liquidation_lines():
        print(f"  risk: {line.split('  ')[-1]}")
    async with aiohttp.ClientSession() as session:
        for user in (VICTIM_USER, WINNER_USER):
            pos = await api(session, "GET", f"/api/risk/users/{user}")
            print(f"  user {user} positions: {pos}")


def clean() -> None:
    write_maker_keys(mids=None, depth_profile=None)
    mark("scene maker-config keys removed")


def main() -> int:
    stage = sys.argv[1] if len(sys.argv) > 1 else "run"
    if stage == "prep":
        asyncio.run(prep())
    elif stage == "run":
        loop = asyncio.new_event_loop()
        main_task = loop.create_task(run())
        for sig in (signal.SIGINT, signal.SIGTERM):
            loop.add_signal_handler(sig, main_task.cancel)
        try:
            loop.run_until_complete(main_task)
        except asyncio.CancelledError:
            mark("run stopped")
        finally:
            loop.close()
    elif stage == "report":
        asyncio.run(report())
    elif stage == "clean":
        clean()
    else:
        print(__doc__)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
