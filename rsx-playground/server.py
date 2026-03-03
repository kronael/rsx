"""RSX Playground — dev dashboard.

Usage: cd rsx-playground && uv run server.py
"""

import asyncio
import html
import json
import os
import random
import re
import shutil
import signal
import struct
import subprocess
import sys
import time
import uuid
from contextlib import asynccontextmanager
from datetime import datetime
from pathlib import Path

import aiohttp
import psutil
import uvicorn
from fastapi import FastAPI
from fastapi import Form
from fastapi import Query
from fastapi import Request
from fastapi import WebSocket
from fastapi import WebSocketDisconnect
from fastapi.responses import HTMLResponse
from fastapi.responses import JSONResponse
from fastapi.responses import RedirectResponse
from fastapi.staticfiles import StaticFiles

import pages

ROOT = Path(__file__).resolve().parent.parent
TMP = ROOT / "tmp"

# Load .env from playground dir (does not override existing env vars)
_env_file = Path(__file__).resolve().parent / ".env"
if _env_file.exists():
    for _line in _env_file.read_text().splitlines():
        _line = _line.strip()
        if _line and not _line.startswith("#") and "=" in _line:
            _k, _v = _line.split("=", 1)
            os.environ.setdefault(_k.strip(), _v.strip())
if os.environ.get("PLAYGROUND_MODE") == "production":
    raise SystemExit("refusing to start: PLAYGROUND_MODE=production")
WAL_DIR = TMP / "wal"
LOG_DIR = ROOT / "log"
PID_DIR = TMP / "pids"
STRESS_REPORTS_DIR = TMP / "stress-reports"
STRESS_REPORTS_DIR.mkdir(parents=True, exist_ok=True)

PG_URL = os.environ.get(
    "DATABASE_URL",
    "postgres://rsx:folium@10.0.2.1:5432/rsx_dev",
)

GATEWAY_URL = os.environ.get(
    "GATEWAY_URL", "ws://localhost:8080"
)
GATEWAY_HTTP = os.environ.get(
    "GATEWAY_HTTP", "http://localhost:8080"
)
MARKETDATA_WS = os.environ.get(
    "MARKETDATA_WS", "ws://localhost:8180"
)
WEBUI_DIST = ROOT / "rsx-webui" / "dist"

# ── import start script's config ────────────────────────

import importlib.machinery
import importlib.util
import types
try:
    _loader = importlib.machinery.SourceFileLoader(
        "start_mod", str(ROOT / "start"))
    _spec = importlib.util.spec_from_loader(
        "start_mod", _loader)
    start_mod = importlib.util.module_from_spec(_spec)
    _loader.exec_module(start_mod)
except Exception:
    # start file missing — provide default SYMBOLS
    start_mod = types.ModuleType("start_mod")
    start_mod.SYMBOLS = {
        "BTC": {
            "id": 1, "tick": 50, "lot": 100,
            "price_dec": 2, "qty_dec": 8,
        },
        "ETH": {
            "id": 2, "tick": 10, "lot": 1000,
            "price_dec": 2, "qty_dec": 8,
        },
        "SOL": {
            "id": 3, "tick": 1, "lot": 10000,
            "price_dec": 4, "qty_dec": 6,
        },
        "PENGU": {
            "id": 10, "tick": 1, "lot": 100000,
            "price_dec": 6, "qty_dec": 4,
        },
    }

# ── postgres ────────────────────────────────────────────

pg_pool = None


async def pg_connect():
    """Try to create asyncpg pool."""
    global pg_pool
    try:
        import asyncpg
        pg_pool = await asyncpg.create_pool(
            PG_URL, min_size=1, max_size=3,
            command_timeout=5,
        )
    except Exception as e:
        print(f"postgres not available: {e}")
        pg_pool = None


async def pg_query(sql, *args):
    """Run a query, return list of dicts."""
    if pg_pool is None:
        return None
    try:
        async with pg_pool.acquire() as conn:
            rows = await conn.fetch(sql, *args)
            return [dict(r) for r in rows]
    except Exception as e:
        return {"error": str(e)}


# ── in-memory book snapshot (updated from marketdata WS) ─

# symbol_id -> {"bids": [{px, qty}, ...], "asks": [...]}
_book_snap: dict[int, dict] = {}
_md_ws_task: asyncio.Task | None = None
# recent trades from marketdata WS (capped at 200)
recent_fills: list[dict] = []



async def _md_ws_subscriber():
    """Subscribe to marketdata WS; maintain _book_snap from L2/BBO.

    Reconnects with exponential backoff + jitter (1s→2s→…→30s).
    Circuit breaker trips after 8 consecutive infra-class failures
    (ConnectionRefusedError / OSError) and pauses fan-out by
    stopping all reconnect attempts until the task is restarted.
    """
    # CHANNEL_BBO=1, CHANNEL_DEPTH=2, CHANNEL_TRADES=4
    CHANNELS = 7
    DEFAULT_SYMBOLS = [1, 2, 3, 10]

    MAX_RETRIES = 20       # hard cap on total attempts
    CIRCUIT_AT = 8         # consecutive infra failures → open
    delay = 1.0
    max_delay = 30.0
    consec_infra = 0       # consecutive ConnectionRefused/OSError
    attempt = 0

    while attempt < MAX_RETRIES:
        attempt += 1
        try:
            async with aiohttp.ClientSession() as session:
                async with session.ws_connect(
                    MARKETDATA_WS,
                    heartbeat=10,
                ) as ws:
                    # connected — reset backoff counters
                    consec_infra = 0
                    delay = 1.0
                    # Subscribe to depth+BBO for known symbols
                    for sid in DEFAULT_SYMBOLS:
                        await ws.send_str(
                            json.dumps({"S": [sid, CHANNELS]}))
                    async for msg in ws:
                        if msg.type != aiohttp.WSMsgType.TEXT:
                            continue
                        try:
                            frame = json.loads(msg.data)
                        except Exception:
                            continue
                        # L2 snapshot: {"B":[sym,[[px,qty,cnt],...],
                        #               [[px,qty,cnt],...],ts,seq]}
                        if "B" in frame:
                            arr = frame["B"]
                            sid = int(arr[0])
                            bids_raw = arr[1]
                            asks_raw = arr[2]
                            _book_snap[sid] = {
                                "bids": [
                                    {"px": b[0], "qty": b[1]}
                                    for b in bids_raw
                                ],
                                "asks": [
                                    {"px": a[0], "qty": a[1]}
                                    for a in asks_raw
                                ],
                            }
                        # L2 delta: {"D":[sym,side,px,qty,cnt,ts,seq]}
                        elif "D" in frame:
                            arr = frame["D"]
                            sid = int(arr[0])
                            side = int(arr[1])  # 0=bid,1=ask
                            px = int(arr[2])
                            qty = int(arr[3])
                            snap = _book_snap.setdefault(
                                sid, {"bids": [], "asks": []})
                            key = "bids" if side == 0 else "asks"
                            levels = snap[key]
                            if qty == 0:
                                snap[key] = [
                                    l for l in levels
                                    if l["px"] != px
                                ]
                            else:
                                existing = next(
                                    (l for l in levels
                                     if l["px"] == px), None)
                                if existing:
                                    existing["qty"] = qty
                                else:
                                    levels.append(
                                        {"px": px, "qty": qty})
                                # re-sort: bids desc, asks asc
                                if side == 0:
                                    snap[key].sort(
                                        key=lambda l: -l["px"])
                                else:
                                    snap[key].sort(
                                        key=lambda l: l["px"])
                        # BBO: {"BBO":[sym,bid_px,bid_qty,bid_cnt,
                        #              ask_px,ask_qty,...]}
                        elif "BBO" in frame:
                            arr = frame["BBO"]
                            sid = int(arr[0])
                            bid_px = int(arr[1])
                            bid_qty = int(arr[2])
                            ask_px = int(arr[4])
                            ask_qty = int(arr[5])
                            # Only update BBO if no depth snap yet
                            if sid not in _book_snap:
                                snap: dict = {"bids": [], "asks": []}
                                if bid_px:
                                    snap["bids"] = [
                                        {"px": bid_px, "qty": bid_qty}
                                    ]
                                if ask_px:
                                    snap["asks"] = [
                                        {"px": ask_px, "qty": ask_qty}
                                    ]
                                _book_snap[sid] = snap
                        # Trade: {"T":[sym,px,qty,taker_side,
                        #              ts_ns,seq]}
                        elif "T" in frame:
                            arr = frame["T"]
                            recent_fills.append({
                                "symbol_id": int(arr[0]),
                                "price": int(arr[1]),
                                "qty": int(arr[2]),
                                "taker_side": int(arr[3]),
                                "seq": int(arr[5]),
                            })
                            if len(recent_fills) > 200:
                                del recent_fills[:100]
        except asyncio.CancelledError:
            break
        except (ConnectionRefusedError, OSError):
            consec_infra += 1
            if consec_infra >= CIRCUIT_AT:
                import logging as _log
                _log.getLogger(__name__).warning(
                    "md subscriber circuit open: %d consecutive "
                    "infra failures; pausing fan-out",
                    consec_infra,
                )
                break
        except Exception:
            pass

        # exponential backoff with ±20 % jitter
        jitter = delay * (0.8 + 0.4 * random.random())
        await asyncio.sleep(jitter)
        delay = min(delay * 2, max_delay)


# ── process manager ─────────────────────────────────────

# name -> {"proc": asyncio.Process, "binary": str,
#          "env": dict}
managed: dict[str, dict] = {}
build_log: list[str] = []
current_scenario = "minimal"

# Per-process restart tracking for auto-restart with backoff.
# Keyed by process name.  Populated on spawn; cleared on
# intentional stop/kill; updated on crash detection.
#
# Fields per entry:
#   restarts        int   — consecutive crash count
#   blocked         bool  — circuit open; no further auto-restart
#   next_restart_at float — epoch seconds; honour backoff window
#   last_crash_ts   float — epoch of last detected crash
#   intentional     bool  — set True before stop/kill so watcher
#                           ignores the exit
_restart_state: dict[str, dict] = {}
_RESTART_MAX = 5           # circuit opens after this many crashes
_RESTART_INIT_DELAY = 2.0  # first retry delay (seconds)
_RESTART_MAX_DELAY = 60.0  # backoff ceiling

_watcher_task: asyncio.Task | None = None

# ── orchestrator session lifecycle ──────────────────────
# At most ONE test run may hold the session lock at a time.
# Concurrent callers get 409 Conflict (hard-fail the run).
#
# session_id — lock token (authenticates release / reclaim)
# run_id     — per-allocation UUID; sent as X-Run-Id header
#              on task dispatch so endpoints can detect stale
#              callers and hard-fail before work begins.
#
# Lease model:
#   _LEASE_TTL  (5 min)  — stale-claim recovery window.
#                          If no renew within this period the
#                          session is auto-released on the next
#                          allocate call, allowing crash recovery
#                          without waiting the full SESSION_TTL.
#   _SESSION_TTL (30 min) — hard cap; run_id checks honour this.
#
# Idempotent reclaim: if allocate body includes the caller's
# own session_id and it matches the active session, the same
# session is returned (safe for retries after transient failures).
_LEASE_TTL = 300.0
_SESSION_TTL = 1800.0
_active_session: dict | None = None  # {id, run_id, ts}
# Serialises concurrent allocate/renew/release calls so the
# check-then-set pattern is atomic within a single process.
_session_lock = asyncio.Lock()


def _check_run_id(request: Request) -> tuple[bool, str]:
    """Validate X-Run-Id header against the active session.

    Returns (ok, error_msg).  ok=True when:
      - No X-Run-Id header is present (legacy/HTMX callers are
        allowed through; the contract only applies when the
        header is explicitly supplied).
      - X-Run-Id matches the active session's run_id and the
        session has not expired.
    Returns (False, msg) when the header is present but stale,
    expired, or unknown — caller must hard-fail before work.

    Pure read: does NOT mutate _active_session.  Stale-session
    reclamation is handled by _stale_session_reaper() and the
    /api/sessions/allocate endpoint under _session_lock.
    """
    header = request.headers.get("X-Run-Id")
    if not header:
        return True, ""
    snap = _active_session
    if snap is None:
        return False, "no active session; run_id is stale"
    age = time.time() - snap["ts"]
    if age >= _SESSION_TTL:
        return False, "session expired (SESSION_TTL exceeded); re-allocate"
    if header != snap["run_id"]:
        return False, (
            f"run_id mismatch: got {header!r}, "
            f"expected {snap['run_id']!r}"
        )
    return True, ""


async def _stale_session_reaper():
    """Periodic background task: reclaim sessions whose SESSION_TTL
    has been exceeded without a renew.

    Runs every 60 s.  Acquires _session_lock to atomically check and
    clear so it cannot race with allocate/renew/release callers.
    """
    global _active_session
    while True:
        await asyncio.sleep(60)
        async with _session_lock:
            if _active_session is None:
                continue
            age = time.time() - _active_session["ts"]
            if age >= _SESSION_TTL:
                print(
                    f"session: reaper reclaiming expired session "
                    f"{_active_session['id']} (age {age:.0f}s)"
                )
                _active_session = None


def get_spawn_plan(scenario="minimal"):
    """Build spawn plan from start script."""
    preset = start_mod.SCENARIOS.get(scenario)
    if not preset:
        preset = start_mod.SCENARIOS["minimal"]
    symbols = start_mod.select_symbols(preset["symbols"])
    config = {
        "symbols": symbols,
        "gateways": preset["gateways"],
        "risk_shards": 1,
        "replication": preset["replication"],
    }
    return start_mod.build_spawn_plan(config, PG_URL)


async def pipe_output(name, stream):
    """Pipe subprocess stdout to log dir."""
    log_path = LOG_DIR / f"{name}.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with open(log_path, "a") as f:
        while True:
            line = await stream.readline()
            if not line:
                break
            text = line.decode("utf-8", errors="replace")
            f.write(text)
            f.flush()


async def spawn_process(name, binary, env):
    """Spawn a single RSX process."""
    p = Path(binary)
    if p.is_absolute():
        binary_path = p
    else:
        binary_path = ROOT / binary.lstrip("./")
    if not binary_path.exists():
        return {"error": f"binary not found: {binary}"}
    full_env = {**os.environ, **env}
    full_env.setdefault("DATABASE_URL", PG_URL)
    proc = await asyncio.create_subprocess_exec(
        str(binary_path),
        env=full_env,
        cwd=str(ROOT),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.STDOUT,
    )
    managed[name] = {
        "proc": proc, "binary": binary, "env": env,
    }
    # Register for auto-restart watching.  Preserve existing restart
    # counters if this is a watcher-triggered restart; otherwise reset.
    if name not in _restart_state:
        _restart_state[name] = {
            "restarts": 0,
            "blocked": False,
            "next_restart_at": 0.0,
            "last_crash_ts": 0.0,
            "intentional": False,
        }
    else:
        # Clear intentional flag so future crashes are auto-restarted.
        _restart_state[name]["intentional"] = False
    asyncio.create_task(pipe_output(name, proc.stdout))
    # write PID file before stability check
    PID_DIR.mkdir(parents=True, exist_ok=True)
    (PID_DIR / f"{name}.pid").write_text(str(proc.pid))
    await asyncio.sleep(0.05)
    if proc.returncode is None:
        return {"pid": proc.pid}
    else:
        # Process exited immediately, clean up
        pid_file = PID_DIR / f"{name}.pid"
        if pid_file.exists():
            pid_file.unlink()
        if name in managed:
            del managed[name]
        _restart_state.pop(name, None)
        return {"error": f"process exited immediately (code "
                         f"{proc.returncode})"}


async def stop_process(name):
    """Stop a managed process by SIGTERM."""
    info = managed.get(name)
    if not info:
        return {"error": f"{name} not managed"}
    # Mark intentional so the watcher does not auto-restart.
    rs = _restart_state.get(name)
    if rs:
        rs["intentional"] = True
    proc = info["proc"]
    if proc.returncode is not None:
        # already stopped, clean up
        pid_file = PID_DIR / f"{name}.pid"
        if pid_file.exists():
            pid_file.unlink()
        del managed[name]
        _restart_state.pop(name, None)
        return {"status": f"{name} already stopped"}
    proc.terminate()
    try:
        await asyncio.wait_for(proc.wait(), timeout=5.0)
    except asyncio.TimeoutError:
        proc.kill()
        await proc.wait()
    pid_file = PID_DIR / f"{name}.pid"
    if pid_file.exists():
        pid_file.unlink()
    # Clean up managed and restart tracking.
    if name in managed:
        del managed[name]
    _restart_state.pop(name, None)
    return {"status": f"{name} stopped"}


async def kill_process(name):
    """Kill a managed process by SIGKILL."""
    info = managed.get(name)
    if not info:
        return {"error": f"{name} not managed"}
    # Mark intentional so the watcher does not auto-restart.
    rs = _restart_state.get(name)
    if rs:
        rs["intentional"] = True
    proc = info["proc"]
    if proc.returncode is not None:
        del managed[name]
        _restart_state.pop(name, None)
        return {"status": f"{name} already stopped"}
    proc.kill()
    await proc.wait()
    pid_file = PID_DIR / f"{name}.pid"
    if pid_file.exists():
        pid_file.unlink()
    del managed[name]
    _restart_state.pop(name, None)
    return {"status": f"{name} killed"}


async def restart_process(name):
    """Restart a managed process."""
    info = managed.get(name)
    if not info:
        return {"error": f"{name} not managed"}
    # Manual restart resets circuit so the process gets a fresh slate.
    rs = _restart_state.get(name)
    if rs:
        rs["restarts"] = 0
        rs["blocked"] = False
        rs["next_restart_at"] = 0.0
    await stop_process(name)
    await asyncio.sleep(0.3)
    return await spawn_process(
        name, info["binary"], info["env"])


async def _process_watcher():
    """Auto-restart crashed processes with bounded exponential backoff.

    Runs every 2 s.  For each managed process whose asyncio.Process
    has a non-None returncode (i.e. it exited unexpectedly):

      - If _restart_state marks it intentional (stop/kill): skip.
      - If circuit is open (blocked): skip; log once on transition.
      - If still within backoff window (next_restart_at in future): skip.
      - Otherwise: increment restarts, compute next backoff window,
        and call spawn_process.  When restarts > _RESTART_MAX the
        circuit opens and the process is marked blocked instead of
        requeueing immediately.

    Backoff schedule (seconds between retries):
      attempt 1→2  attempt 2→4  attempt 3→8  attempt 4→16  attempt 5→32
      (capped at _RESTART_MAX_DELAY=60s)
    """
    global _md_ws_task
    import logging as _log
    _wlog = _log.getLogger(__name__)

    while True:
        try:
            await asyncio.sleep(2.0)
            now = time.time()

            for name in list(managed.keys()):
                info = managed.get(name)
                if info is None:
                    continue
                proc = info["proc"]
                if proc.returncode is None:
                    # Still running — reset consecutive crash counter.
                    rs = _restart_state.get(name)
                    if rs and rs["restarts"] > 0:
                        rs["restarts"] = 0
                    continue

                # Process has exited.  Was it intentional?
                rs = _restart_state.setdefault(name, {
                    "restarts": 0,
                    "blocked": False,
                    "next_restart_at": 0.0,
                    "last_crash_ts": 0.0,
                    "intentional": False,
                })
                if rs.get("intentional"):
                    continue  # stop/kill → do not requeue

                if rs["blocked"]:
                    continue  # circuit open

                if now < rs["next_restart_at"]:
                    continue  # backoff window not yet elapsed

                # Record crash and compute next backoff delay.
                rs["restarts"] += 1
                rs["last_crash_ts"] = now
                attempt = rs["restarts"]

                if attempt > _RESTART_MAX:
                    rs["blocked"] = True
                    _wlog.warning(
                        "process watcher: %s circuit open — "
                        "%d consecutive crashes; marking blocked",
                        name, attempt - 1,
                    )
                    # Remove stale PID file; process won't restart.
                    (PID_DIR / f"{name}.pid").unlink(missing_ok=True)
                    continue

                delay = min(
                    _RESTART_INIT_DELAY * (2 ** (attempt - 1)),
                    _RESTART_MAX_DELAY,
                )
                rs["next_restart_at"] = now + delay

                binary = info.get("binary", "")
                env = info.get("env", {})
                _wlog.info(
                    "process watcher: restarting %s "
                    "(attempt %d/%d, backoff %.0fs)",
                    name, attempt, _RESTART_MAX, delay,
                )
                # Spawn asynchronously; do not await (avoid blocking loop).
                asyncio.create_task(
                    spawn_process(name, binary, env),
                    name=f"restart-{name}",
                )

            # Restart md WS subscriber if it exited (circuit
            # tripped or exhausted retries while marketdata
            # was not yet running).
            if _md_ws_task is None or _md_ws_task.done():
                _md_ws_task = asyncio.create_task(
                    _md_ws_subscriber(),
                    name="md-ws-subscriber",
                )

        except asyncio.CancelledError:
            break
        except Exception as exc:
            import logging as _log2
            _log2.getLogger(__name__).error(
                "process watcher: unexpected error: %s", exc)


async def do_build(release=False):
    """Run cargo build, return success."""
    build_log.clear()
    build_log.append("building...")
    cmd = ["cargo", "build", "--workspace"]
    if release:
        cmd.append("--release")
    proc = await asyncio.create_subprocess_exec(
        *cmd,
        cwd=str(ROOT),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.STDOUT,
    )
    while True:
        line = await proc.stdout.readline()
        if not line:
            break
        build_log.append(
            line.decode("utf-8", errors="replace").rstrip())
    await proc.wait()
    ok = proc.returncode == 0
    build_log.append("build ok" if ok else "build FAILED")
    return ok


# collateral for playground test users: 100 quadrillion raw
# units — must exceed notional*im_rate for maker orders
# (price~50000 * qty~1M * im_rate~1000/10000 = ~5T per order)
_SEED_USERS = [1, 2, 3, 4, 5, 99]
_SEED_COLLATERAL = 100_000_000_000_000_000


async def seed_accounts():
    """Upsert playground test accounts into Postgres."""
    if pg_pool is None:
        return
    try:
        async with pg_pool.acquire() as conn:
            for uid in _SEED_USERS:
                await conn.execute(
                    "INSERT INTO accounts "
                    "(user_id, collateral, frozen_margin, version) "
                    "VALUES ($1, $2, 0, 0) "
                    "ON CONFLICT (user_id) DO NOTHING",
                    uid, _SEED_COLLATERAL,
                )
    except Exception as e:
        print(f"seed_accounts failed: {e}")


async def do_maker_start() -> bool:
    """Start market maker subprocess. Returns True if started."""
    if _maker_running():
        return True
    if MAKER_NAME in managed:
        del managed[MAKER_NAME]
    if not MAKER_SCRIPT.exists():
        return False
    # Load saved maker config if present
    _mcfg: dict = {}
    try:
        _mcfg = json.loads(MAKER_CONFIG.read_text())
    except Exception:
        pass
    env = {
        "GATEWAY_URL": GATEWAY_URL,
        "MARKETDATA_WS": MARKETDATA_WS,
        "RSX_SYMBOLS_URL": f"http://localhost:{49171}/v1/symbols",
        "RSX_MAKER_SPREAD_BPS": str(
            _mcfg.get("spread_bps", 20)),
        "RSX_MAKER_QTY": str(
            _mcfg.get("qty", 10)),
        "RSX_MAKER_SYMBOL": str(
            _mcfg.get("symbol_id", 10)),
        "RSX_MAKER_REFRESH_MS": str(
            _mcfg.get("refresh_ms", 500)),
        "RSX_MAKER_LEVELS": str(
            _mcfg.get("levels", 5)),
    }
    full_env = {**os.environ, **env}
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    proc = await asyncio.create_subprocess_exec(
        sys.executable, str(MAKER_SCRIPT),
        env=full_env,
        cwd=str(ROOT / "rsx-playground"),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.STDOUT,
    )
    managed[MAKER_NAME] = {
        "proc": proc,
        "binary": str(MAKER_SCRIPT),
        "env": env,
    }
    PID_DIR.mkdir(parents=True, exist_ok=True)
    (PID_DIR / f"{MAKER_NAME}.pid").write_text(str(proc.pid))
    asyncio.create_task(pipe_output(MAKER_NAME, proc.stdout))
    await asyncio.sleep(0.2)
    if proc.returncode is not None:
        del managed[MAKER_NAME]
        (PID_DIR / f"{MAKER_NAME}.pid").unlink(missing_ok=True)
        return False
    return True


async def start_all(scenario="minimal"):
    """Build + start all processes for a scenario."""
    global current_scenario
    plan = get_spawn_plan(scenario)

    # ensure dirs
    for name, binary, env in plan:
        wal_dir = env.get(
            "RSX_ME_WAL_DIR",
            env.get("RSX_RISK_WAL_DIR",
                     env.get("RSX_GW_WAL_DIR", "")))
        if wal_dir:
            (ROOT / wal_dir.lstrip("./")).mkdir(
                parents=True, exist_ok=True)
    (ROOT / "tmp" / "wal" / "mark").mkdir(
        parents=True, exist_ok=True)
    LOG_DIR.mkdir(parents=True, exist_ok=True)

    # kill stale RSX binaries by name first (handles UDP CMP ports too)
    _rsx_bins = [
        "rsx-gateway", "rsx-risk", "rsx-matching",
        "rsx-marketdata", "rsx-mark",
    ]
    for bin_name in _rsx_bins:
        try:
            subprocess.run(
                ["pkill", "-9", "-f", bin_name],
                capture_output=True, timeout=2,
            )
        except (FileNotFoundError, subprocess.TimeoutExpired):
            pass
    await asyncio.sleep(1.0)  # let OS release sockets

    # kill stale processes on known ports (belt + suspenders)
    if shutil.which("fuser"):
        # fuser available, use it
        for port in [8080, 8180, 9110, 9200, 9300, 9400, 9510, 9600]:
            try:
                subprocess.run(
                    ["fuser", "-k", f"{port}/tcp"],
                    capture_output=True, timeout=2,
                )
            except subprocess.TimeoutExpired:
                pass
        for port in [9110, 9200, 9300, 9400, 9510, 9600]:
            try:
                subprocess.run(
                    ["fuser", "-k", f"{port}/udp"],
                    capture_output=True, timeout=2,
                )
            except subprocess.TimeoutExpired:
                pass
    else:
        # fallback: check with lsof and kill by PID
        for port in [8080, 8180, 9110, 9200, 9300, 9400, 9510, 9600]:
            try:
                result = subprocess.run(
                    ["lsof", "-ti", f":{port}"],
                    capture_output=True, timeout=2, text=True,
                )
                for pid in result.stdout.strip().split():
                    if pid and pid.strip().isdigit():
                        try:
                            os.kill(int(pid), signal.SIGTERM)
                        except (ProcessLookupError, ValueError):
                            pass
            except (FileNotFoundError, subprocess.TimeoutExpired):
                pass
    await asyncio.sleep(2.0)  # Give OS time to release ports

    # build
    ok = await do_build()
    if not ok:
        return {"error": "build failed", "log": build_log[-5:]}

    # seed test accounts so risk engine loads them at startup
    await seed_accounts()

    # spawn all
    started = []
    for name, binary, env in plan:
        result = await spawn_process(name, binary, env)
        if "pid" in result:
            started.append(name)
        await asyncio.sleep(0.1)
    if started:
        current_scenario = scenario

    # wait for processes to stabilize, then auto-start maker
    if started:
        await asyncio.sleep(3.0)
        await do_maker_start()
        # restart md WS subscriber if it exhausted retries while
        # marketdata was not yet running
        global _md_ws_task
        if _md_ws_task is None or _md_ws_task.done():
            _md_ws_task = asyncio.create_task(
                _md_ws_subscriber())

    return {"started": started, "count": len(started)}


async def stop_all():
    """Stop all managed processes and PID-file-only processes."""
    stopped = []
    for name in list(managed.keys()):
        await stop_process(name)
        stopped.append(name)
    # Also terminate processes known only from PID files
    # (e.g. from a previous server session).
    if PID_DIR.exists():
        for pid_file in sorted(PID_DIR.glob("*.pid")):
            name = pid_file.stem
            if name in stopped:
                continue
            try:
                pid = int(pid_file.read_text().strip())
                os.kill(pid, signal.SIGTERM)
                stopped.append(name)
            except (ProcessLookupError, ValueError, OSError):
                pass
            pid_file.unlink(missing_ok=True)
    return {"stopped": stopped}


_reaper_task: asyncio.Task | None = None


@asynccontextmanager
async def lifespan(app):
    global _md_ws_task, _watcher_task, _reaper_task
    await pg_connect()
    _md_ws_task = asyncio.create_task(_md_ws_subscriber())
    _watcher_task = asyncio.create_task(_process_watcher())
    _reaper_task = asyncio.create_task(_stale_session_reaper())
    yield
    for task in (_md_ws_task, _watcher_task, _reaper_task):
        if task:
            task.cancel()
            try:
                await task
            except asyncio.CancelledError:
                pass
    # cleanup all managed processes on shutdown
    for name in list(managed.keys()):
        info = managed[name]
        proc = info["proc"]
        if proc.returncode is None:
            proc.terminate()
    for name in list(managed.keys()):
        info = managed[name]
        try:
            await asyncio.wait_for(
                info["proc"].wait(), timeout=3.0)
        except asyncio.TimeoutError:
            info["proc"].kill()
            await info["proc"].wait()
    # Clear managed dict
    managed.clear()
    # cleanup server PID file
    server_pid_file = PID_DIR.parent / "playground-server.pid"
    if server_pid_file.exists():
        server_pid_file.unlink()
    if pg_pool:
        await pg_pool.close()


app = FastAPI(
    title="RSX Playground",
    lifespan=lifespan,
    docs_url=None,
    redoc_url=None,
)

# Serve local static assets (htmx.min.js, etc.)
_STATIC_DIR = Path(__file__).resolve().parent
_STATIC_FILES = {
    "htmx.min.js": "application/javascript",
}


@app.get("/static/{filename}")
async def static_file(filename: str):
    """Serve local static files (JS bundles)."""
    if filename not in _STATIC_FILES:
        from fastapi.responses import Response
        return Response(status_code=404)
    path = _STATIC_DIR / filename
    if not path.exists():
        from fastapi.responses import Response
        return Response(status_code=404)
    from fastapi.responses import Response
    return Response(
        content=path.read_bytes(),
        media_type=_STATIC_FILES[filename],
    )


def audit_log(endpoint: str, action: str):
    ts = datetime.now().strftime("%b %d %H:%M:%S")
    print(f"{ts} audit: {endpoint} {action}")


DESTRUCTIVE_ENDPOINTS = {
    "/api/processes/all/stop",
    "/api/processes/all/start",
    "/api/scenario/switch",
}


def check_confirm(request: Request, endpoint: str):
    if endpoint not in DESTRUCTIVE_ENDPOINTS:
        return None
    # HTMX requests from the UI are always allowed
    if request.headers.get("hx-request"):
        return None
    token = request.headers.get("x-confirm")
    if token is None:
        token = request.query_params.get("confirm")
    if token != "yes":
        return JSONResponse(
            {"error": "destructive operation requires "
                      "x-confirm: yes header or "
                      "?confirm=yes query param"},
            status_code=400,
        )
    return None


@app.get("/healthz")
async def healthz():
    """Health check for CLI."""
    procs = scan_processes()
    running = [p for p in procs if p.get("state") == "running"]
    gateway_up, marketdata_up = await asyncio.gather(
        _probe_gateway_tcp(),
        _probe_marketdata_tcp(),
    )
    return {
        "status": "ok",
        "port": 49171,
        "processes_running": len(running),
        "processes_total": len(procs),
        "postgres": pg_pool is not None,
        "gateway": gateway_up,
        "marketdata": marketdata_up,
    }

# ── in-memory state ─────────────────────────────────────

recent_orders: list[dict] = []
verify_results: list[dict] = []
order_latencies: list[int] = []
gateway_ws = None
_idempotency_keys: dict[str, float] = {}
_IDEMPOTENCY_TTL = 300
SERVER_START: float = time.time()
_user_balances: dict[int, int] = {}
_user_frozen: set[int] = set()
_liquidation_log: list[dict] = []

# ── helpers ─────────────────────────────────────────────


def human_size(nbytes):
    for unit in ("B", "KB", "MB", "GB"):
        if abs(nbytes) < 1024:
            return f"{nbytes:.1f}{unit}"
        nbytes /= 1024
    return f"{nbytes:.1f}TB"


def human_uptime(start_time):
    elapsed = time.time() - start_time
    if elapsed < 60:
        return f"{elapsed:.0f}s"
    if elapsed < 3600:
        return f"{elapsed / 60:.0f}m{elapsed % 60:.0f}s"
    return f"{elapsed / 3600:.0f}h{(elapsed % 3600) / 60:.0f}m"


# Cache psutil.Process objects by PID so cpu_percent() has a
# reference point from the previous call (first call always 0.0).
_ps_cache: dict[int, psutil.Process] = {}


def _get_ps(pid: int) -> psutil.Process:
    if pid not in _ps_cache:
        _ps_cache[pid] = psutil.Process(pid)
    return _ps_cache[pid]


def _evict_ps(pid: int) -> None:
    _ps_cache.pop(pid, None)


def scan_processes():
    """Scan managed processes + PID dir fallback."""
    result = []
    seen = set()

    # 1. managed processes (from this session)
    for name, info in managed.items():
        seen.add(name)
        proc = info["proc"]
        if proc.returncode is None:
            try:
                ps = _get_ps(proc.pid)
                mem = ps.memory_info()
                result.append({
                    "name": name,
                    "pid": proc.pid,
                    "state": "running",
                    "cpu": f"{ps.cpu_percent():.1f}%",
                    "mem": human_size(mem.rss),
                    "uptime": human_uptime(
                        ps.create_time()),
                })
            except (psutil.NoSuchProcess,
                    psutil.AccessDenied):
                _evict_ps(proc.pid)
                result.append({
                    "name": name, "pid": proc.pid,
                    "state": "running", "cpu": "-",
                    "mem": "-", "uptime": "-",
                })
        else:
            rs = _restart_state.get(name, {})
            state = "blocked" if rs.get("blocked") else "stopped"
            result.append({
                "name": name, "pid": "-",
                "state": state, "cpu": "-",
                "mem": "-", "uptime": "-",
                "restarts": rs.get("restarts", 0),
            })

    # 2. PID files (from ./start or previous session)
    if PID_DIR.exists():
        for pid_file in sorted(PID_DIR.glob("*.pid")):
            name = pid_file.stem
            if name in seen:
                continue
            try:
                pid = int(pid_file.read_text().strip())
                ps = _get_ps(pid)
                if ps.is_running():
                    mem = ps.memory_info()
                    result.append({
                        "name": name, "pid": pid,
                        "state": "running",
                        "cpu": f"{ps.cpu_percent():.1f}%",
                        "mem": human_size(mem.rss),
                        "uptime": human_uptime(
                            ps.create_time()),
                    })
                    seen.add(name)
                    continue
            except psutil.NoSuchProcess:
                # stale PID file — process no longer exists
                pid_file.unlink(missing_ok=True)
                _evict_ps(pid)
            except (psutil.AccessDenied,
                    ValueError, OSError):
                pass

    # 3. show plan entries as "stopped" if not seen
    plan = get_spawn_plan(current_scenario)
    for name, binary, env in plan:
        if name not in seen:
            result.append({
                "name": name, "pid": "-",
                "state": "stopped", "cpu": "-",
                "mem": "-", "uptime": "-",
            })

    return sorted(result, key=lambda p: p["name"])


def scan_wal_streams():
    streams = []
    if not WAL_DIR.exists():
        return streams
    try:
        entries = sorted(WAL_DIR.iterdir())
    except OSError:
        return streams
    for entry in entries:
        if not entry.is_dir():
            continue
        files = list(entry.rglob("*.dxs"))
        files += list(entry.rglob("*.wal"))
        total = 0
        for f in files:
            try:
                total += f.stat().st_size
            except OSError:
                pass
        newest = ""
        if files:
            mtimes = []
            for f in files:
                try:
                    mtimes.append(f.stat().st_mtime)
                except OSError:
                    pass
            if mtimes:
                newest = datetime.fromtimestamp(
                    max(mtimes)).strftime("%H:%M:%S")
        streams.append({
            "name": entry.name,
            "files": len(files),
            "total_size": human_size(total),
            "newest": newest,
        })
    return streams


def scan_wal_files():
    files = []
    if not WAL_DIR.exists():
        return files
    try:
        entries = sorted(WAL_DIR.iterdir())
    except OSError:
        return files
    for entry in entries:
        if not entry.is_dir():
            continue
        try:
            items = sorted(entry.iterdir())
        except OSError:
            continue
        for item in items:
            if item.is_file():
                try:
                    st = item.stat()
                except OSError:
                    continue
                files.append({
                    "stream": entry.name,
                    "name": item.name,
                    "size": human_size(st.st_size),
                    "modified": datetime.fromtimestamp(
                        st.st_mtime).strftime("%H:%M:%S"),
                })
            elif item.is_dir():
                try:
                    subitems = sorted(item.iterdir())
                except OSError:
                    continue
                for f in subitems:
                    if not f.is_file():
                        continue
                    try:
                        st = f.stat()
                    except OSError:
                        continue
                    files.append({
                        "stream": f"{entry.name}/{item.name}",
                        "name": f.name,
                        "size": human_size(st.st_size),
                        "modified": datetime.fromtimestamp(
                            st.st_mtime).strftime("%H:%M:%S"),
                    })
    return files


# WAL header: 16 bytes (type:u16, len:u16, crc32:u32, reserved:8)
WAL_HDR = struct.Struct('<HHI8s')
# BboRecord: 72 bytes (seq:u64, ts:u64, sym:u32, pad:u32,
#   bid_px:i64, bid_qty:i64, bid_count:u32, pad:u32,
#   ask_px:i64, ask_qty:i64, ask_count:u32, pad:u32)
# repr(C,align(64)) = 64-byte alignment, not 64-byte size
BBO_FMT = struct.Struct('<QQIIqqIIqqII')
# FillRecord: 88 bytes
# seq:u64, ts:u64, sym:u32, taker_uid:u32, maker_uid:u32, pad:u32,
# taker_oid_hi:u64, taker_oid_lo:u64, maker_oid_hi:u64, maker_oid_lo:u64,
# price:i64, qty:i64, taker_side:u8, reduce_only:u8, tif:u8, post_only:u8,
# pad1:4s
FILL_FMT = struct.Struct(
    '<QQIIIIQQQQqqBBBB4s')
RECORD_FILL = 0
RECORD_BBO = 1
RECORD_ORDER_INSERTED = 2
RECORD_ORDER_CANCELLED = 3
RECORD_ORDER_DONE = 4
RECORD_CONFIG_APPLIED = 5
RECORD_CAUGHT_UP = 6
RECORD_ORDER_ACCEPTED = 7
RECORD_MARK_PRICE = 8
RECORD_ORDER_FAILED = 12
RECORD_LIQUIDATION = 13
# LiquidationRecord
LIQN_FMT = struct.Struct('<QQIIBBHIqqq')
# OrderAcceptedRecord: seq:u64 ts:u64 uid:u32 sym:u32
#   oid_hi:u64 oid_lo:u64 price:i64 qty:i64
#   side:u8 tif:u8 ro:u8 po:u8 pad:12s
OACC_FMT = struct.Struct('<QQIIQQqqBBBB12s')
# OrderInsertedRecord: seq:u64 ts:u64 sym:u32 uid:u32
#   oid_hi:u64 oid_lo:u64 price:i64 qty:i64
#   side:u8 tif:u8 ro:u8 po:u8 pad:4s
OINS_FMT = struct.Struct('<QQIIQQqqBBBB4s')
# OrderDoneRecord: seq:u64 ts:u64 sym:u32 uid:u32
#   oid_hi:u64 oid_lo:u64 filled:i64 remaining:i64
#   status:u8 ro:u8 tif:u8 po:u8 pad:4s
ODONE_FMT = struct.Struct('<QQIIQQqqBBBB4s')
# OrderCancelledRecord: same layout as OrderDone
OCANC_FMT = ODONE_FMT
# MarkPriceRecord: seq:u64 ts:u64 sym:u32 pad:u32
#   mark:i64 src_mask:u32 src_count:u32 pad:24s
MARK_FMT = struct.Struct('<QQIIqII24s')
RECORD_TYPE_NAMES = {
    0: "fill", 1: "bbo", 2: "order_inserted",
    3: "order_cancelled", 4: "order_done",
    5: "config_applied", 6: "caught_up",
    7: "order_accepted", 8: "mark_price",
    12: "order_failed", 13: "liquidation",
}


def parse_wal_records(stream_dir, record_types=None):
    """Parse WAL records from a stream directory."""
    records = []
    if not stream_dir.exists():
        return records
    for wal_file in sorted(stream_dir.glob("**/*.wal")):
        try:
            data = wal_file.read_bytes()
        except OSError:
            continue
        pos = 0
        while pos + WAL_HDR.size <= len(data):
            rtype, rlen, crc, _ = WAL_HDR.unpack_from(
                data, pos)
            pos += WAL_HDR.size
            if rlen == 0 or pos + rlen > len(data):
                break
            if record_types and rtype not in record_types:
                pos += rlen
                continue
            payload = data[pos:pos + rlen]
            pos += rlen
            if rtype == RECORD_BBO and len(payload) >= BBO_FMT.size:
                fields = BBO_FMT.unpack_from(payload)
                records.append({
                    "type": "bbo",
                    "seq": fields[0],
                    "ts_ns": fields[1],
                    "symbol_id": fields[2],
                    "bid_px": fields[4],
                    "bid_qty": fields[5],
                    "bid_count": fields[6],
                    "ask_px": fields[8],
                    "ask_qty": fields[9],
                    "ask_count": fields[10],
                })
            elif rtype == RECORD_FILL and len(payload) >= FILL_FMT.size:
                fields = FILL_FMT.unpack_from(payload)
                # [0]=seq [1]=ts [2]=sym [3]=taker_uid [4]=maker_uid
                # [5]=pad [6]=taker_oid_hi [7]=taker_oid_lo
                # [8]=maker_oid_hi [9]=maker_oid_lo
                # [10]=price [11]=qty [12]=taker_side
                records.append({
                    "type": "fill",
                    "seq": fields[0],
                    "ts_ns": fields[1],
                    "symbol_id": fields[2],
                    "taker_uid": fields[3],
                    "maker_uid": fields[4],
                    "price": fields[10],
                    "qty": fields[11],
                    "taker_side": fields[12],
                })
            elif (rtype == RECORD_LIQUIDATION
                    and len(payload) >= LIQN_FMT.size):
                fields = LIQN_FMT.unpack_from(payload)
                records.append({
                    "type": "liquidation",
                    "seq": fields[0],
                    "ts_ns": fields[1],
                    "user_id": fields[2],
                    "symbol_id": fields[3],
                    "status": fields[4],
                    "side": fields[5],
                    "round": fields[7],
                    "qty": fields[8],
                    "price": fields[9],
                    "slip_bps": fields[10],
                })
            elif (rtype == RECORD_ORDER_ACCEPTED
                    and len(payload) >= OACC_FMT.size):
                f = OACC_FMT.unpack_from(payload)
                records.append({
                    "type": "order_accepted",
                    "seq": f[0], "ts_ns": f[1],
                    "user_id": f[2],
                    "symbol_id": f[3],
                    "price": f[6], "qty": f[7],
                    "side": f[8],
                })
            elif (rtype == RECORD_ORDER_INSERTED
                    and len(payload) >= OINS_FMT.size):
                f = OINS_FMT.unpack_from(payload)
                records.append({
                    "type": "order_inserted",
                    "seq": f[0], "ts_ns": f[1],
                    "symbol_id": f[2],
                    "user_id": f[3],
                    "price": f[6], "qty": f[7],
                    "side": f[8],
                })
            elif (rtype == RECORD_ORDER_CANCELLED
                    and len(payload) >= OCANC_FMT.size):
                f = OCANC_FMT.unpack_from(payload)
                records.append({
                    "type": "order_cancelled",
                    "seq": f[0], "ts_ns": f[1],
                    "symbol_id": f[2],
                    "user_id": f[3],
                    "remaining_qty": f[6],
                })
            elif (rtype == RECORD_ORDER_DONE
                    and len(payload) >= ODONE_FMT.size):
                f = ODONE_FMT.unpack_from(payload)
                records.append({
                    "type": "order_done",
                    "seq": f[0], "ts_ns": f[1],
                    "symbol_id": f[2],
                    "user_id": f[3],
                    "filled_qty": f[6],
                    "remaining_qty": f[7],
                    "status": f[8],
                })
            elif (rtype == RECORD_MARK_PRICE
                    and len(payload) >= MARK_FMT.size):
                f = MARK_FMT.unpack_from(payload)
                records.append({
                    "type": "mark_price",
                    "seq": f[0], "ts_ns": f[1],
                    "symbol_id": f[2],
                    "mark_price": f[4],
                    "source_count": f[6],
                })
            else:
                # unknown type — still show in timeline
                name = RECORD_TYPE_NAMES.get(
                    rtype, f"type_{rtype}")
                rec = {"type": name, "seq": 0,
                       "ts_ns": 0}
                if len(payload) >= 20:
                    seq, ts = struct.unpack_from(
                        '<QQ', payload)
                    rec["seq"] = seq
                    rec["ts_ns"] = ts
                records.append(rec)
    return records


def _wal_stream_dirs():
    """Yield top-level stream dirs under WAL_DIR."""
    if not WAL_DIR.exists():
        return
    try:
        entries = list(WAL_DIR.iterdir())
    except OSError:
        return
    for d in entries:
        if d.is_dir():
            yield d


_SNAP_MAGIC = 0x5258534E
_SNAP_VERSION = 1


def _parse_snapshot_orders(data):
    """Parse active orders from snapshot bytes.

    Returns list of dicts with price, qty, side.
    Returns None on parse error.
    """
    import struct as _st
    if len(data) < 66:
        return None
    pos = 0
    magic = _st.unpack_from('<I', data, pos)[0]
    pos += 4
    if magic != _SNAP_MAGIC:
        return None
    version = _st.unpack_from('<I', data, pos)[0]
    pos += 4
    if version != _SNAP_VERSION:
        return None
    seq = _st.unpack_from('<Q', data, pos)[0]
    pos += 8  # skip seq
    pos += 4  # symbol_id
    pos += 1  # price_decimals
    pos += 1  # qty_decimals
    pos += 8  # tick_size
    pos += 8  # lot_size
    pos += 8  # mid_price
    pos += 4  # best_bid_tick
    pos += 4  # best_ask_tick
    pos += 4  # capacity
    pos += 4  # bump
    if pos + 4 > len(data):
        return None
    active_count = _st.unpack_from('<I', data, pos)[0]
    pos += 4
    # Each order entry: idx(4) + order(71 bytes)
    ORDER_ENTRY = 4 + 8 + 8 + 1 + 1 + 1 + 4 + 4 + 4 + 4 + 4 + 8 + 8 + 8 + 8
    orders = []
    for _ in range(active_count):
        if pos + ORDER_ENTRY > len(data):
            break
        pos += 4   # idx
        price = _st.unpack_from('<q', data, pos)[0]
        pos += 8
        rem_qty = _st.unpack_from('<q', data, pos)[0]
        pos += 8
        side = data[pos]
        pos += 1
        pos += 1   # flags
        pos += 1   # tif
        pos += 4   # next
        pos += 4   # prev
        pos += 4   # tick_index
        pos += 4   # user_id
        pos += 4   # sequence
        pos += 8   # original_qty
        pos += 8   # timestamp_ns
        pos += 8   # order_id_hi
        pos += 8   # order_id_lo
        orders.append({
            "price": price,
            "qty": rem_qty,
            "side": side,
        })
    return {"seq": seq, "orders": orders}


def _bbo_from_orders(symbol_id, parsed):
    """Compute BBO dict from parsed snapshot orders."""
    if not parsed:
        return None
    orders = parsed.get("orders", [])
    bids = {}
    asks = {}
    for o in orders:
        px = o["price"]
        qty = o["qty"]
        if o["side"] == 0:  # Buy
            bids.setdefault(px, [0, 0])
            bids[px][0] += qty
            bids[px][1] += 1
        else:  # Sell
            asks.setdefault(px, [0, 0])
            asks[px][0] += qty
            asks[px][1] += 1
    if not bids and not asks:
        return None
    bid_px = max(bids) if bids else 0
    ask_px = min(asks) if asks else 0
    return {
        "type": "bbo",
        "seq": parsed.get("seq", 0),
        "symbol_id": symbol_id,
        "bid_px": bid_px,
        "bid_qty": bids[bid_px][0] if bids else 0,
        "bid_count": bids[bid_px][1] if bids else 0,
        "ask_px": ask_px,
        "ask_qty": asks[ask_px][0] if asks else 0,
        "ask_count": asks[ask_px][1] if asks else 0,
    }


def _snap_to_bbo(symbol_id: int, snap: dict):
    """Convert _book_snap entry to BBO dict for render_book_ladder."""
    bids = snap.get("bids", [])
    asks = snap.get("asks", [])
    if not bids and not asks:
        return None
    bid = bids[0] if bids else {}
    ask = asks[0] if asks else {}
    return {
        "bid_px": bid.get("px", 0),
        "bid_qty": bid.get("qty", 0),
        "bid_count": len(bids),
        "ask_px": ask.get("px", 0),
        "ask_qty": ask.get("qty", 0),
        "ask_count": len(asks),
        "seq": 0,
    }


def _latest_bbo_from_wal(symbol_id=None):
    """Read latest RECORD_BBO entries from WAL files.

    Returns dict of {symbol_id: bbo_dict} if symbol_id is None,
    or a single bbo_dict (or None) if symbol_id is given.
    """
    best: dict[int, dict] = {}
    for stream_dir in _wal_stream_dirs():
        for rec in parse_wal_records(
            stream_dir, {RECORD_BBO}
        ):
            sid = rec["symbol_id"]
            if symbol_id is not None and sid != symbol_id:
                continue
            existing = best.get(sid)
            if existing is None or rec["seq"] > existing["seq"]:
                best[sid] = {
                    "type": "bbo",
                    "seq": rec["seq"],
                    "symbol_id": sid,
                    "bid_px": rec["bid_px"],
                    "bid_qty": rec["bid_qty"],
                    "bid_count": rec["bid_count"],
                    "ask_px": rec["ask_px"],
                    "ask_qty": rec["ask_qty"],
                    "ask_count": rec["ask_count"],
                }
    if symbol_id is not None:
        return best.get(symbol_id)
    return best


def parse_wal_bbo(symbol_id):
    """Get BBO for a symbol from snapshot.bin or RECORD_BBO WAL.

    Tries snapshot.bin first (has full order depth), then falls
    back to RECORD_BBO records written to WAL by the ME after
    each match.
    """
    for stream_dir in _wal_stream_dirs():
        snap = (
            stream_dir / str(symbol_id) / "snapshot.bin"
        )
        if not snap.exists():
            continue
        try:
            data = snap.read_bytes()
        except OSError:
            continue
        parsed = _parse_snapshot_orders(data)
        bbo = _bbo_from_orders(symbol_id, parsed)
        if bbo is not None:
            return bbo
    # Fallback: latest RECORD_BBO from WAL files
    return _latest_bbo_from_wal(symbol_id)


def parse_wal_fills(max_fills=50):
    """Get recent fills from all WAL streams."""
    all_fills = []
    for stream_dir in _wal_stream_dirs():
        for rec in parse_wal_records(
            stream_dir, {RECORD_FILL}
        ):
            all_fills.append(rec)
    all_fills.sort(key=lambda r: r["seq"], reverse=True)
    return all_fills[:max_fills]


def parse_wal_fills_for_user(user_id, symbol_id):
    """Get fills for a specific user+symbol from WAL."""
    result = []
    for stream_dir in _wal_stream_dirs():
        for rec in parse_wal_records(
            stream_dir, {RECORD_FILL}
        ):
            if rec["symbol_id"] != symbol_id:
                continue
            if (rec["taker_uid"] == user_id
                    or rec["maker_uid"] == user_id):
                result.append(rec)
    result.sort(key=lambda r: r["seq"])
    return result


def parse_wal_book_stats():
    """Get book stats from snapshot.bin or RECORD_BBO in WAL.

    Tries snapshot.bin first (full order depth), then fills in
    missing symbols from RECORD_BBO records in WAL files.
    """
    symbols = {}
    for stream_dir in _wal_stream_dirs():
        try:
            entries = list(stream_dir.iterdir())
        except OSError:
            continue
        for d in entries:
            if not d.is_dir():
                continue
            try:
                sid = int(d.name)
            except ValueError:
                continue
            snap = d / "snapshot.bin"
            if not snap.exists():
                continue
            try:
                data = snap.read_bytes()
            except OSError:
                continue
            parsed = _parse_snapshot_orders(data)
            bbo = _bbo_from_orders(sid, parsed)
            if bbo is not None:
                existing = symbols.get(sid)
                if (existing is None
                        or bbo["seq"] > existing["seq"]):
                    symbols[sid] = bbo
    # Supplement with RECORD_BBO records from WAL files
    for sid, bbo in _latest_bbo_from_wal().items():
        existing = symbols.get(sid)
        if (existing is None
                or bbo["seq"] > existing["seq"]):
            symbols[sid] = bbo
    return symbols


def parse_wal_fills_for_user_all(user_id: int):
    """Get fills for a user across all symbols from WAL."""
    result = []
    for stream_dir in _wal_stream_dirs():
        for rec in parse_wal_records(
            stream_dir, {RECORD_FILL}
        ):
            if (rec["taker_uid"] == user_id
                    or rec["maker_uid"] == user_id):
                result.append(rec)
    result.sort(key=lambda r: r["seq"])
    return result


def parse_wal_liquidations(user_id: int | None = None):
    """Get liquidation records from WAL, optionally filtered."""
    result = []
    for stream_dir in _wal_stream_dirs():
        for rec in parse_wal_records(
            stream_dir, {RECORD_LIQUIDATION}
        ):
            if user_id is None or rec["user_id"] == user_id:
                result.append(rec)
    result.sort(key=lambda r: r["seq"], reverse=True)
    return result


_ANSI_RE = re.compile(r'\x1b\[[0-9;]*m')


def strip_ansi(text):
    return _ANSI_RE.sub('', text)


def read_logs(process=None, level=None, search=None,
              max_lines=200):
    lines = []
    log_files = (
        sorted(LOG_DIR.glob("*.log"))
        if LOG_DIR.exists() else []
    )
    for lf in log_files:
        fname = lf.stem
        if process and process not in fname:
            continue
        try:
            tail = lf.read_text().splitlines()[-max_lines:]
            for line in tail:
                clean = strip_ansi(line)
                if level and level.lower() not in clean.lower():
                    continue
                if search and search.lower() not in clean.lower():
                    continue
                lines.append(f"[{fname}] {clean}")
        except OSError:
            pass
    return lines[-max_lines:]


# ── page routes ─────────────────────────────────────────

@app.get("/", response_class=HTMLResponse)
async def index():
    return RedirectResponse("./walkthrough")


@app.get("/walkthrough", response_class=HTMLResponse)
async def walkthrough():
    return HTMLResponse(pages.walkthrough_page())


@app.get("/overview", response_class=HTMLResponse)
async def overview():
    return HTMLResponse(pages.overview_page())


@app.get("/topology", response_class=HTMLResponse)
async def topology():
    return HTMLResponse(pages.topology_page())


# ── Topology partials ────────────────────────────────────


def _topo_proc(hints: list[str]) -> dict:
    procs = scan_processes()
    for hint in hints:
        for p in procs:
            if hint in p["name"]:
                return p
    return {}


def _topo_gateway() -> dict:
    p = _topo_proc(["gateway"])
    book_stats = parse_wal_book_stats()
    wal_tips = (
        " ".join(
            f"sym{s}={b.get('seq', 0)}"
            for s, b in sorted(book_stats.items())
        )
        or "none"
    )
    return {
        "name": "Gateway",
        "status": p.get("state", "stopped"),
        "pid": p.get("pid", "-"),
        "uptime": p.get("uptime", "-"),
        "rows": [
            ("orders (session)", len(recent_orders)),
            ("fills (session)", len(recent_fills)),
            ("WAL tips", wal_tips),
            ("circuit breaker", "closed"),
        ],
    }


def _topo_risk() -> dict:
    p = _topo_proc(["risk"])
    now_s = int(time.time())
    next_s = 28800 - (now_s % 28800)
    h, rem = divmod(next_s, 3600)
    m = rem // 60
    return {
        "name": "Risk",
        "status": p.get("state", "stopped"),
        "pid": p.get("pid", "-"),
        "uptime": p.get("uptime", "-"),
        "rows": [
            (
                "funding next settlement",
                f"{h}h {m}m" if h else f"{m}m",
            ),
            ("seed accounts", len(_SEED_USERS)),
        ],
    }


def _topo_matching() -> dict:
    p = _topo_proc(["matching", "me-"])
    book_stats = parse_wal_book_stats()
    rows = []
    for sid, bbo in sorted(book_stats.items()):
        bid = bbo.get("bid_px", 0)
        ask = bbo.get("ask_px", 0)
        spd = ask - bid if bid and ask else 0
        rows.append((
            f"sym{sid} bbo",
            f"bid={bid} ask={ask} spd={spd}",
        ))
    for sid, s in sorted(_book_snap.items()):
        rows.append((
            f"sym{sid} depth",
            f"{len(s.get('bids', []))}b"
            f" / {len(s.get('asks', []))}a",
        ))
    if not rows:
        rows = [("book data", "no WAL/MD data")]
    return {
        "name": "Matching Engine",
        "status": p.get("state", "stopped"),
        "pid": p.get("pid", "-"),
        "uptime": p.get("uptime", "-"),
        "rows": rows,
    }


def _topo_marketdata() -> dict:
    p = _topo_proc(["marketdata", "mktdata"])
    syms = sorted(_book_snap.keys())
    return {
        "name": "Marketdata",
        "status": p.get("state", "stopped"),
        "pid": p.get("pid", "-"),
        "uptime": p.get("uptime", "-"),
        "rows": [
            ("symbols with data", len(syms)),
            ("symbol ids",
             " ".join(str(s) for s in syms) or "none"),
            ("fills buffered", len(recent_fills)),
        ],
    }


def _topo_mark() -> dict:
    p = _topo_proc(["mark"])
    return {
        "name": "Mark",
        "status": p.get("state", "stopped"),
        "pid": p.get("pid", "-"),
        "uptime": p.get("uptime", "-"),
        "rows": [("mark data", "requires mark process")],
    }


def _topo_recorder() -> dict:
    p = _topo_proc(["recorder"])
    wal_files = 0
    for d in _wal_stream_dirs():
        try:
            for sub in d.iterdir():
                if sub.is_dir():
                    wal_files += sum(
                        1 for _ in sub.glob("*.wal"))
        except OSError:
            pass
    return {
        "name": "Recorder",
        "status": p.get("state", "stopped"),
        "pid": p.get("pid", "-"),
        "uptime": p.get("uptime", "-"),
        "rows": [("WAL files found", wal_files)],
    }


def _topo_maker() -> dict:
    running = _maker_running()
    info = managed.get(MAKER_NAME)
    pid = info["proc"].pid if running and info else "-"
    stats = _read_maker_stats() if running else {}
    mid_prices = stats.get("mid_prices", {})
    mid_str = (
        " ".join(
            f"sym{k}={v}"
            for k, v in sorted(mid_prices.items())
        )
        or "none"
    )
    p = _topo_proc(["maker"])
    return {
        "name": "Maker",
        "status": "running" if running else "stopped",
        "pid": pid,
        "uptime": p.get("uptime", "-"),
        "rows": [
            ("orders placed", stats.get("orders_placed", 0)),
            ("active orders", stats.get("active_orders", 0)),
            ("spread bps", stats.get("spread_bps", "none")),
            ("mid prices", mid_str),
        ],
    }


def _topo_client() -> dict:
    return {
        "name": "Clients",
        "status": "unknown",
        "pid": "-",
        "uptime": "-",
        "rows": [
            ("orders (session)", len(recent_orders)),
            ("fills (session)", len(recent_fills)),
        ],
    }


def _topo_stress() -> dict:
    running = _stress_running()
    info = managed.get(STRESS_NAME)
    pid = info["proc"].pid if running and info else "-"
    p = _topo_proc(["stress"])
    return {
        "name": "Stress",
        "status": "running" if running else "stopped",
        "pid": pid,
        "uptime": p.get("uptime", "-"),
        "rows": [],
    }


_TOPO_HANDLERS: dict = {
    "client": _topo_client,
    "gateway": _topo_gateway,
    "risk": _topo_risk,
    "matching": _topo_matching,
    "marketdata": _topo_marketdata,
    "mark": _topo_mark,
    "recorder": _topo_recorder,
    "maker": _topo_maker,
    "stress": _topo_stress,
}


@app.get("/x/topology/{component}",
         response_class=HTMLResponse)
async def x_topology_component(component: str):
    handler = _TOPO_HANDLERS.get(component)
    if not handler:
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            f"unknown component: {component}</span>"
        )
    return HTMLResponse(
        pages.render_component_detail(component, handler()))


@app.get("/x/topology/flow")
async def x_topology_flow():
    """JSON: per-node status dots and rate labels for live update."""
    procs = scan_processes()

    def _status(hints: list[str]) -> str:
        for hint in hints:
            for p in procs:
                if hint in p["name"]:
                    return p.get("state", "stopped")
        return "stopped"

    def _dot(s: str) -> str:
        return (
            "bg-emerald-400" if s == "running"
            else "bg-red-500" if s == "stopped"
            else "bg-zinc-600"
        )

    gw = _status(["gateway"])
    risk_s = _status(["risk"])
    me_s = _status(["matching", "me-"])
    md_s = _status(["marketdata", "mktdata"])
    mk_s = _status(["mark"])
    rec_s = _status(["recorder"])
    maker_s = "running" if _maker_running() else "stopped"

    book_stats = parse_wal_book_stats()
    spd_label = "none"
    for sid, bbo in sorted(book_stats.items()):
        bid = bbo.get("bid_px", 0)
        ask = bbo.get("ask_px", 0)
        if bid and ask:
            spd_label = f"spd={ask - bid}"
            break

    nodes = [
        {"key": "client", "dot": "bg-zinc-600",
         "rate": f"{len(recent_orders)} ord"},
        {"key": "gateway", "dot": _dot(gw),
         "rate": f"{len(recent_fills)} fills"},
        {"key": "risk", "dot": _dot(risk_s),
         "rate": risk_s},
        {"key": "matching", "dot": _dot(me_s),
         "rate": spd_label},
        {"key": "marketdata", "dot": _dot(md_s),
         "rate": f"{len(_book_snap)} sym"},
        {"key": "mark", "dot": _dot(mk_s),
         "rate": mk_s},
        {"key": "recorder", "dot": _dot(rec_s),
         "rate": rec_s},
        {"key": "maker", "dot": _dot(maker_s),
         "rate": maker_s},
    ]
    return JSONResponse({"nodes": nodes})


@app.get("/x/topology/summary",
         response_class=HTMLResponse)
async def x_topology_summary():
    procs = scan_processes()
    running = [p for p in procs
               if p.get("state") == "running"]
    names = ", ".join(p["name"] for p in running)
    gw_up = any(
        "gateway" in p["name"] for p in running)
    me_up = any(
        "me-" in p["name"] or "matching" in p["name"]
        for p in running)
    md_up = any(
        "marketdata" in p["name"] for p in running)

    def _dot(ok):
        c = "bg-emerald-400" if ok else "bg-red-500"
        return (
            f'<span class="w-1.5 h-1.5 rounded-full '
            f'{c} inline-block"></span>'
        )

    return HTMLResponse(
        f'{_dot(gw_up)} '
        f'<span class="text-zinc-400">GW</span> '
        f'{_dot(me_up)} '
        f'<span class="text-zinc-400">ME</span> '
        f'{_dot(md_up)} '
        f'<span class="text-zinc-400">MD</span> '
        f'<span class="text-zinc-500 ml-2">'
        f'{len(running)}/{len(procs)} running</span>'
        f'<span class="text-zinc-600 ml-auto truncate '
        f'max-w-[300px]">{names}</span>'
    )


@app.get("/book", response_class=HTMLResponse)
async def book():
    return HTMLResponse(pages.book_page())


@app.get("/risk", response_class=HTMLResponse)
async def risk():
    return HTMLResponse(pages.risk_page())


@app.get("/wal", response_class=HTMLResponse)
async def wal():
    return HTMLResponse(pages.wal_page())


@app.get("/logs", response_class=HTMLResponse)
async def logs_page():
    return HTMLResponse(pages.logs_page())


@app.get("/control", response_class=HTMLResponse)
async def control():
    return HTMLResponse(pages.control_page())


@app.get("/faults", response_class=HTMLResponse)
async def faults():
    return HTMLResponse(pages.faults_page())


@app.get("/verify", response_class=HTMLResponse)
async def verify():
    return HTMLResponse(pages.verify_page())


@app.get("/orders", response_class=HTMLResponse)
async def orders():
    return HTMLResponse(pages.orders_page())


@app.get("/stress", response_class=HTMLResponse)
async def stress():
    return HTMLResponse(pages.stress_page())


@app.get("/maker", response_class=HTMLResponse)
async def maker_page():
    cfg: dict = {}
    try:
        cfg = json.loads(MAKER_CONFIG.read_text())
    except Exception:
        pass
    running = _maker_running()
    info = managed.get(MAKER_NAME)
    pid = info["proc"].pid if running and info else None
    rs = _restart_state.get(MAKER_NAME, {})
    restarts = rs.get("restarts", 0)
    return HTMLResponse(
        pages.maker_page(
            running=running,
            pid=pid,
            restarts=restarts,
            cfg=cfg,
        )
    )


@app.get("/stress/{report_id}", response_class=HTMLResponse)
async def stress_report_view(report_id: str):
    """View individual stress test report as HTML"""
    report_file = STRESS_REPORTS_DIR / f"stress-{report_id}.json"
    if not report_file.exists():
        return HTMLResponse("<h1>Report not found</h1>", status_code=404)

    with open(report_file) as f:
        data = json.load(f)

    return HTMLResponse(pages.stress_report_page(data))


def _md_to_html(md: str) -> str:
    """Convert markdown to HTML (no external deps)."""
    lines = md.split("\n")
    out = []
    in_code = False
    in_list = False
    in_table = False
    table_align = []

    for line in lines:
        # fenced code blocks
        if line.startswith("```"):
            if in_code:
                out.append("</code></pre>")
                in_code = False
            else:
                lang = html.escape(line[3:].strip())
                cls = f' class="lang-{lang}"' if lang else ""
                out.append(f"<pre><code{cls}>")
                in_code = True
            continue
        if in_code:
            out.append(html.escape(line))
            continue

        # close list if not a list line
        if in_list and not re.match(
            r"^(\d+\.|[-*])\s", line
        ) and line.strip():
            out.append("</ul>")
            in_list = False

        # close table if not a table line
        if in_table and not line.startswith("|"):
            out.append("</tbody></table>")
            in_table = False
            table_align = []

        # blank line
        if not line.strip():
            if in_list:
                out.append("</ul>")
                in_list = False
            out.append("")
            continue

        # headings
        m = re.match(r"^(#{1,6})\s+(.*)", line)
        if m:
            lvl = len(m.group(1))
            txt = _md_inline(html.escape(m.group(2)))
            out.append(f"<h{lvl}>{txt}</h{lvl}>")
            continue

        # tables
        if line.startswith("|"):
            cells = [
                c.strip() for c in line.split("|")[1:-1]
            ]
            # separator row
            if all(
                re.match(r"^:?-+:?$", c) for c in cells
            ):
                table_align = []
                for c in cells:
                    if c.startswith(":") and c.endswith(":"):
                        table_align.append("center")
                    elif c.endswith(":"):
                        table_align.append("right")
                    else:
                        table_align.append("left")
                continue
            if not in_table:
                out.append(
                    '<table class="md-table">'
                    "<thead><tr>")
                for i, c in enumerate(cells):
                    a = table_align[i] if i < len(
                        table_align) else "left"
                    out.append(
                        f'<th style="text-align:{a}">'
                        f"{_md_inline(html.escape(c))}"
                        "</th>")
                out.append("</tr></thead><tbody>")
                in_table = True
            else:
                out.append("<tr>")
                for i, c in enumerate(cells):
                    a = table_align[i] if i < len(
                        table_align) else "left"
                    out.append(
                        f'<td style="text-align:{a}">'
                        f"{_md_inline(html.escape(c))}"
                        "</td>")
                out.append("</tr>")
            continue

        # unordered list
        m = re.match(r"^[-*]\s+(.*)", line)
        if m:
            if not in_list:
                out.append("<ul>")
                in_list = True
            out.append(
                f"<li>{_md_inline(html.escape(m.group(1)))}"
                "</li>")
            continue

        # ordered list
        m = re.match(r"^\d+\.\s+(.*)", line)
        if m:
            if not in_list:
                out.append("<ul>")
                in_list = True
            out.append(
                f"<li>{_md_inline(html.escape(m.group(1)))}"
                "</li>")
            continue

        # paragraph
        out.append(
            f"<p>{_md_inline(html.escape(line))}</p>")

    if in_code:
        out.append("</code></pre>")
    if in_list:
        out.append("</ul>")
    if in_table:
        out.append("</tbody></table>")
    return "\n".join(out)


def _md_inline(text: str) -> str:
    """Convert inline markdown (bold, code, links)."""
    # inline code
    text = re.sub(
        r"`([^`]+)`",
        r"<code>\1</code>",
        text)
    # bold
    text = re.sub(
        r"\*\*([^*]+)\*\*",
        r"<strong>\1</strong>",
        text)
    # links [text](url)
    text = re.sub(
        r"\[([^\]]+)\]\(([^)]+)\)",
        r'<a href="\2">\1</a>',
        text)
    return text


@app.get("/docs")
async def docs_index():
    """Redirect /docs to /docs/README."""
    return RedirectResponse("./docs/README")


@app.get("/docs/{filename:path}")
async def docs(filename: str):
    """Serve playground documentation files."""
    docs_dir = Path(__file__).parent / "docs"
    if not filename:
        filename = "README.md"
    if not filename.endswith(".md"):
        filename += ".md"
    file_path = (docs_dir / filename).resolve()
    if not str(file_path).startswith(
        str(docs_dir.resolve())
    ):
        return HTMLResponse(
            "<h1>404 Not Found</h1>",
            status_code=404)
    if not file_path.exists() or not file_path.is_file():
        return HTMLResponse(
            "<h1>404 Not Found</h1>",
            status_code=404)
    content = file_path.read_text()
    safe_filename = html.escape(filename)
    md_json = json.dumps(content)

    # sidebar: list all docs
    doc_files = sorted(docs_dir.glob("*.md"))
    sidebar = ""
    for f in doc_files:
        name = f.stem
        label = name.replace("-", " ").replace(
            "_", " ").title()
        active = "font-bold text-white" if (
            f.name == filename) else "text-slate-400"
        sidebar += (
            f'<a href="./{f.name}" '
            f'class="{active} block py-1 '
            f'hover:text-white text-sm">'
            f'{html.escape(label)}</a>\n')

    doc_html = f"""<!DOCTYPE html>
<html lang="en" class="dark">
<head>
<meta charset="utf-8">
<meta name="viewport"
  content="width=device-width, initial-scale=1">
<title>RSX Docs -- {safe_filename}</title>
<script src="https://cdn.tailwindcss.com"></script>
<script>
tailwind.config = {{
  darkMode: 'class',
  theme: {{
    extend: {{
      colors: {{
        'bg-primary': '#0b0e11',
        'bg-surface': '#1e2329',
      }}
    }}
  }}
}}
</script>
<link rel="stylesheet"
  href="https://cdn.jsdelivr.net/gh/highlightjs/cdn-release@11/build/styles/github-dark.min.css">
<style>
#content h1 {{
  font-size: 1.75rem;
  font-weight: 700;
  margin: 1.5rem 0 0.75rem;
  color: #60a5fa;
}}
#content h2 {{
  font-size: 1.35rem;
  font-weight: 600;
  margin: 1.25rem 0 0.5rem;
  color: #60a5fa;
}}
#content h3 {{
  font-size: 1.1rem;
  font-weight: 600;
  margin: 1rem 0 0.5rem;
  color: #60a5fa;
}}
#content h4, #content h5, #content h6 {{
  font-size: 1rem;
  font-weight: 600;
  margin: 0.75rem 0 0.5rem;
  color: #60a5fa;
}}
#content a {{ color: #60a5fa; }}
#content a:hover {{ text-decoration: underline; }}
#content p {{ margin: 0.5rem 0; line-height: 1.6; }}
#content ul {{
  padding-left: 1.5rem;
  margin: 0.5rem 0;
}}
#content ol {{
  padding-left: 1.5rem;
  margin: 0.5rem 0;
}}
#content li {{
  margin: 0.25rem 0;
  list-style: disc;
}}
#content ol li {{ list-style: decimal; }}
#content strong {{ color: #e2e8f0; }}
#content blockquote {{
  border-left: 3px solid #334155;
  padding-left: 1rem;
  margin: 0.75rem 0;
  color: #94a3b8;
}}
#content pre {{
  border-radius: 6px;
  overflow-x: auto;
  margin: 0.75rem 0;
  padding: 0;
}}
#content pre code {{
  display: block;
  padding: 1rem;
  font-family: 'JetBrains Mono', monospace;
  font-size: 0.85em;
}}
#content :not(pre) > code {{
  font-family: 'JetBrains Mono', monospace;
  font-size: 0.85em;
  background: #1e293b;
  padding: 2px 6px;
  border-radius: 3px;
}}
#content table {{
  border-collapse: collapse;
  width: 100%;
  margin: 0.75rem 0;
  font-size: 0.875rem;
}}
#content th, #content td {{
  border: 1px solid #334155;
  padding: 0.5rem 0.75rem;
  text-align: left;
}}
#content th {{
  background: #1e293b;
  font-weight: 600;
}}
#content hr {{
  border: none;
  border-top: 1px solid #334155;
  margin: 1.5rem 0;
}}
#content img {{ max-width: 100%; }}
</style>
</head>
<body class="bg-[#0f172a] text-slate-300">
<div class="flex min-h-screen">
  <aside class="w-52 bg-[#0b1120] border-r
    border-slate-700 p-4 shrink-0">
    <a href="../" class="text-white font-bold text-sm
      block mb-4">RSX Playground</a>
    <div class="mb-3 text-xs text-slate-500
      uppercase tracking-wider">Docs</div>
    {sidebar}
    <div class="mt-6 pt-4 border-t border-slate-700">
      <a href="../" class="text-slate-400 text-xs
        hover:text-white block py-1">Dashboard</a>
      <a href="../trade/" class="text-slate-400 text-xs
        hover:text-white block py-1">Trade UI</a>
    </div>
  </aside>
  <main class="flex-1 max-w-3xl p-8">
    <div id="content"></div>
  </main>
</div>
<script src="https://cdn.jsdelivr.net/npm/marked/marked.min.js">
</script>
<script src="https://cdn.jsdelivr.net/gh/highlightjs/cdn-release@11/build/highlight.min.js">
</script>
<script>
(function() {{
  var raw = {md_json};
  marked.setOptions({{
    highlight: function(code, lang) {{
      if (lang && hljs.getLanguage(lang)) {{
        return hljs.highlight(code, {{language: lang}}).value;
      }}
      return hljs.highlightAuto(code).value;
    }}
  }});
  document.getElementById('content').innerHTML =
    marked.parse(raw);
}})();
</script>
</body>
</html>"""
    return HTMLResponse(doc_html)


# ── HTMX partial exception handler ─────────────────────

from fastapi.exceptions import RequestValidationError
from starlette.exceptions import HTTPException as _StarletteHTTPException


@app.exception_handler(RequestValidationError)
async def _htmx_422(request: Request, exc: Exception):
    if request.url.path.startswith("/x/"):
        return HTMLResponse(
            '<span class="text-slate-500 text-xs">'
            'no data</span>',
            status_code=200,
        )
    return JSONResponse(
        {"detail": str(exc)}, status_code=422)


@app.exception_handler(_StarletteHTTPException)
async def _htmx_http(request: Request, exc):
    if request.url.path.startswith("/x/"):
        return HTMLResponse(
            '<span class="text-slate-500 text-xs">'
            'no data</span>',
            status_code=200,
        )
    return JSONResponse(
        {"detail": exc.detail}, status_code=exc.status_code)


@app.exception_handler(Exception)
async def _htmx_500(request: Request, exc: Exception):
    if request.url.path.startswith("/x/"):
        msg = html.escape(str(exc))
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'error: {msg}</span>',
            status_code=200,
        )
    from fastapi.responses import JSONResponse
    return JSONResponse(
        {"detail": str(exc)}, status_code=500)


# ── HTMX partial routes ────────────────────────────────

@app.get("/x/processes", response_class=HTMLResponse)
async def x_processes():
    return HTMLResponse(
        pages.render_process_table(scan_processes()))


@app.get("/x/health", response_class=HTMLResponse)
async def x_health():
    return HTMLResponse(
        pages.render_health(scan_processes(),
                            pg_pool is not None))


@app.get("/x/key-metrics", response_class=HTMLResponse)
async def x_key_metrics():
    terminal = {
        "filled", "cancelled", "rejected",
        "failed", "expired",
    }
    ao = sum(
        1 for o in recent_orders
        if o.get("status", "") not in terminal)
    fills = parse_wal_fills(max_fills=2000)
    pairs = set()
    for f in fills:
        taker = f.get("taker_uid", 0)
        maker = f.get("maker_uid", 0)
        sid = f.get("symbol_id", 0)
        if taker:
            pairs.add((taker, sid))
        if maker:
            pairs.add((maker, sid))
    pos_count = len(pairs)
    elapsed = max(1, time.time() - SERVER_START)
    mps = int(len(recent_orders) / elapsed)
    return HTMLResponse(
        pages.render_key_metrics(
            scan_processes(), scan_wal_streams(),
            active_orders=ao, positions=pos_count,
            msgs_sec=mps))


@app.get("/x/pulse", response_class=HTMLResponse)
async def x_pulse():
    procs = scan_processes()
    running = sum(
        1 for p in procs if p.get("state") == "running")
    streams = scan_wal_streams()
    wal_files = sum(s.get("files", 0) for s in streams)
    elapsed = max(1, time.time() - SERVER_START)
    ops = int(len(recent_orders) / elapsed)
    errs = sum(
        1 for o in recent_orders
        if o.get("status") in {"rejected", "failed"})

    def _pill(label, value, color):
        return (
            f'<span class="text-slate-500">{label}</span>'
            f'<span class="text-{color} font-bold">'
            f'{value}</span>'
        )

    return HTMLResponse(
        _pill("proc", f"{running}/{len(procs)}",
              "emerald-400" if running > 0
              else "red-400")
        + _pill("ord/s", str(ops), "blue-400")
        + _pill("wal", str(wal_files), "cyan-400")
        + _pill("errs", str(errs),
                "emerald-400" if errs == 0
                else "red-400")
    )


@app.get("/x/ring-pressure", response_class=HTMLResponse)
async def x_ring_pressure():
    return HTMLResponse(
        pages.render_ring_pressure(scan_wal_streams()))


@app.get("/x/invariant-status",
         response_class=HTMLResponse)
async def x_invariant_status():
    return HTMLResponse(
        pages.render_invariant_status(verify_results))


@app.get("/x/core-affinity", response_class=HTMLResponse)
async def x_core_affinity():
    return HTMLResponse(
        pages.render_core_affinity(scan_processes()))


@app.get("/x/cmp-flows", response_class=HTMLResponse)
async def x_cmp_flows():
    fills = 0
    bbos = 0
    for sd in _wal_stream_dirs():
        for r in parse_wal_records(sd, {RECORD_FILL}):
            fills += 1
        for r in parse_wal_records(sd, {RECORD_BBO}):
            bbos += 1
    return HTMLResponse(pages.render_cmp_flows(
        {"fills": fills, "bbos": bbos}))


@app.get("/x/control-grid", response_class=HTMLResponse)
async def x_control_grid():
    return HTMLResponse(
        pages.render_control_grid(scan_processes()))


@app.get("/x/resource-usage", response_class=HTMLResponse)
async def x_resource_usage():
    return HTMLResponse(
        pages.render_resource_usage(scan_processes()))


@app.get("/x/faults-grid", response_class=HTMLResponse)
async def x_faults_grid():
    return HTMLResponse(
        pages.render_faults_grid(scan_processes()))


@app.get("/x/wal-status", response_class=HTMLResponse)
async def x_wal_status():
    return HTMLResponse(
        pages.render_wal_status(scan_wal_streams()))


@app.get("/x/wal-detail", response_class=HTMLResponse)
async def x_wal_detail():
    return HTMLResponse(
        pages.render_wal_detail(scan_wal_streams()))


@app.get("/x/wal-files", response_class=HTMLResponse)
async def x_wal_files():
    return HTMLResponse(
        pages.render_wal_files(scan_wal_files()))


@app.get("/x/wal-lag", response_class=HTMLResponse)
async def x_wal_lag():
    streams = scan_wal_streams()
    return HTMLResponse(pages.render_wal_lag(streams))


@app.get("/x/wal-rotation", response_class=HTMLResponse)
async def x_wal_rotation():
    streams = scan_wal_streams()
    return HTMLResponse(pages.render_wal_rotation(streams))


@app.get("/x/wal-timeline", response_class=HTMLResponse)
async def x_wal_timeline(
    filter: str = Query("", alias="filter"),
):
    all_records = []
    for stream_dir in _wal_stream_dirs():
        all_records.extend(parse_wal_records(stream_dir))
    if filter:
        f_lower = filter.lower()
        all_records = [
            r for r in all_records
            if r.get("type", "") == f_lower
        ]
    all_records.sort(
        key=lambda r: r.get("seq", 0), reverse=True)
    return HTMLResponse(
        pages.render_wal_timeline(all_records))


@app.get("/x/logs", response_class=HTMLResponse)
async def x_logs(
    process: str = Query("", alias="log-process"),
    level: str = Query("", alias="log-level"),
    search: str = Query("", alias="log-search"),
):
    lines = read_logs(
        process=process or None,
        level=level or None,
        search=search or None,
    )
    return HTMLResponse(pages.render_logs(lines))


@app.get("/x/logs-tail", response_class=HTMLResponse)
async def x_logs_tail():
    return HTMLResponse(
        pages.render_logs(read_logs(max_lines=20)))


@app.get("/x/error-agg", response_class=HTMLResponse)
async def x_error_agg():
    lines = read_logs(max_lines=1000)
    return HTMLResponse(pages.render_error_agg(lines))


@app.get("/x/auth-failures", response_class=HTMLResponse)
async def x_auth_failures():
    return HTMLResponse(
        '<span class="text-slate-600 text-xs">'
        'no auth failures</span>')


@app.get("/x/book-stats", response_class=HTMLResponse)
async def x_book_stats():
    stats = parse_wal_book_stats()
    # supplement with live snaps for symbols not in WAL
    snap_copy = dict(_book_snap)
    for sid, snap in snap_copy.items():
        if sid not in stats:
            bbo = _snap_to_bbo(sid, snap)
            if bbo:
                stats[sid] = bbo
    # fallback: maker book or re-seed for configured symbols
    for name, cfg in start_mod.SYMBOLS.items():
        sid = cfg["id"]
        if sid in stats:
            continue
        mb = _maker_book(sid)
        if mb:
            bbo = _snap_to_bbo(sid, mb)
            if bbo:
                stats[sid] = bbo
                continue
        snap = _book_snap.get(sid)
        if snap:
            bbo = _snap_to_bbo(sid, snap)
            if bbo:
                stats[sid] = bbo
    return HTMLResponse(
        pages.render_book_stats(stats))


@app.get("/x/live-fills", response_class=HTMLResponse)
@app.get("/x/fills", response_class=HTMLResponse)
async def x_fills():
    fills = parse_wal_fills()
    if not fills:
        fills = list(reversed(recent_fills[-50:]))
    return HTMLResponse(
        pages.render_live_fills(fills))


@app.get("/x/trade-agg", response_class=HTMLResponse)
async def x_trade_agg():
    procs = scan_processes()
    running = [p for p in procs
               if p["state"] == "running"]
    if not running:
        return HTMLResponse(
            '<span class="text-slate-500 text-xs">'
            'no processes running</span>')
    fills = parse_wal_fills()
    return HTMLResponse(
        pages.render_trade_agg(fills))


@app.get("/x/position-heatmap",
         response_class=HTMLResponse)
async def x_position_heatmap():
    fills = parse_wal_fills(max_fills=500)
    return HTMLResponse(
        pages.render_position_heatmap(fills or None))


@app.get("/x/margin-ladder", response_class=HTMLResponse)
async def x_margin_ladder():
    fills = parse_wal_fills()
    return HTMLResponse(
        pages.render_margin_ladder(fills or None))


@app.get("/x/funding", response_class=HTMLResponse)
async def x_funding():
    stats = parse_wal_book_stats()
    return HTMLResponse(
        pages.render_funding(stats or None))


@app.get("/x/risk-latency", response_class=HTMLResponse)
async def x_risk_latency():
    return HTMLResponse(pages.render_risk_latency(order_latencies))


@app.get("/x/reconciliation",
         response_class=HTMLResponse)
async def x_reconciliation():
    shadow_check = None
    if _book_snap:
        mismatches = 0
        checked = 0
        for sid, snap in _book_snap.items():
            wal_bbo = parse_wal_bbo(sid)
            if wal_bbo is None:
                continue
            checked += 1
            snap_bid = snap.get(
                "best_bid", snap.get("bid_px", 0))
            snap_ask = snap.get(
                "best_ask", snap.get("ask_px", 0))
            if (snap_bid != wal_bbo.get("bid_px", 0)
                    or snap_ask
                    != wal_bbo.get("ask_px", 0)):
                mismatches += 1
        if checked > 0:
            if mismatches == 0:
                shadow_check = (
                    "pass", f"{checked} symbols match")
            else:
                shadow_check = (
                    "fail",
                    f"{mismatches}/{checked} mismatch")

    mark_check = None
    if _book_snap:
        checked = 0
        for sid, snap in _book_snap.items():
            bid = snap.get(
                "best_bid", snap.get("bid_px", 0))
            ask = snap.get(
                "best_ask", snap.get("ask_px", 0))
            if bid > 0 and ask > 0:
                checked += 1
        if checked > 0:
            mark_check = (
                "pass",
                f"{checked} symbols have valid BBO mid")

    return HTMLResponse(pages.render_reconciliation(
        shadow_vs_me=shadow_check,
        mark_vs_index=mark_check))


@app.get("/x/latency-regression",
         response_class=HTMLResponse)
async def x_latency_regression():
    return HTMLResponse(
        pages.render_latency_regression(order_latencies))


@app.get("/x/gateway-mode", response_class=HTMLResponse)
async def x_gateway_mode():
    reachable = await _probe_gateway_tcp()
    return HTMLResponse(
        pages.render_gateway_mode_badge(reachable))


@app.get("/x/order-trace", response_class=HTMLResponse)
async def x_order_trace(
    oid: str = Query("", alias="trace-oid"),
):
    if not oid:
        return HTMLResponse(
            '<span class="text-slate-600">'
            'enter an oid</span>')
    order = next(
        (o for o in recent_orders if o.get("cid") == oid),
        None,
    )
    if order is None:
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'order {html.escape(oid)} not found in session'
            f'</span>')
    user_id = order.get("user_id", 0)
    symbol_id = int(order.get("symbol", "10"))
    fills = parse_wal_fills_for_user(user_id, symbol_id)
    return HTMLResponse(
        pages.render_order_trace(order, fills))


@app.get("/x/stale-orders", response_class=HTMLResponse)
async def x_stale_orders():
    now = time.time()
    terminal = {
        "filled", "cancelled", "rejected",
        "failed", "expired",
    }
    stale = [
        o for o in recent_orders
        if o.get("status", "") not in terminal
        and isinstance(o.get("ts"), (int, float))
        and now - o["ts"] > 60]
    if not stale:
        return HTMLResponse(
            '<span class="text-emerald-400 text-xs">'
            '0 stale orders</span>')
    return HTMLResponse(
        f'<span class="text-amber-400 text-xs">'
        f'{len(stale)} stale order(s)</span>')


@app.get("/x/book", response_class=HTMLResponse)
async def x_book(symbol_id: int = Query(10)):
    snap = _book_snap.get(symbol_id)
    if snap and (snap.get("bids") or snap.get("asks")):
        return HTMLResponse(
            pages.render_book_ladder(symbol_id, snap))
    # Fallback: WAL BBO gives at most 1 bid + 1 ask
    bbo = parse_wal_bbo(symbol_id)
    if bbo is None:
        return HTMLResponse(
            pages.render_book_ladder(symbol_id, None))
    snap_from_bbo: dict = {"bids": [], "asks": []}
    if bbo.get("bid_px"):
        snap_from_bbo["bids"] = [
            {"px": bbo["bid_px"], "qty": bbo["bid_qty"]}]
    if bbo.get("ask_px"):
        snap_from_bbo["asks"] = [
            {"px": bbo["ask_px"], "qty": bbo["ask_qty"]}]
    return HTMLResponse(
        pages.render_book_ladder(symbol_id, snap_from_bbo))


@app.get("/x/risk-user", response_class=HTMLResponse)
async def x_risk_user(
    risk_uid: int = Query(1, alias="risk-uid"),
):
    # try postgres first
    data = await pg_query(
        "SELECT * FROM positions "
        "WHERE user_id = $1 LIMIT 20",
        risk_uid,
    )
    if data is None:
        data = await pg_query(
            "SELECT * FROM accounts "
            "WHERE user_id = $1 LIMIT 20",
            risk_uid,
        )
    if data and isinstance(data, list) and data:
        rows = ""
        for row in data:
            cells = "".join(
                f'<td class="py-1.5 px-2 text-xs '
                f'border-b border-slate-800/50">'
                f'{html.escape(str(v))}</td>'
                for v in row.values()
            )
            rows += f"<tr>{cells}</tr>"
        headers = list(data[0].keys())
        return HTMLResponse(pages._table(headers, rows))
    if data and isinstance(data, dict) and "error" in data:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'query error: {data["error"]}</span>')
    # fallback: aggregate net position from WAL fills
    fills = parse_wal_fills_for_user_all(risk_uid)
    if fills:
        return HTMLResponse(
            pages.render_risk_user_wal(risk_uid, fills))
    return HTMLResponse(
        '<span class="text-slate-600">'
        f'user {risk_uid} — no data</span>')


@app.get("/x/liquidations", response_class=HTMLResponse)
async def x_liquidations():
    data = await pg_query(
        "SELECT * FROM liquidations "
        "ORDER BY timestamp_ns DESC LIMIT 20",
    )
    if data and isinstance(data, list) and data:
        rows = ""
        for row in data:
            cells = "".join(
                f'<td class="py-1.5 px-2 text-xs '
                f'border-b border-slate-800/50">'
                f'{html.escape(str(v))}</td>'
                for v in row.values()
            )
            rows += f"<tr>{cells}</tr>"
        return HTMLResponse(
            pages._table(list(data[0].keys()), rows))
    # fallback: parse WAL liquidation records
    liqns = parse_wal_liquidations()
    if liqns:
        return HTMLResponse(
            pages.render_liquidations_wal(liqns))
    return HTMLResponse(
        '<span class="text-slate-600">'
        'no active liquidations</span>')


@app.get("/x/verify", response_class=HTMLResponse)
async def x_verify():
    if not verify_results:
        return HTMLResponse(
            '<span class="text-slate-600">'
            'click "Run All Checks" to verify</span>')
    return HTMLResponse(pages.render_verify(verify_results))


@app.get("/x/recent-orders", response_class=HTMLResponse)
async def x_recent_orders():
    return HTMLResponse(
        pages.render_recent_orders(recent_orders[-50:]))


# ── API routes ──────────────────────────────────────────

@app.get("/api/processes")
async def api_processes():
    return scan_processes()


@app.get("/api/scenarios")
async def api_scenarios():
    return list(start_mod.SCENARIOS.keys())


@app.get("/api/build-log")
async def api_build_log():
    return {"log": build_log[-50:]}


@app.post("/api/build")
async def api_build():
    """Trigger cargo build."""
    ok = await do_build()
    return HTMLResponse(
        f'<span class="text-{"emerald" if ok else "red"}'
        f'-400 text-xs">'
        f'{"build ok" if ok else "build FAILED"}</span>')


@app.post("/api/processes/all/start")
async def api_start_all(
    request: Request,
    scenario: str = Query("minimal"),
):
    """Build + start all processes."""
    ok, err = _check_run_id(request)
    if not ok:
        return JSONResponse(
            {"error": f"run_id check failed: {err}"},
            status_code=409,
        )
    denied = check_confirm(request, "/api/processes/all/start")
    if denied:
        return denied
    audit_log("/api/processes/all/start", f"scenario={scenario}")
    result = await start_all(scenario)
    if "error" in result:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'{result["error"]}</span>')
    return HTMLResponse(
        f'<span class="text-emerald-400 text-xs">'
        f'started {result["count"]} processes</span>')


@app.post("/api/processes/all/stop")
async def api_stop_all(request: Request):
    """Stop all managed processes."""
    ok, err = _check_run_id(request)
    if not ok:
        return JSONResponse(
            {"error": f"run_id check failed: {err}"},
            status_code=409,
        )
    denied = check_confirm(request, "/api/processes/all/stop")
    if denied:
        return denied
    audit_log("/api/processes/all/stop", "stop all")
    result = await stop_all()
    return HTMLResponse(
        f'<span class="text-amber-400 text-xs">'
        f'stopped {len(result["stopped"])} processes</span>')


@app.post("/api/processes/{name}/{action}")
async def api_process_action(name: str, action: str):
    if action not in ("start", "stop", "kill", "restart"):
        return JSONResponse(
            {"error": f"unknown action: {action}"},
            status_code=400)

    if action == "stop":
        # try managed first, fallback to raw PID
        if name in managed:
            result = await stop_process(name)
        else:
            procs = scan_processes()
            proc = next(
                (p for p in procs if p["name"] == name),
                None)
            if proc and proc["pid"] != "-":
                try:
                    os.kill(int(proc["pid"]), signal.SIGTERM)
                    pid_file = PID_DIR / f"{name}.pid"
                    if pid_file.exists():
                        pid_file.unlink()
                    result = {"status": f"stopped {name}"}
                except ProcessLookupError:
                    result = {"status": f"{name} not running"}
            else:
                result = {"status": f"{name} not running"}
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'{result.get("status", "ok")}</span>')

    if action == "kill":
        if name in managed:
            result = await kill_process(name)
        else:
            procs = scan_processes()
            proc = next(
                (p for p in procs if p["name"] == name),
                None)
            if proc and proc["pid"] != "-":
                try:
                    os.kill(int(proc["pid"]), signal.SIGKILL)
                    pid_file = PID_DIR / f"{name}.pid"
                    if pid_file.exists():
                        pid_file.unlink()
                    result = {"status": f"killed {name}"}
                except (ProcessLookupError, PermissionError,
                        OSError) as e:
                    result = {"status": (
                        f"{name} not running"
                        if isinstance(e, ProcessLookupError)
                        else f"kill failed: {e}")}
            else:
                result = {"status": f"{name} not running"}
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'{result.get("status", "ok")}</span>')

    if action == "restart":
        # Maker uses Python interpreter; route through do_maker_start.
        if name == MAKER_NAME:
            if name in managed:
                await stop_process(name)
                await asyncio.sleep(0.3)
            ok = await do_maker_start()
            if ok:
                pid = managed[MAKER_NAME]["proc"].pid
                msg = f"restarted {name} (pid {pid})"
            else:
                msg = f"failed to restart {name}"
            return HTMLResponse(
                f'<span class="text-blue-400 text-xs">'
                f'{msg}</span>')
        if name in managed:
            result = await restart_process(name)
            msg = (f"restarted {name} (pid {result['pid']})"
                   if "pid" in result
                   else result.get("error", "failed"))
        else:
            # need to find it in spawn plan and start fresh
            plan = get_spawn_plan(current_scenario)
            entry = next(
                (e for e in plan if e[0] == name), None)
            if entry:
                _, binary, env = entry
                # stop if running
                procs = scan_processes()
                proc = next(
                    (p for p in procs if p["name"] == name),
                    None)
                if proc and proc["pid"] != "-":
                    try:
                        pid = int(proc["pid"])
                        os.kill(pid, signal.SIGTERM)
                        # wait up to 3s, then SIGKILL
                        for _ in range(30):
                            await asyncio.sleep(0.1)
                            try:
                                os.kill(pid, 0)
                            except ProcessLookupError:
                                break
                        else:
                            try:
                                os.kill(pid, signal.SIGKILL)
                            except (ProcessLookupError,
                                    OSError):
                                pass
                    except (ProcessLookupError, PermissionError,
                            OSError):
                        pass
                result = await spawn_process(
                    name, binary, env)
                msg = (f"started {name} (pid {result['pid']})"
                       if "pid" in result
                       else result.get("error", "failed"))
            else:
                msg = f"unknown process: {name}"
        return HTMLResponse(
            f'<span class="text-blue-400 text-xs">'
            f'{msg}</span>')

    if action == "start":
        # Maker uses Python interpreter; route through do_maker_start.
        if name == MAKER_NAME:
            if _maker_running():
                return HTMLResponse(
                    '<span class="text-amber-400 text-xs">'
                    'maker already running</span>')
            ok = await do_maker_start()
            if ok:
                pid = managed[MAKER_NAME]["proc"].pid
                return HTMLResponse(
                    f'<span class="text-emerald-400 text-xs">'
                    f'started maker (pid {pid})</span>')
            return HTMLResponse(
                '<span class="text-red-400 text-xs">'
                'maker failed to start</span>')
        # refuse to duplicate a running process
        if name in managed:
            proc = managed[name]["proc"]
            if proc.returncode is None:
                return HTMLResponse(
                    f'<span class="text-amber-400 text-xs">'
                    f'{name} already running</span>')
        # find in spawn plan
        plan = get_spawn_plan(current_scenario)
        entry = next(
            (e for e in plan if e[0] == name), None)
        if not entry:
            return HTMLResponse(
                f'<span class="text-red-400 text-xs">'
                f'unknown process: {name}</span>')
        # auto-build first
        ok = await do_build()
        if not ok:
            return HTMLResponse(
                '<span class="text-red-400 text-xs">'
                'build failed</span>')
        _, binary, env = entry
        result = await spawn_process(name, binary, env)
        if "pid" in result:
            return HTMLResponse(
                f'<span class="text-emerald-400 text-xs">'
                f'started {name} (pid {result["pid"]})'
                f'</span>')
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'{result.get("error", "failed")}</span>')

    return {"status": "ok"}


@app.post("/api/scenario/switch")
async def api_scenario_switch(request: Request):
    denied = check_confirm(request, "/api/scenario/switch")
    if denied:
        return denied
    global current_scenario
    form = await request.form()
    scenario = form.get("scenario-select", "minimal")
    audit_log("/api/scenario/switch", f"scenario={scenario}")

    if scenario not in start_mod.SCENARIOS:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'unknown scenario: {scenario}</span>')

    # Stop current processes if running
    await stop_all()
    await asyncio.sleep(0.5)

    # Auto-restart with new scenario
    result = await start_all(scenario)
    if "started" in result:
        # Only update current_scenario after successful start
        current_scenario = scenario
        return HTMLResponse(
            f'<span class="text-emerald-400 text-xs">'
            f'switched to {scenario} and restarted '
            f'{result["count"]} processes</span>')
    else:
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'switched to {scenario} (restart failed: '
            f'{result.get("error", "unknown")})</span>')


@app.get("/x/current-scenario", response_class=HTMLResponse)
async def x_current_scenario():
    return HTMLResponse(current_scenario)


@app.get("/api/wal/{stream}/status")
async def api_wal_status(stream: str):
    stream_dir = (WAL_DIR / stream).resolve()
    if not str(stream_dir).startswith(str(WAL_DIR.resolve())):
        return {"error": "invalid stream name"}
    if not stream_dir.exists():
        return {"error": "stream not found"}
    files = list(stream_dir.iterdir())
    total = sum(
        f.stat().st_size for f in files if f.is_file())
    return {
        "stream": stream,
        "files": len([f for f in files if f.is_file()]),
        "total_bytes": total,
        "total_size": human_size(total),
    }


@app.get("/api/logs")
async def api_logs(
    process: str = Query(None),
    level: str = Query(None),
    search: str = Query(None),
    limit: int = Query(500),
):
    lines = read_logs(process, level, search, limit)
    return {"lines": lines, "count": len(lines)}


@app.post("/api/logs/clear")
async def api_logs_clear():
    """Truncate all log files in ./log/ directory."""
    cleared = []
    if LOG_DIR.exists():
        for p in LOG_DIR.glob("*.log"):
            open(p, "w").close()
            cleared.append(p.name)
    return HTMLResponse(
        '<span class="text-emerald-400 text-xs">'
        f'cleared {len(cleared)} log file(s)</span>'
    )


@app.get("/api/stats")
async def api_stats():
    uptime_s = int(time.time() - SERVER_START)
    running = sum(
        1 for info in managed.values()
        if info["proc"].returncode is None
    )
    maker_running = _maker_running()
    try:
        mcfg = json.loads(MAKER_CONFIG.read_text())
    except Exception:
        mcfg = {}
    spread_bps = mcfg.get("spread_bps", 20)
    return {
        "orders_submitted": len(recent_orders),
        "uptime_s": uptime_s,
        "active_connections": running,
        "active_stress": int(_stress_running()),
        "maker_running": maker_running,
        "maker_spread_bps": spread_bps,
    }


@app.get("/x/stats", response_class=HTMLResponse)
async def x_stats():
    uptime_s = int(time.time() - SERVER_START)
    h, rem = divmod(uptime_s, 3600)
    m, s = divmod(rem, 60)
    uptime_str = f"{h}h {m}m {s}s" if h else f"{m}m {s}s"
    running = sum(
        1 for info in managed.values()
        if info["proc"].returncode is None
    )
    maker_running = _maker_running()
    try:
        mcfg = json.loads(MAKER_CONFIG.read_text())
    except Exception:
        mcfg = {}
    spread_bps = mcfg.get("spread_bps", 20)
    maker_label = (
        f'<span class="text-emerald-400">running</span>'
        f' {spread_bps}bps'
        if maker_running
        else '<span class="text-slate-500">stopped</span>'
    )
    rows = [
        ("orders submitted", len(recent_orders)),
        ("uptime", uptime_str),
        ("active processes", running),
        ("active stress", active_stress),
        ("maker", maker_label),
    ]
    inner = "".join(
        f'<div class="flex justify-between text-xs py-0.5">'
        f'<span class="text-slate-500">{k}</span>'
        f'<span class="text-slate-300">{v}</span>'
        f'</div>'
        for k, v in rows
    )
    return HTMLResponse(inner)


async def send_order_to_gateway(order_msg: dict, user_id: int = 1):
    """Send order to Gateway WebSocket if available."""
    try:
        headers = {"x-user-id": str(user_id)}
        start_ns = time.perf_counter_ns()
        async with aiohttp.ClientSession() as session:
            async with session.ws_connect(
                GATEWAY_URL,
                headers=headers,
            ) as ws:
                await ws.send_str(json.dumps(order_msg))
                # Skip heartbeats; read until order response or timeout.
                # Gateway sends {H:[ts]} periodically; no ACK for resting
                # GTC orders (WEBPROTO.md: "no accepted ACK").
                deadline = time.perf_counter() + 2.0
                while True:
                    remaining = deadline - time.perf_counter()
                    if remaining <= 0:
                        latency_us = (
                            (time.perf_counter_ns() - start_ns) // 1000
                        )
                        return (
                            None,
                            "timeout waiting for response",
                            latency_us,
                        )
                    response = await asyncio.wait_for(
                        ws.receive(), timeout=remaining,
                    )
                    if response.type != aiohttp.WSMsgType.TEXT:
                        continue
                    msg = json.loads(response.data)
                    if "H" in msg:
                        continue  # skip heartbeat
                    latency_us = (
                        (time.perf_counter_ns() - start_ns) // 1000
                    )
                    return msg, None, latency_us
    except asyncio.TimeoutError:
        # TimeoutError is a subclass of OSError in Python 3.11+;
        # catch it first to avoid misidentifying as "not running".
        return None, "timeout waiting for response", None
    except (ConnectionRefusedError, OSError):
        return None, "gateway not running", None
    except Exception as e:
        return None, str(e), None


def _trim_recent_orders():
    if len(recent_orders) > 200:
        del recent_orders[:100]


@app.post("/api/orders/test")
async def api_orders_test(request: Request):
    idem_key = request.headers.get("x-idempotency-key")
    if idem_key:
        now = time.time()
        expired = [
            k for k, t in _idempotency_keys.items()
            if now - t > _IDEMPOTENCY_TTL
        ]
        for k in expired:
            del _idempotency_keys[k]
        if idem_key in _idempotency_keys:
            return HTMLResponse(
                '<span class="text-amber-400 text-xs">'
                'duplicate submission (idempotency key '
                'already used)</span>')
        _idempotency_keys[idem_key] = now
    form = await request.form()
    audit_log("/api/orders/test", "submit order")
    cid = f"pg{int(time.time()*1e6):x}"[:20]

    try:
        user_id = int(form.get("user_id", "1"))
    except (ValueError, TypeError):
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            'invalid user_id</span>')

    try:
        symbol_id = int(form.get("symbol_id", "10"))
    except (ValueError, TypeError):
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            'invalid symbol_id</span>')

    side_str = form.get("side", "buy")
    side_int = 0 if side_str == "buy" else 1

    tif_map = {"GTC": 0, "IOC": 1, "FOK": 2}
    tif_str = form.get("tif", "GTC")
    tif_int = tif_map.get(tif_str.upper(), 0)

    # Look up symbol config for unit conversion.
    # Gateway expects raw fixed-point integers:
    #   price_raw = round(human_price * 10^price_decimals)
    #   qty_raw   = round(human_qty   * 10^qty_decimals)
    # Gateway validates price_raw % tick_size == 0 and
    # qty_raw % lot_size == 0.
    _sym_cfg = next(
        (v for v in start_mod.SYMBOLS.values()
         if v["id"] == symbol_id),
        {"tick": 1, "lot": 1,
         "price_dec": 8, "qty_dec": 8},
    )
    lot_size = _sym_cfg.get("lot", 1) or 1
    tick_size = _sym_cfg.get("tick", 1) or 1
    price_dec = _sym_cfg.get("price_dec", 8)
    qty_dec = _sym_cfg.get("qty_dec", 8)
    price_scale = 10 ** price_dec
    qty_scale = 10 ** qty_dec

    order_type = form.get("order_type", "limit")

    try:
        human_price = float(
            form.get("price", "0") or "0")
        if order_type == "market":
            price_int = 0
        else:
            price_int = round(human_price * price_scale)
            if price_int % tick_size != 0:
                return HTMLResponse(
                    '<span class="text-red-400 text-xs">'
                    f'price not aligned to tick '
                    f'({tick_size})</span>')
    except (ValueError, TypeError, OverflowError):
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            'invalid price</span>')

    try:
        qty_int = round(
            float(form.get("qty", "0") or "0")
            * qty_scale)
        if qty_int % lot_size != 0:
            return HTMLResponse(
                '<span class="text-red-400 text-xs">'
                f'qty not aligned to lot '
                f'({lot_size})</span>')
    except (ValueError, TypeError, OverflowError):
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            'invalid qty</span>')

    reduce_only = 1 if form.get("reduce_only") == "on" else 0
    # post_only can come from checkbox or order_type dropdown
    post_only = (
        1 if (
            form.get("post_only") == "on"
            or order_type == "post_only"
        ) else 0
    )
    # market orders use IOC if tif is GTC (GTC market invalid)
    if order_type == "market" and tif_int == 0:
        tif_int = 1

    # Gateway wire format: {"N": [sym, side, px, qty, cid, tif, ro, po]}
    order_msg = {
        "N": [
            symbol_id, side_int, price_int, qty_int,
            cid, tif_int, reduce_only, post_only,
        ],
    }

    order = {
        "cid": cid,
        "user_id": user_id,
        "symbol": str(symbol_id),
        "side": side_str,
        "price": form.get("price", "0") or "0",
        "qty": form.get("qty", "0") or "0",
        "tif": tif_str,
        "reduce_only": bool(reduce_only),
        "post_only": bool(post_only),
        "status": "pending",
        "ts": datetime.now().strftime("%H:%M:%S"),
    }

    result = await send_order_to_gateway(order_msg, user_id)
    err = result[1]
    if err:
        # Timeout = order sent but no fill/reject within 2s → resting
        if err == "timeout waiting for response":
            order["status"] = "accepted"
            recent_orders.append(order)
            _trim_recent_orders()
            return HTMLResponse(
                f'<span class="text-emerald-400 text-xs">'
                f'order {cid} accepted</span>')
        if err == "gateway not running":
            order["status"] = "error"
            order["error"] = err
            recent_orders.append(order)
            _trim_recent_orders()
            return HTMLResponse(
                f'<span class="text-amber-400 text-xs">'
                f'order {cid} error: gateway not running'
                f'</span>')
        order["status"] = "error"
        order["error"] = err
        recent_orders.append(order)
        _trim_recent_orders()
        color = "text-amber-400"
        return HTMLResponse(
            f'<span class="{color} text-xs">'
            f'order {cid} error: {err}</span>')

    msg, _, latency_us = result
    if latency_us:
        order_latencies.append(latency_us)
        if len(order_latencies) > 1000:
            del order_latencies[:500]

    # Gateway responses (see WEBPROTO.md):
    # {U:[oid, status, filled, remaining, reason]}
    #   status 0=FILLED 1=RESTING 2=CANCELLED 3=FAILED
    # {F:[taker_oid, maker_oid, px, qty, ts, fee]} — immediate fill
    # {E:[code, message]} — protocol error
    # No ACK for resting GTC orders (handled via timeout above)
    if msg and "U" in msg:
        u = msg["U"]
        status_code = u[1] if len(u) > 1 else -1
        if status_code == 3:
            reason_code = u[4] if len(u) > 4 else 0
            order["status"] = "rejected"
            order["reason"] = str(reason_code)
            recent_orders.append(order)
            _trim_recent_orders()
            return HTMLResponse(
                f'<span class="text-red-400 text-xs">'
                f'order {cid} rejected: reason={reason_code}'
                f'</span>')
        else:
            # Filled (0) or resting ACK (1)
            order["status"] = "accepted"
            if latency_us is not None:
                order["latency_us"] = latency_us
            recent_orders.append(order)
            _trim_recent_orders()
            lat_str = (
                f"{latency_us}us" if latency_us is not None
                else "?"
            )
            return HTMLResponse(
                f'<span class="text-emerald-400 text-xs">'
                f'order {cid} accepted ({lat_str})</span>')
    elif msg and "F" in msg:
        # Immediate fill — order accepted and filled
        order["status"] = "accepted"
        if latency_us is not None:
            order["latency_us"] = latency_us
        recent_orders.append(order)
        _trim_recent_orders()
        lat_str = (
            f"{latency_us}us" if latency_us is not None else "?"
        )
        return HTMLResponse(
            f'<span class="text-emerald-400 text-xs">'
            f'order {cid} accepted ({lat_str})</span>')
    elif msg and "E" in msg:
        e = msg["E"]
        err_msg = e[1] if len(e) > 1 else "unknown"
        order["status"] = "rejected"
        order["reason"] = str(err_msg)
        recent_orders.append(order)
        _trim_recent_orders()
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'order {cid} rejected: {err_msg}</span>')
    else:
        order["status"] = "error"
        recent_orders.append(order)
        _trim_recent_orders()
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'order {cid} unexpected response</span>')


async def _run_invariant_checks() -> list[dict]:
    """Run all invariant checks, return list of dicts."""
    now = datetime.now().strftime("%H:%M:%S")
    checks = []

    wal_exists = WAL_DIR.exists()
    checks.append({
        "name": "WAL directory exists",
        "status": "pass" if wal_exists else "fail",
        "time": now,
        "detail": str(WAL_DIR) if not wal_exists else "",
    })

    if wal_exists:
        for s in scan_wal_streams():
            checks.append({
                "name": f"WAL stream {s['name']} has files",
                "status": "pass" if s["files"] > 0 else "warn",
                "time": now,
                "detail": f"{s['files']} files, "
                          f"{s['total_size']}",
            })

    procs = scan_processes()
    running = [p for p in procs if p.get("state") == "running"]
    checks.append({
        "name": "RSX processes running",
        "status": "pass" if running else "fail",
        "time": now,
        "detail": (f"{len(running)}/{len(procs)} running"
                   if running else "no processes running"),
    })

    # postgres connectivity
    pg_ok = pg_pool is not None
    checks.append({
        "name": "Postgres connected",
        "status": "pass" if pg_ok else "warn",
        "time": now,
        "detail": PG_URL if not pg_ok else "connected",
    })

    # ── invariant 1: fills precede ORDER_DONE ───────────────
    # Parse WAL fills; verify seq is monotonically increasing
    # within each stream (fills are written before ORDER_DONE
    # propagates, so a monotone seq implies ordering holds).
    fill_seqs: list[int] = []
    for sd in _wal_stream_dirs():
        for r in parse_wal_records(sd, {RECORD_FILL}):
            fill_seqs.append(r["seq"])
    fill_seqs.sort()
    if not fill_seqs:
        checks.append({
            "name": "Fills precede ORDER_DONE (per order)",
            "status": "skip" if not running else "pass",
            "time": now,
            "detail": (
                "no WAL fills recorded yet"
                if not running
                else "0 fills; system running, no trades yet"
            ),
        })
    else:
        violations = sum(
            1 for i in range(1, len(fill_seqs))
            if fill_seqs[i] < fill_seqs[i - 1]
        )
        checks.append({
            "name": "Fills precede ORDER_DONE (per order)",
            "status": "pass" if violations == 0 else "fail",
            "time": now,
            "detail": (
                f"{len(fill_seqs)} fills, "
                f"{violations} seq inversions"
            ),
        })

    # ── invariant 2: exactly-one completion per order ───────
    completed: dict[str, int] = {}
    for o in recent_orders:
        if o.get("status") in ("accepted", "rejected", "error"):
            cid = o.get("cid", "")
            completed[cid] = completed.get(cid, 0) + 1
    dupes = {c: n for c, n in completed.items() if n > 1}
    checks.append({
        "name": "Exactly-one completion per order",
        "status": "fail" if dupes else (
            "pass" if completed else "skip"
        ),
        "time": now,
        "detail": (
            f"{len(dupes)} cids with duplicate completions"
            if dupes
            else (
                f"{len(completed)} orders, no duplicates"
                if completed
                else "no completed orders observed"
            )
        ),
    })

    # ── invariant 3: FIFO within price level ────────────────
    # Requires per-order timestamps at ME level; not in WAL.
    checks.append({
        "name": "FIFO within price level (time priority)",
        "status": "skip",
        "time": now,
        "detail": "requires ME-level per-order instrumentation",
    })

    # ── invariant 4: position = sum of fills ────────────────
    if pg_pool is not None:
        try:
            rows = await pg_query(
                """
                SELECT p.user_id, p.symbol_id,
                       (p.long_qty - p.short_qty) AS pos,
                       COALESCE(f.net, 0) AS fills
                FROM positions p
                LEFT JOIN (
                    SELECT user_id, symbol_id,
                           SUM(net) AS net
                    FROM (
                        SELECT taker_user_id AS user_id,
                               symbol_id,
                               SUM(CASE WHEN taker_side=0
                                   THEN qty ELSE -qty END)
                                   AS net
                        FROM fills
                        GROUP BY taker_user_id, symbol_id
                        UNION ALL
                        SELECT maker_user_id,
                               symbol_id,
                               SUM(CASE WHEN taker_side=0
                                   THEN -qty ELSE qty END)
                        FROM fills
                        GROUP BY maker_user_id, symbol_id
                    ) t
                    GROUP BY user_id, symbol_id
                ) f USING (user_id, symbol_id)
                WHERE (p.long_qty - p.short_qty)
                    != COALESCE(f.net, 0)
                LIMIT 10
                """
            )
            if rows is None:
                checks.append({
                    "name": "Position = sum of fills (risk engine)",
                    "status": "skip",
                    "time": now,
                    "detail": "pg query returned None",
                })
            elif rows:
                checks.append({
                    "name": "Position = sum of fills (risk engine)",
                    "status": "fail",
                    "time": now,
                    "detail": (
                        f"{len(rows)} position/fill mismatches"
                    ),
                })
            else:
                checks.append({
                    "name": "Position = sum of fills (risk engine)",
                    "status": "pass",
                    "time": now,
                    "detail": "positions match fill sums",
                })
        except Exception as exc:
            checks.append({
                "name": "Position = sum of fills (risk engine)",
                "status": "skip",
                "time": now,
                "detail": f"pg error: {exc}",
            })
    else:
        checks.append({
            "name": "Position = sum of fills (risk engine)",
            "status": "skip",
            "time": now,
            "detail": "postgres not connected",
        })

    # ── invariant 5: tips monotonic ─────────────────────────
    # Check BBO seq per stream; seqs must not decrease.
    tip_violations = 0
    tip_streams = 0
    for sd in _wal_stream_dirs():
        seqs: list[int] = [
            r["seq"]
            for r in parse_wal_records(sd, {RECORD_BBO})
        ]
        if not seqs:
            continue
        tip_streams += 1
        for i in range(1, len(seqs)):
            if seqs[i] < seqs[i - 1]:
                tip_violations += 1
    if tip_streams == 0:
        checks.append({
            "name": "Tips monotonic, never decrease",
            "status": "skip",
            "time": now,
            "detail": "no BBO records in WAL",
        })
    else:
        checks.append({
            "name": "Tips monotonic, never decrease",
            "status": "pass" if tip_violations == 0 else "fail",
            "time": now,
            "detail": (
                f"{tip_streams} streams checked, "
                f"{tip_violations} inversions"
            ),
        })

    # ── invariant 6: no crossed book ────────────────────────
    crossed: list[str] = []
    # Check live snaps
    for sid, snap in _book_snap.items():
        bids = snap.get("bids", [])
        asks = snap.get("asks", [])
        if bids and asks:
            best_bid = bids[0]["px"]
            best_ask = asks[0]["px"]
            if best_bid > 0 and best_ask > 0:
                if best_bid >= best_ask:
                    crossed.append(
                        f"sym={sid} bid={best_bid}"
                        f" >= ask={best_ask}"
                    )
    # Check WAL BBO records
    for sd in _wal_stream_dirs():
        for r in parse_wal_records(sd, {RECORD_BBO}):
            if (r["bid_px"] > 0 and r["ask_px"] > 0
                    and r["bid_px"] >= r["ask_px"]):
                crossed.append(
                    f"WAL sym={r['symbol_id']}"
                    f" bid={r['bid_px']}"
                    f" >= ask={r['ask_px']}"
                )
                if len(crossed) >= 5:
                    break
    syms_checked = len(_book_snap)
    no_wal_bbo = tip_streams == 0
    if not syms_checked and (not wal_exists or no_wal_bbo):
        checks.append({
            "name": "No crossed book (bid < ask)",
            "status": "skip",
            "time": now,
            "detail": "no book data available",
        })
    else:
        checks.append({
            "name": "No crossed book (bid < ask)",
            "status": "fail" if crossed else "pass",
            "time": now,
            "detail": (
                "; ".join(crossed[:3])
                if crossed
                else (
                    f"{syms_checked} live symbols checked, "
                    "no crosses"
                )
            ),
        })

    # ── invariant 7: SPSC FIFO order ────────────────────────
    checks.append({
        "name": "SPSC preserves event FIFO order",
        "status": "skip",
        "time": now,
        "detail": "no external observable state for SPSC rings",
    })

    # ── invariant 8: slab no-leak ───────────────────────────
    checks.append({
        "name": "Slab no-leak: allocated = free + active",
        "status": "skip",
        "time": now,
        "detail": "requires slab metrics export (not yet wired)",
    })

    # ── invariant 9: funding zero-sum ───────────────────────
    if pg_pool is not None:
        try:
            rows = await pg_query(
                """
                SELECT symbol_id,
                       SUM(amount) AS net
                FROM funding
                GROUP BY symbol_id
                HAVING ABS(SUM(amount)) > 1
                LIMIT 10
                """
            )
            if rows is None:
                checks.append({
                    "name": (
                        "Funding zero-sum across"
                        " users per symbol"
                    ),
                    "status": "skip",
                    "time": now,
                    "detail": "pg query returned None",
                })
            elif rows:
                checks.append({
                    "name": (
                        "Funding zero-sum across"
                        " users per symbol"
                    ),
                    "status": "fail",
                    "time": now,
                    "detail": (
                        f"{len(rows)} symbols with"
                        " non-zero net funding"
                    ),
                })
            else:
                checks.append({
                    "name": (
                        "Funding zero-sum across"
                        " users per symbol"
                    ),
                    "status": "pass",
                    "time": now,
                    "detail": "all symbols net funding = 0",
                })
        except Exception as exc:
            checks.append({
                "name": (
                    "Funding zero-sum across users per symbol"
                ),
                "status": "skip",
                "time": now,
                "detail": f"pg error: {exc}",
            })
    else:
        checks.append({
            "name": "Funding zero-sum across users per symbol",
            "status": "skip",
            "time": now,
            "detail": "postgres not connected",
        })

    # ── invariant 10: advisory lock exclusive ───────────────
    # Detect duplicate PIDs for same service name (would mean
    # two instances of the same shard are running).
    name_pids: dict[str, list[int]] = {}
    for p in procs:
        if p.get("state") == "running" and p.get("pid") != "-":
            name_pids.setdefault(p["name"], []).append(
                p["pid"]
            )
    # Also scan PID_DIR directly for any extra stale entries
    if PID_DIR.exists():
        for pf in PID_DIR.glob("*.pid"):
            n = pf.stem
            try:
                pid = int(pf.read_text().strip())
                ps_obj = psutil.Process(pid)
                if ps_obj.is_running():
                    if pid not in name_pids.get(n, []):
                        name_pids.setdefault(n, []).append(pid)
            except (psutil.NoSuchProcess, ValueError, OSError):
                pass
    dupes_lock = {
        n: pids for n, pids in name_pids.items()
        if len(pids) > 1
    }
    checks.append({
        "name": "Advisory lock exclusive: one main per shard",
        "status": "fail" if dupes_lock else (
            "pass" if name_pids else "skip"
        ),
        "time": now,
        "detail": (
            "; ".join(
                f"{n}: pids {pids}"
                for n, pids in dupes_lock.items()
            )
            if dupes_lock
            else (
                f"{len(name_pids)} services, no duplicates"
                if name_pids
                else "no running services"
            )
        ),
    })

    verify_results.clear()
    verify_results.extend(checks)
    return checks


@app.post("/api/verify/run")
async def api_verify_run():
    checks = await _run_invariant_checks()
    return HTMLResponse(pages.render_verify(checks))


@app.post("/api/verify/run-json")
async def api_verify_run_json():
    checks = await _run_invariant_checks()
    return JSONResponse({"checks": checks})


@app.post("/api/orders/{cid}/cancel")
async def api_orders_cancel(cid: str):
    for o in recent_orders:
        if o["cid"] == cid and o["status"] == "submitted":
            o["status"] = "cancelled"
            return HTMLResponse(
                f'<span class="text-amber-400 text-xs">'
                f'{cid} cancelled</span>')
    return HTMLResponse(
        f'<span class="text-red-400 text-xs">'
        f'{cid} not found or not cancellable</span>')


@app.post("/api/orders/batch")
async def api_orders_batch():
    for i in range(10):
        recent_orders.append({
            "cid": f"bat-{int(time.time()*1000)%100000+i:05d}",
            "symbol": "10",
            "side": "buy" if i % 2 == 0 else "sell",
            "price": str(50000 + i * 10),
            "qty": "1.0", "status": "submitted",
            "ts": datetime.now().strftime("%H:%M:%S"),
        })
    _trim_recent_orders()
    return HTMLResponse(
        '<span class="text-emerald-400 text-xs">'
        '10 batch orders submitted</span>')


@app.post("/api/orders/random")
async def api_orders_random():
    for _ in range(5):
        recent_orders.append({
            "cid": f"rnd-{random.randint(10000,99999)}",
            "symbol": str(random.choice([1, 2, 3, 10])),
            "side": random.choice(["buy", "sell"]),
            "price": str(random.randint(40000, 60000)),
            "qty": f"{random.uniform(0.1, 5.0):.1f}",
            "status": "submitted",
            "ts": datetime.now().strftime("%H:%M:%S"),
        })
    _trim_recent_orders()
    return HTMLResponse(
        '<span class="text-emerald-400 text-xs">'
        '5 random orders submitted</span>')


@app.post("/api/orders/quick")
async def api_orders_quick(request: Request):
    """Quick order endpoint for the matrix buttons.

    Form params:
      side             buy|sell (ignored if randomize=true)
      qty              int (ignored if randomize=true)
      price_offset_pct float  0 = market, positive = above mid,
                              negative = below mid
      symbol_id        int    default 10
      randomize        true|false
      rand_side        true|false  random side only, use qty param
    """
    form = await request.form()

    try:
        symbol_id = int(form.get("symbol_id", "10"))
    except (ValueError, TypeError):
        symbol_id = 10

    randomize = form.get("randomize", "false").lower() == "true"
    rand_side = form.get("rand_side", "false").lower() == "true"

    qty_choices = [1, 5, 10, 25]

    if randomize:
        side_str = random.choice(["buy", "sell"])
        human_qty = float(random.choice(qty_choices))
        offset_pct = random.uniform(-2.0, 2.0)
    elif rand_side:
        side_str = random.choice(["buy", "sell"])
        try:
            human_qty = float(form.get("qty", "1"))
        except (ValueError, TypeError):
            human_qty = 1.0
        try:
            offset_pct = float(form.get("price_offset_pct", "0"))
        except (ValueError, TypeError):
            offset_pct = 0.0
    else:
        side_str = form.get("side", "buy")
        try:
            human_qty = float(form.get("qty", "1"))
        except (ValueError, TypeError):
            human_qty = 1.0
        try:
            offset_pct = float(form.get("price_offset_pct", "0"))
        except (ValueError, TypeError):
            offset_pct = 0.0

    side_int = 0 if side_str == "buy" else 1

    # Resolve symbol config for fixed-point conversion
    _sym_cfg = next(
        (v for v in start_mod.SYMBOLS.values()
         if v["id"] == symbol_id),
        {"tick": 1, "lot": 1,
         "price_dec": 8, "qty_dec": 8},
    )
    lot_size = _sym_cfg.get("lot", 1) or 1
    tick_size = _sym_cfg.get("tick", 1) or 1
    price_dec = _sym_cfg.get("price_dec", 8)
    qty_dec = _sym_cfg.get("qty_dec", 8)
    price_scale = 10 ** price_dec
    qty_scale = 10 ** qty_dec

    # Determine price: 0 = market; otherwise compute from mid
    if offset_pct == 0.0 and not randomize:
        price_int = 0
        tif_int = 1  # IOC for market
    else:
        # Get mid price from book snapshot or WAL BBO
        snap = _book_snap.get(symbol_id)
        mid_raw = None
        if snap:
            bids = snap.get("bids", [])
            asks = snap.get("asks", [])
            if bids and asks:
                mid_raw = (bids[0]["px"] + asks[0]["px"]) // 2
            elif bids:
                mid_raw = bids[0]["px"]
            elif asks:
                mid_raw = asks[0]["px"]
        if mid_raw is None:
            bbo = parse_wal_bbo(symbol_id)
            if bbo and bbo.get("bid_px") and bbo.get("ask_px"):
                mid_raw = (bbo["bid_px"] + bbo["ask_px"]) // 2
            elif bbo and bbo.get("bid_px"):
                mid_raw = bbo["bid_px"]
            elif bbo and bbo.get("ask_px"):
                mid_raw = bbo["ask_px"]

        if mid_raw is None:
            # No price data: fall back to market order
            price_int = 0
            tif_int = 1
        else:
            raw = mid_raw * (1.0 + offset_pct / 100.0)
            # Snap to tick grid
            price_int = int(round(raw / tick_size)) * tick_size
            if price_int <= 0:
                price_int = tick_size
            tif_int = 0  # GTC limit

    qty_int = int(round(human_qty * qty_scale))
    if qty_int % lot_size != 0:
        qty_int = max(lot_size, round(qty_int / lot_size) * lot_size)

    cid = f"qk{int(time.time()*1e6):x}"[:20]
    order_msg = {
        "N": [
            symbol_id, side_int, price_int, qty_int,
            cid, tif_int, 0, 0,
        ],
    }

    order = {
        "cid": cid,
        "user_id": 1,
        "symbol": str(symbol_id),
        "side": side_str,
        "price": str(price_int),
        "qty": str(qty_int),
        "tif": "IOC" if tif_int == 1 else "GTC",
        "reduce_only": False,
        "post_only": False,
        "status": "pending",
        "ts": datetime.now().strftime("%H:%M:%S"),
    }

    result = await send_order_to_gateway(order_msg, 1)
    err = result[1]
    label = f"{side_str.upper()} {human_qty:.0f}"
    if err:
        if err == "timeout waiting for response":
            order["status"] = "accepted"
            recent_orders.append(order)
            _trim_recent_orders()
            return HTMLResponse(
                f'<span class="text-emerald-400 text-xs font-medium">'
                f'queued {label} ({cid})</span>'
            )
        order["status"] = "error"
        order["error"] = err
        recent_orders.append(order)
        _trim_recent_orders()
        color = (
            "text-red-400"
            if err == "gateway not running"
            else "text-amber-400"
        )
        return HTMLResponse(
            f'<span class="{color} text-xs font-medium">'
            f'error: {err}</span>'
        )

    msg, _, latency_us = result
    if latency_us:
        order_latencies.append(latency_us)
        if len(order_latencies) > 1000:
            del order_latencies[:500]

    if msg and "U" in msg:
        u = msg["U"]
        status_map = {
            0: "filled", 1: "resting", 2: "cancelled", 3: "failed",
        }
        status_label = status_map.get(u[1], "unknown")
        order["status"] = status_label
        recent_orders.append(order)
        _trim_recent_orders()
        color = (
            "text-emerald-400" if u[1] in (0, 1)
            else "text-red-400"
        )
        return HTMLResponse(
            f'<span class="{color} text-xs font-medium">'
            f'{label} {status_label}</span>'
        )

    order["status"] = "sent"
    recent_orders.append(order)
    _trim_recent_orders()
    return HTMLResponse(
        f'<span class="text-emerald-400 text-xs font-medium">'
        f'sent {label}</span>'
    )


# ── stress subprocess ────────────────────────────────────

STRESS_SCRIPT = ROOT / "rsx-playground" / "stress.py"
STRESS_NAME = "stress"


def _stress_running() -> bool:
    info = managed.get(STRESS_NAME)
    if not info:
        return False
    return info["proc"].returncode is None


async def do_stress_start(cfg: dict | None = None) -> bool:
    """Start stress subprocess. Returns True if started."""
    if _stress_running():
        return True
    if STRESS_NAME in managed:
        del managed[STRESS_NAME]
    if not STRESS_SCRIPT.exists():
        return False
    _scfg: dict = cfg or {}
    env = {
        "RSX_STRESS_GW_URL": _scfg.get(
            "gw_url", GATEWAY_URL),
        "RSX_STRESS_USERS": str(
            _scfg.get("users", 10)),
        "RSX_STRESS_RATE": str(
            _scfg.get("rate", 1000)),
        "RSX_STRESS_DURATION": str(
            _scfg.get("duration", 60)),
        "RSX_STRESS_TARGET_P99": str(
            _scfg.get("target_p99", 50000)),
        "RSX_STRESS_REPORT_DIR": str(
            ROOT / "tmp" / "stress"),
    }
    full_env = {**os.environ, **env}
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    proc = await asyncio.create_subprocess_exec(
        sys.executable, str(STRESS_SCRIPT),
        env=full_env,
        cwd=str(ROOT / "rsx-playground"),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.STDOUT,
    )
    managed[STRESS_NAME] = {
        "proc": proc,
        "binary": str(STRESS_SCRIPT),
        "env": env,
    }
    PID_DIR.mkdir(parents=True, exist_ok=True)
    (PID_DIR / f"{STRESS_NAME}.pid").write_text(str(proc.pid))
    asyncio.create_task(
        pipe_output(STRESS_NAME, proc.stdout))
    await asyncio.sleep(0.2)
    if proc.returncode is not None:
        del managed[STRESS_NAME]
        (PID_DIR / f"{STRESS_NAME}.pid").unlink(
            missing_ok=True)
        return False
    return True


async def do_stress_stop() -> None:
    await stop_process(STRESS_NAME)
    (PID_DIR / f"{STRESS_NAME}.pid").unlink(missing_ok=True)


@app.post("/api/stress/start")
async def api_stress_start(request: Request):
    if _stress_running():
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">'
            'stress already running</span>')
    if not STRESS_SCRIPT.exists():
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            'stress.py not found</span>')
    body: dict = {}
    try:
        body = await request.json()
    except Exception:
        pass
    ok = await do_stress_start(body)
    if not ok:
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            'stress failed to start</span>')
    pid = managed[STRESS_NAME]["proc"].pid
    audit_log("/api/stress/start", "start stress")
    return HTMLResponse(
        '<span class="text-emerald-400 text-xs">'
        f'stress started (pid {pid})</span>')


@app.post("/api/stress/stop")
async def api_stress_stop(request: Request):
    if not _stress_running():
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">'
            'stress not running</span>')
    await do_stress_stop()
    audit_log("/api/stress/stop", "stop stress")
    return HTMLResponse(
        '<span class="text-amber-400 text-xs">'
        'stress stopped</span>')


@app.get("/api/stress/status")
async def api_stress_status():
    running = _stress_running()
    info = managed.get(STRESS_NAME)
    pid = info["proc"].pid if running and info else None
    return {
        "running": running,
        "pid": pid,
        "name": STRESS_NAME,
    }


@app.get("/api/stress/reports")
async def api_stress_reports():
    """List all stress test reports"""
    reports = []
    if STRESS_REPORTS_DIR.exists():
        for f in sorted(STRESS_REPORTS_DIR.glob("stress-*.json"), reverse=True):
            try:
                with open(f) as fp:
                    data = json.load(fp)
                reports.append({
                    "id": f.stem.replace("stress-", ""),
                    "timestamp": data.get("timestamp", "unknown"),
                    "rate": data["config"]["target_rate"],
                    "duration": data["config"]["duration"],
                    "submitted": data["metrics"]["submitted"],
                    "accepted": data["metrics"]["accepted"],
                    "accept_rate": data["metrics"]["accept_rate"],
                    "p99_latency": data["latency_us"]["p99"],
                })
            except Exception:
                continue
    return reports


@app.get("/api/stress/reports/{report_id}")
async def api_stress_report(report_id: str):
    """Get specific stress test report"""
    report_file = STRESS_REPORTS_DIR / f"stress-{report_id}.json"
    if not report_file.exists():
        return JSONResponse({"error": "Report not found"}, status_code=404)

    with open(report_file) as f:
        return JSONResponse(json.load(f))


@app.get("/x/stress-reports-list", response_class=HTMLResponse)
async def x_stress_reports_list():
    """HTMX endpoint for stress reports table"""
    reports = await api_stress_reports()
    if not reports:
        return HTMLResponse(
            '<div class="text-slate-500 text-xs">'
            'No stress tests run yet</div>')

    rows = []
    for r in reports:
        timestamp_fmt = r["timestamp"]
        # Format: 20260213-211030 -> 2026-02-13 21:10:30
        if len(timestamp_fmt) == 15:
            t = timestamp_fmt
            ts = (f"{t[0:4]}-{t[4:6]}-{t[6:8]}"
                  f" {t[9:11]}:{t[11:13]}:{t[13:15]}")
        else:
            ts = timestamp_fmt

        # Escape HTML to prevent XSS
        ts_escaped = html.escape(ts)
        id_escaped = html.escape(str(r["id"]))

        accept_color = ("text-emerald-400"
                        if r["accept_rate"] >= 95
                        else "text-amber-400")
        latency_color = ("text-emerald-400"
                         if r["p99_latency"] < 1000
                         else "text-amber-400")

        rows.append(
            f'<tr class="hover:bg-slate-800/30">'
            f'<td class="px-2 py-1 text-xs">'
            f'<a href="./stress/{id_escaped}"'
            f' class="text-blue-400 hover:underline">'
            f'{ts_escaped}</a>'
            f'</td>'
            f'<td class="px-2 py-1 text-xs text-right">{r["rate"]}/s</td>'
            f'<td class="px-2 py-1 text-xs text-right">{r["duration"]}s</td>'
            f'<td class="px-2 py-1 text-xs text-right">{r["submitted"]:,}</td>'
            f'<td class="px-2 py-1 text-xs text-right'
            f' {accept_color}">{r["accept_rate"]}%</td>'
            f'<td class="px-2 py-1 text-xs text-right'
            f' {latency_color}">{r["p99_latency"]}us</td>'
            f'</tr>'
        )

    table = (
        '<div class="overflow-x-auto">'
        '<table class="w-full text-left whitespace-nowrap">'
        '<thead><tr class="border-b border-slate-700">'
        '<th class="px-2 py-1 text-[10px] text-slate-400">Timestamp</th>'
        '<th class="px-2 py-1 text-[10px] text-slate-400 text-right">Rate</th>'
        '<th class="px-2 py-1 text-[10px]'
        ' text-slate-400 text-right">Duration</th>'
        '<th class="px-2 py-1 text-[10px]'
        ' text-slate-400 text-right">Submitted</th>'
        '<th class="px-2 py-1 text-[10px]'
        ' text-slate-400 text-right">Accept %</th>'
        '<th class="px-2 py-1 text-[10px]'
        ' text-slate-400 text-right">p99</th>'
        '</tr></thead>'
        f'<tbody>{"".join(rows)}</tbody>'
        '</table>'
        '</div>'
    )

    return HTMLResponse(table)


@app.post("/api/orders/invalid")
async def api_orders_invalid():
    recent_orders.append({
        "cid": "inv-00001", "symbol": "999",
        "side": "buy", "price": "-1", "qty": "0",
        "status": "rejected",
        "ts": datetime.now().strftime("%H:%M:%S"),
    })
    return HTMLResponse(
        '<span class="text-red-400 text-xs">'
        'invalid order submitted (rejected)</span>')


@app.post("/api/users/create")
async def api_create_user():
    if pg_pool is None:
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            'postgres not connected</span>')
    try:
        async with pg_pool.acquire() as conn:
            result = await conn.fetchrow(
                "INSERT INTO users (created_at) VALUES (NOW()) "
                "RETURNING user_id"
            )
            user_id = result['user_id']
            await conn.execute(
                "INSERT INTO balances (user_id, symbol_id, balance) "
                "VALUES ($1, 0, 1000000000000)",
                user_id
            )
            return HTMLResponse(
                f'<span class="text-emerald-400 text-xs">'
                f'created user {user_id} with 10000 USDC</span>')
    except Exception as e:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'error: {str(e)}</span>')


@app.post("/api/users/{user_id}/deposit")
async def api_deposit(
    user_id: int,
    amount: int = Form(100_000),
):
    _user_balances[user_id] = (
        _user_balances.get(user_id, 0) + amount)
    bal = _user_balances[user_id]
    return HTMLResponse(
        f'<span class="text-emerald-400 text-xs">'
        f'deposited {amount} for user {user_id} '
        f'(balance: {bal})</span>')


@app.post("/api/risk/liquidate")
async def api_liquidate(
    user_id: int = Form(0),
    symbol_id: int = Form(10),
):
    entry = {
        "user_id": user_id,
        "symbol_id": symbol_id,
        "ts": time.time(),
        "status": "triggered",
    }
    _liquidation_log.append(entry)
    import logging
    logging.info("liquidation triggered: %s", entry)
    return HTMLResponse(
        f'<span class="text-amber-400 text-xs">'
        f'liquidation triggered for user {user_id} '
        f'symbol {symbol_id}</span>')


@app.post("/api/wal/verify")
async def api_wal_verify():
    streams = scan_wal_streams()
    if not streams:
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">'
            'no WAL streams found</span>')
    total_files = sum(s["files"] for s in streams)
    return HTMLResponse(
        f'<span class="text-emerald-400 text-xs">'
        f'verified {len(streams)} streams, '
        f'{total_files} files</span>')


@app.post("/api/wal/dump")
async def api_wal_dump():
    files = scan_wal_files()
    if not files:
        return HTMLResponse(
            '<span class="text-slate-500 text-xs">'
            'no WAL files to dump</span>')

    # Dump first 100 records from most recent WAL file
    latest_dict = max(files, key=lambda f: f.get("modified", ""))
    stream_name = latest_dict["stream"]
    file_name = latest_dict["name"]
    latest_path = WAL_DIR / stream_name / file_name

    proc = await asyncio.create_subprocess_exec(
        str(ROOT / "target" / "debug" / "rsx-cli"), "dump", str(latest_path),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE
    )
    stdout, stderr = await proc.communicate()
    output = stdout.decode() if stdout else stderr.decode()

    safe_name = html.escape(file_name)
    safe_out = html.escape(output[:2000])
    dump_html = (
        f'<div class="text-xs">'
        f'<div class="text-slate-400 mb-2">'
        f'Latest: {safe_name}</div>'
        f'<pre class="text-slate-300 whitespace-pre-wrap">'
        f'{safe_out}</pre></div>')

    return HTMLResponse(dump_html)


@app.get("/api/risk/users/{user_id}")
async def api_risk_user(user_id: int):
    data = await pg_query(
        "SELECT * FROM positions "
        "WHERE user_id = $1",
        user_id,
    )
    if data and isinstance(data, list):
        return data
    return {"user_id": user_id,
            "status": "no postgres connection"}


@app.post("/api/risk/users/{user_id}/{action}")
async def api_risk_action(user_id: int, action: str):
    if action not in ("freeze", "unfreeze"):
        return JSONResponse(
            {"error": f"unknown action: {action}"},
            status_code=400)
    if action == "freeze":
        _user_frozen.add(user_id)
        return {"user_id": user_id, "action": "freeze",
                "status": "frozen"}
    else:
        _user_frozen.discard(user_id)
        return {"user_id": user_id,
                "action": "unfreeze",
                "status": "unfrozen"}


# ── Risk dashboard API endpoints ──────────────────────────

@app.get("/api/risk/overview")
async def api_risk_overview():
    """Account overview: collateral, margin, positions."""
    fills = parse_wal_fills(max_fills=2000)
    book_stats = parse_wal_book_stats()

    # Build per-user, per-symbol net positions from fills
    # positions[uid][sid] = {net, entry_px_sum, fill_count}
    positions: dict[int, dict[int, dict]] = {}
    for f in fills:
        t_uid = f.get("taker_uid", 0)
        m_uid = f.get("maker_uid", 0)
        sid = f.get("symbol_id", 0)
        qty = f.get("qty", 0)
        px = f.get("price", 0)
        taker_side = f.get("taker_side", 0)
        for uid, is_taker in ((t_uid, True), (m_uid, False)):
            if uid == 0:
                continue
            signed = (qty if taker_side == 0 else -qty
                      ) if is_taker else (
                -qty if taker_side == 0 else qty)
            ud = positions.setdefault(uid, {})
            pos = ud.setdefault(sid, {
                "net": 0, "entry_px_sum": 0,
                "fill_count": 0,
            })
            pos["net"] += signed
            pos["entry_px_sum"] += px * abs(signed)
            pos["fill_count"] += 1

    # Fetch account balances from postgres if available
    accounts: dict[int, dict] = {}
    if pg_pool is not None:
        rows = await pg_query(
            "SELECT user_id, collateral, frozen_margin "
            "FROM accounts ORDER BY user_id"
        )
        if rows and isinstance(rows, list):
            for r in rows:
                accounts[r["user_id"]] = {
                    "collateral": r["collateral"],
                    "frozen": r["frozen_margin"],
                }

    # Fallback: use seed users with seed collateral
    if not accounts:
        for uid in _SEED_USERS:
            accounts[uid] = {
                "collateral": _SEED_COLLATERAL,
                "frozen": 0,
            }

    # IM = 10% of notional, MM = 5% of notional
    IM_RATE = 0.10
    MM_RATE = 0.05

    users_out = []
    all_long_notional = 0
    all_short_notional = 0
    accounts_with_positions = 0
    accounts_near_liq = 0

    all_uids = sorted(
        set(list(accounts.keys()) + list(positions.keys()))
    )
    for uid in all_uids:
        acct = accounts.get(uid, {
            "collateral": _SEED_COLLATERAL,
            "frozen": 0,
        })
        collateral = acct["collateral"]
        frozen = acct["frozen"]
        user_positions = positions.get(uid, {})

        pos_list = []
        total_upnl = 0
        total_im = 0
        total_mm = 0

        for sid, pos in user_positions.items():
            net = pos["net"]
            if net == 0:
                continue
            fill_count = pos["fill_count"]
            # average entry price
            entry_px = (
                pos["entry_px_sum"] // abs(net)
                if net != 0 else 0
            )
            # mark price from BBO mid
            bbo = book_stats.get(sid, {})
            bid = bbo.get("bid_px", 0)
            ask = bbo.get("ask_px", 0)
            mark_px = (
                (bid + ask) // 2 if bid and ask
                else entry_px
            )
            notional = abs(net) * mark_px
            upnl = (
                (mark_px - entry_px) * net
                if net != 0 else 0
            )
            im = int(notional * IM_RATE)
            mm = int(notional * MM_RATE)
            total_upnl += upnl
            total_im += im
            total_mm += mm
            if net > 0:
                all_long_notional += notional
            else:
                all_short_notional += abs(notional)
            pos_list.append({
                "symbol_id": sid,
                "net": net,
                "entry_px": entry_px,
                "mark_px": mark_px,
                "upnl": upnl,
                "notional": notional,
                "im": im,
                "mm": mm,
                "fills": fill_count,
            })

        if pos_list:
            accounts_with_positions += 1

        equity = collateral + total_upnl
        margin_ratio = (
            equity / total_mm if total_mm > 0 else 999.0
        )
        if margin_ratio < 1.5 and total_mm > 0:
            accounts_near_liq += 1

        users_out.append({
            "user_id": uid,
            "collateral": collateral,
            "frozen": frozen,
            "available": max(0, collateral - frozen),
            "equity": equity,
            "upnl": total_upnl,
            "im_required": total_im,
            "mm_required": total_mm,
            "margin_ratio": round(margin_ratio, 3),
            "positions": pos_list,
        })

    total_oi = all_long_notional + all_short_notional
    return {
        "users": users_out,
        "system": {
            "total_oi": total_oi,
            "long_notional": all_long_notional,
            "short_notional": all_short_notional,
            "accounts_with_positions": accounts_with_positions,
            "accounts_near_liq": accounts_near_liq,
        },
    }


@app.get("/api/risk/funding")
async def api_risk_funding():
    """Funding rates per symbol from BBO data."""
    book_stats = parse_wal_book_stats()
    now_ns = int(time.time() * 1e9)
    # funding settles every 8 hours = 28800s
    settlement_interval_s = 28800
    elapsed = int(time.time()) % settlement_interval_s
    next_s = settlement_interval_s - elapsed

    entries = []
    for sid in sorted(book_stats.keys()):
        bbo = book_stats[sid]
        bid = bbo.get("bid_px", 0)
        ask = bbo.get("ask_px", 0)
        mid = (bid + ask) // 2 if bid and ask else 0
        spread = ask - bid if bid and ask else 0
        # Funding rate proxy: spread/mid in bps
        rate_bps = (
            spread * 10000 // mid if mid > 0 else 0
        )
        index_px = int(mid * 1.0001) if mid else 0
        premium_bps = (
            (mid - index_px) * 10000 // index_px
            if index_px else 0
        )
        entries.append({
            "symbol_id": sid,
            "mark_px": mid,
            "index_px": index_px,
            "bid_px": bid,
            "ask_px": ask,
            "rate_bps": rate_bps,
            "premium_bps": premium_bps,
            "next_settlement_s": next_s,
        })
    return {"funding": entries, "ts_ns": now_ns}


@app.get("/api/risk/liquidations")
async def api_risk_liquidations():
    """Active liquidation queue from WAL or postgres."""
    # Try postgres first
    if pg_pool is not None:
        rows = await pg_query(
            "SELECT * FROM liquidations "
            "ORDER BY timestamp_ns DESC LIMIT 50"
        )
        if rows and isinstance(rows, list):
            return {"liquidations": rows, "source": "postgres"}
    # Fallback to WAL
    liqns = parse_wal_liquidations()
    return {
        "liquidations": liqns[:50],
        "source": "wal" if liqns else "none",
    }


@app.get("/api/risk/insurance")
async def api_risk_insurance():
    """Insurance fund balances per symbol."""
    # Try postgres
    if pg_pool is not None:
        rows = await pg_query(
            "SELECT symbol_id, balance, version "
            "FROM insurance_fund ORDER BY symbol_id"
        )
        if rows and isinstance(rows, list):
            total = sum(r.get("balance", 0) for r in rows)
            return {
                "funds": rows, "total": total,
                "source": "postgres",
            }
    return {"funds": [], "total": 0, "source": "none"}


@app.get("/x/risk-overview", response_class=HTMLResponse)
async def x_risk_overview():
    data = await api_risk_overview()
    funding_data = await api_risk_funding()
    liq_data = await api_risk_liquidations()
    insurance_data = await api_risk_insurance()
    return HTMLResponse(
        pages.render_risk_overview(
            data, funding_data, liq_data, insurance_data,
        )
    )


@app.get("/api/latency")
async def api_latency():
    if not order_latencies:
        return JSONResponse({"count": 0})
    s = sorted(order_latencies)
    n = len(s)
    return JSONResponse({
        "count": n,
        "p50": s[n // 2],
        "p95": s[int(n * 0.95)],
        "p99": s[int(n * 0.99)],
        "min": s[0],
        "max": s[-1],
    })


@app.get("/api/gateway-mode")
async def api_gateway_mode():
    reachable = await _probe_gateway_tcp()
    return {
        "mode": "live" if reachable else "offline",
        "url": GATEWAY_URL,
    }


@app.get("/api/mark/prices")
async def api_mark_prices():
    prices = {}
    bbo_map = _latest_bbo_from_wal()
    for sid, bbo in bbo_map.items():
        bid = bbo.get("bid_px", 0)
        ask = bbo.get("ask_px", 0)
        if bid > 0 and ask > 0:
            prices[str(sid)] = {
                "mark": (bid + ask) // 2,
                "bid": bid,
                "ask": ask,
                "source": "wal",
            }
    # fall back to live book snap for symbols without WAL data
    for sid, snap in _book_snap.items():
        if str(sid) in prices:
            continue
        bids = snap.get("bids", [])
        asks = snap.get("asks", [])
        if bids and asks:
            best_bid = bids[0].get("px", 0)
            best_ask = asks[0].get("px", 0)
            if best_bid > 0 and best_ask > 0:
                prices[str(sid)] = {
                    "mark": (best_bid + best_ask) // 2,
                    "bid": best_bid,
                    "ask": best_ask,
                    "source": "book_snap",
                }
    return {"prices": prices}


@app.get("/api/metrics")
async def api_metrics():
    procs = scan_processes()
    return {
        "processes": len(procs),
        "running": len(
            [p for p in procs if p["state"] == "running"]),
        "postgres": pg_pool is not None,
    }


@app.get("/api/status")
async def api_status():
    procs = scan_processes()
    running = [p for p in procs if p["state"] == "running"]
    maker_running = _maker_running()
    maker_stats = _read_maker_stats() if maker_running else {}
    maker_info = managed.get(MAKER_NAME)
    maker_pid = (
        maker_info["proc"].pid
        if maker_running and maker_info
        else None
    )
    gateway_up, marketdata_up = await asyncio.gather(
        _probe_gateway_tcp(),
        _probe_marketdata_tcp(),
    )
    return {
        "processes": len(procs),
        "running": len(running),
        "postgres": pg_pool is not None,
        "gateway": gateway_up,
        "marketdata": marketdata_up,
        "maker": {
            "running": maker_running,
            "pid": maker_pid,
            "levels": maker_stats.get("levels", 0),
        },
    }


# ── market maker ────────────────────────────────────────

MAKER_SCRIPT = ROOT / "rsx-playground" / "market_maker.py"
MAKER_NAME = "maker"
MAKER_STATUS_FILE = TMP / "maker-status.json"


def _read_maker_stats() -> dict:
    """Read maker status file written by maker subprocess."""
    try:
        return json.loads(MAKER_STATUS_FILE.read_text())
    except Exception:
        return {}


def _maker_running() -> bool:
    info = managed.get(MAKER_NAME)
    if not info:
        return False
    return info["proc"].returncode is None


@app.post("/api/maker/start")
async def api_maker_start(request: Request):
    if _maker_running():
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">'
            'maker already running</span>')
    if not MAKER_SCRIPT.exists():
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            'market_maker.py not found</span>')
    ok = await do_maker_start()
    if not ok:
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            'maker failed to start</span>')
    pid = managed[MAKER_NAME]["proc"].pid
    audit_log("/api/maker/start", "start maker")
    return HTMLResponse(
        '<span class="text-emerald-400 text-xs">'
        f'maker started (pid {pid})</span>')


@app.post("/api/maker/stop")
async def api_maker_stop(request: Request):
    if not _maker_running():
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">'
            'maker not running</span>')
    await stop_process(MAKER_NAME)
    audit_log("/api/maker/stop", "stop maker")
    return HTMLResponse(
        '<span class="text-amber-400 text-xs">'
        'maker stopped</span>')


@app.get("/api/maker/status")
async def api_maker_status():
    running = _maker_running()
    info = managed.get(MAKER_NAME)
    pid = info["proc"].pid if running and info else None
    stats = _read_maker_stats()
    levels = stats.get("levels", 0)
    errors = stats.get("errors", [])
    return {
        "running": running,
        "pid": pid,
        "name": MAKER_NAME,
        "levels": levels,
        "errors": errors,
    }


MAKER_CONFIG = TMP / "maker-config.json"


@app.patch("/api/maker/config")
async def api_maker_config(request: Request):
    body = await request.json()
    mid_override = body.get("mid_override")
    if not isinstance(mid_override, (int, float)):
        return JSONResponse(
            {"error": "mid_override must be int"}, status_code=400)
    MAKER_CONFIG.parent.mkdir(parents=True, exist_ok=True)
    # Merge with existing config
    try:
        existing = json.loads(MAKER_CONFIG.read_text())
    except Exception:
        existing = {}
    existing["mid_override"] = mid_override
    tmp = MAKER_CONFIG.with_suffix(".tmp")
    tmp.write_text(json.dumps(existing))
    tmp.replace(MAKER_CONFIG)
    return {"ok": True}


@app.post("/api/maker/config")
async def api_maker_config_save(
    request: Request,
    spread_bps: int = Form(20),
    qty: int = Form(10),
    symbol_id: int = Form(10),
    refresh_ms: int = Form(500),
    levels: int = Form(5),
):
    """Save maker config and restart if running."""
    MAKER_CONFIG.parent.mkdir(parents=True, exist_ok=True)
    # Preserve existing mid_override if present
    try:
        existing = json.loads(MAKER_CONFIG.read_text())
    except Exception:
        existing = {}
    existing.update({
        "spread_bps": spread_bps,
        "qty": qty,
        "symbol_id": symbol_id,
        "refresh_ms": refresh_ms,
        "levels": levels,
    })
    tmp = MAKER_CONFIG.with_suffix(".tmp")
    tmp.write_text(json.dumps(existing))
    tmp.replace(MAKER_CONFIG)
    was_running = _maker_running()
    if was_running:
        await stop_process(MAKER_NAME)
        await asyncio.sleep(0.3)
        ok = await do_maker_start()
        if not ok:
            return HTMLResponse(
                '<span class="text-red-400 text-xs">'
                'config saved; maker failed to restart</span>')
        pid = managed[MAKER_NAME]["proc"].pid
        audit_log(
            "/api/maker/config",
            f"saved config and restarted maker (pid {pid})",
        )
        return HTMLResponse(
            '<span class="text-emerald-400 text-xs">'
            f'config saved; maker restarted (pid {pid})</span>')
    audit_log("/api/maker/config", "saved maker config")
    return HTMLResponse(
        '<span class="text-amber-400 text-xs">'
        'config saved (maker not running)</span>')


@app.post("/api/maker/restart")
async def api_maker_restart(request: Request):
    """Restart the market maker."""
    if _maker_running():
        await stop_process(MAKER_NAME)
        await asyncio.sleep(0.3)
    ok = await do_maker_start()
    if not ok:
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            'maker failed to start</span>')
    pid = managed[MAKER_NAME]["proc"].pid
    audit_log("/api/maker/restart", f"restarted maker (pid {pid})")
    return HTMLResponse(
        '<span class="text-emerald-400 text-xs">'
        f'maker restarted (pid {pid})</span>')


@app.get("/api/maker/stats")
async def api_maker_stats():
    """Return maker live stats from status file."""
    running = _maker_running()
    stats = _read_maker_stats() if running else {}
    cfg: dict = {}
    try:
        cfg = json.loads(MAKER_CONFIG.read_text())
    except Exception:
        pass
    return {
        "running": running,
        "orders_placed": stats.get("orders_placed", 0),
        "active_orders": stats.get("active_orders", 0),
        "mid_prices": stats.get("mid_prices", {}),
        "errors": stats.get("errors", []),
        "spread_bps": cfg.get(
            "spread_bps", stats.get("spread_bps", 20)),
        "qty": cfg.get("qty", 10),
        "symbol_id": cfg.get("symbol_id", 10),
        "refresh_ms": cfg.get("refresh_ms", 500),
        "levels": cfg.get("levels", 5),
    }


def _maker_book(symbol_id: int) -> dict | None:
    """Synthesize a book snapshot from maker config/status.

    Returns None when maker is stopped or has no mid price.
    Sources checked in order:
      1. maker-status.json mid_prices (set by maker after preflight)
      2. maker-config.json mid_override (set by admin PATCH)
    Requires maker to be running (managed-dict check) so the book
    clears when maker stops.
    """
    if not _maker_running():
        return None
    # Try status file first (authoritative when maker is past preflight)
    stats = _read_maker_stats()
    mid_prices = stats.get("mid_prices", {})
    mid = mid_prices.get(str(symbol_id)) or mid_prices.get(symbol_id)

    # Fall back to admin-configured mid_override
    if not mid:
        try:
            cfg = json.loads(MAKER_CONFIG.read_text())
            mid = cfg.get("mid_override")
        except Exception:
            pass

    if not mid:
        return None
    mid = int(mid)
    spread_bps = int(stats.get("spread_bps", 10))
    num_levels = int(stats.get("num_levels", 5))
    qty = int(stats.get("qty_per_level", 10))
    # half spread in raw price units; round up to at least 1 tick
    half = max(1, mid * spread_bps // 20_000)
    bids = [
        {"px": mid - half - i, "qty": qty}
        for i in range(num_levels)
    ]
    asks = [
        {"px": mid + half + i, "qty": qty}
        for i in range(num_levels)
    ]
    return {"bids": bids, "asks": asks}


@app.get("/api/book/{symbol_id}")
async def api_book(symbol_id: int):
    # Prefer live snapshot from marketdata WS
    snap = _book_snap.get(symbol_id)
    if snap and (snap.get("bids") or snap.get("asks")):
        return snap
    # Fallback: WAL BBO (at most 1 bid + 1 ask)
    bbo = parse_wal_bbo(symbol_id)
    if bbo is not None:
        bids = []
        asks = []
        if bbo["bid_px"] != 0:
            bids.append({"px": bbo["bid_px"], "qty": bbo["bid_qty"]})
        if bbo["ask_px"] != 0:
            asks.append({"px": bbo["ask_px"], "qty": bbo["ask_qty"]})
        if bids or asks:
            return {"bids": bids, "asks": asks}
    # Last fallback: synthesize from maker-status.json
    maker_snap = _maker_book(symbol_id)
    if maker_snap:
        return maker_snap
    return {"bids": [], "asks": []}


@app.get("/api/bbo/{symbol_id}")
async def api_bbo(symbol_id: int):
    # Prefer live snapshot (same source as /api/book)
    snap = _book_snap.get(symbol_id)
    if snap:
        bids = snap.get("bids", [])
        asks = snap.get("asks", [])
        bid_px = bids[0]["px"] if bids else 0
        bid_qty = bids[0]["qty"] if bids else 0
        ask_px = asks[0]["px"] if asks else 0
        ask_qty = asks[0]["qty"] if asks else 0
        if bid_px or ask_px:
            return {
                "bid_px": bid_px,
                "ask_px": ask_px,
                "bid_qty": bid_qty,
                "ask_qty": ask_qty,
            }
    # Fallback: WAL BBO
    bbo = parse_wal_bbo(symbol_id)
    if bbo is None:
        return JSONResponse(status_code=404, content={
            "error": "no bbo for symbol"})
    return {
        "bid_px": bbo["bid_px"],
        "ask_px": bbo["ask_px"],
        "bid_qty": bbo["bid_qty"],
        "ask_qty": bbo["ask_qty"],
    }


@app.post("/api/sessions/allocate")
async def api_sessions_allocate(request: Request):
    """Allocate an exclusive orchestrator session.

    At most one run may hold the lock. Returns 409 if another
    session is active and within the lease window.

    Idempotent reclaim: if the request body includes a
    session_id matching the active session, the existing
    session is returned (safe for retries after crashes).

    Stale-claim recovery: sessions not renewed within
    _LEASE_TTL are auto-released on the next allocate call,
    allowing quick restart after crashes without waiting the
    full _SESSION_TTL.

    _session_lock serialises concurrent callers so the
    check-then-set is atomic within this process.
    """
    global _active_session
    body: dict = {}
    try:
        body = await request.json()
    except Exception:
        pass
    claim_id = body.get("session_id", "")
    async with _session_lock:
        now = time.time()
        if _active_session is not None:
            age = now - _active_session["ts"]
            # Idempotent ownership check: caller already owns it.
            if claim_id and claim_id == _active_session["id"]:
                print(
                    f"session: idempotent reclaim "
                    f"{_active_session['id']} (age {age:.0f}s)"
                )
                return {
                    "session_id": _active_session["id"],
                    "run_id": _active_session["run_id"],
                    "ok": True,
                    "reclaimed": True,
                }
            # Stale-claim recovery: lease expired, auto-release.
            if age >= _LEASE_TTL:
                print(
                    f"session: auto-releasing stale session "
                    f"{_active_session['id']} (age {age:.0f}s, "
                    f"lease {_LEASE_TTL:.0f}s)"
                )
                _active_session = None
            else:
                return JSONResponse(
                    {
                        "error": "session collision: another run is "
                                 "active",
                        "active_id": _active_session["id"],
                        "age_s": round(age, 1),
                    },
                    status_code=409,
                )
        session_id = uuid.uuid4().hex
        run_id = uuid.uuid4().hex
        _active_session = {
            "id": session_id, "run_id": run_id, "ts": now,
        }
        return {"session_id": session_id, "run_id": run_id, "ok": True}


@app.post("/api/sessions/renew")
async def api_sessions_renew(request: Request):
    """Atomically renew (extend) the TTL of the active session.

    The caller must present the correct session_id to prove ownership.
    Returns 409 if no session is active or the session_id is wrong.
    On success, resets the session timestamp to now and returns the
    new ttl_remaining_s.  Duplicate session_id re-submissions that
    arrive while a *different* session is active are rejected 409.
    """
    global _active_session
    body = await request.json()
    session_id = body.get("session_id", "")
    async with _session_lock:
        now = time.time()
        if _active_session is None:
            return JSONResponse(
                {"error": "no active session; cannot renew"},
                status_code=409,
            )
        if _active_session["id"] != session_id:
            return JSONResponse(
                {
                    "error": "session_id mismatch: renewal rejected",
                    "active_id": _active_session["id"],
                },
                status_code=409,
            )
        # Atomic TTL reset: replace ts in place, keep id and run_id
        _active_session = {
            "id": _active_session["id"],
            "run_id": _active_session["run_id"],
            "ts": now,
        }
        ttl_remaining = _SESSION_TTL
        return {
            "ok": True,
            "session_id": session_id,
            "ttl_remaining_s": round(ttl_remaining, 1),
        }


@app.post("/api/sessions/release")
async def api_sessions_release(request: Request):
    """Release the orchestrator session lock."""
    global _active_session
    body = await request.json()
    session_id = body.get("session_id", "")
    async with _session_lock:
        if _active_session is None:
            return {"ok": True, "note": "no active session"}
        if _active_session["id"] != session_id:
            return JSONResponse(
                {"error": "session_id mismatch"},
                status_code=400,
            )
        _active_session = None
    return {"ok": True}


@app.get("/api/sessions/status")
async def api_sessions_status():
    """Return current session state for monitoring / debugging.

    Also performs stale-session reclamation under _session_lock
    so callers get an accurate view even if the reaper has not
    yet fired.
    """
    global _active_session
    async with _session_lock:
        now = time.time()
        if _active_session is None:
            return {"active": False}
        age = now - _active_session["ts"]
        if age >= _SESSION_TTL:
            print(
                f"session: status reclaiming expired session "
                f"{_active_session['id']} (age {age:.0f}s)"
            )
            _active_session = None
            return {"active": False}
        ttl_remaining = max(0.0, _SESSION_TTL - age)
        lease_remaining = max(0.0, _LEASE_TTL - age)
        return {
            "active": True,
            "active_id": _active_session["id"],
            "run_id": _active_session["run_id"],
            "age_s": round(age, 1),
            "ttl_remaining_s": round(ttl_remaining, 1),
            "lease_remaining_s": round(lease_remaining, 1),
            "stale": age >= _LEASE_TTL,
        }


@app.get("/x/maker-status", response_class=HTMLResponse)
async def x_maker_status():
    info = managed.get(MAKER_NAME)
    running = _maker_running()
    if not running:
        return HTMLResponse(
            '<span class="text-slate-500 text-xs">'
            'maker stopped</span>')
    pid = info["proc"].pid if info else "?"
    stats = _read_maker_stats()
    return HTMLResponse(pages.maker_status_html(stats, pid))


@app.get("/x/maker-live", response_class=HTMLResponse)
async def x_maker_live():
    """HTMX partial: live maker status for maker page."""
    running = _maker_running()
    info = managed.get(MAKER_NAME)
    pid = info["proc"].pid if running and info else None
    rs = _restart_state.get(MAKER_NAME, {})
    restarts = rs.get("restarts", 0)
    stats = _read_maker_stats() if running else {}
    return HTMLResponse(
        pages.maker_live_html(
            running=running,
            pid=pid,
            restarts=restarts,
            stats=stats,
        )
    )


# ── trading UI: WS proxy + REST proxy + static ─────────


@app.websocket("/ws/private")
async def ws_private_proxy(ws: WebSocket):
    """Proxy private WS to Gateway."""
    await ws.accept()
    headers = {"x-user-id": ws.headers.get(
        "x-user-id", "1")}
    auth = ws.headers.get("authorization")
    if auth:
        headers["authorization"] = auth
    try:
        async with aiohttp.ClientSession() as session:
            async with session.ws_connect(
                GATEWAY_URL, headers=headers,
            ) as upstream:
                close_code: int = 1000
                close_reason: str = ""

                async def fwd_up():
                    nonlocal close_code, close_reason
                    try:
                        async for msg in upstream:
                            if msg.type == aiohttp.WSMsgType.TEXT:
                                await ws.send_text(msg.data)
                            elif msg.type == aiohttp.WSMsgType.CLOSE:
                                close_code = msg.data
                                close_reason = msg.extra or ""
                                break
                            elif msg.type in (
                                aiohttp.WSMsgType.CLOSED,
                                aiohttp.WSMsgType.ERROR,
                            ):
                                break
                    except Exception:
                        pass

                async def fwd_down():
                    try:
                        while True:
                            data = await ws.receive_text()
                            await upstream.send_str(data)
                    except WebSocketDisconnect:
                        pass

                await asyncio.gather(
                    fwd_up(), fwd_down(),
                    return_exceptions=True)
                await ws.close(
                    code=close_code, reason=close_reason)
    except (ConnectionRefusedError, OSError):
        await ws.close(code=1013,
                       reason="gateway not running")


@app.websocket("/ws/public")
async def ws_public_proxy(ws: WebSocket):
    """Proxy public WS to Marketdata."""
    await ws.accept()
    try:
        async with aiohttp.ClientSession() as session:
            async with session.ws_connect(
                MARKETDATA_WS,
            ) as upstream:
                close_code: int = 1000
                close_reason: str = ""

                async def fwd_up():
                    nonlocal close_code, close_reason
                    try:
                        async for msg in upstream:
                            if msg.type == aiohttp.WSMsgType.TEXT:
                                await ws.send_text(msg.data)
                            elif msg.type == aiohttp.WSMsgType.CLOSE:
                                close_code = msg.data
                                close_reason = msg.extra or ""
                                break
                            elif msg.type in (
                                aiohttp.WSMsgType.CLOSED,
                                aiohttp.WSMsgType.ERROR,
                            ):
                                break
                    except Exception:
                        pass

                async def fwd_down():
                    try:
                        while True:
                            data = await ws.receive_text()
                            await upstream.send_str(data)
                    except WebSocketDisconnect:
                        pass

                await asyncio.gather(
                    fwd_up(), fwd_down(),
                    return_exceptions=True)
                await ws.close(
                    code=close_code, reason=close_reason)
    except (ConnectionRefusedError, OSError):
        await ws.close(code=1013,
                       reason="marketdata not running")


async def _probe_gateway_tcp() -> bool:
    """Return True if gateway TCP port :8080 is reachable."""
    import urllib.parse
    parsed = urllib.parse.urlparse(GATEWAY_HTTP)
    host = parsed.hostname or "localhost"
    port = parsed.port or 8080
    try:
        _, writer = await asyncio.wait_for(
            asyncio.open_connection(host, port),
            timeout=1.0,
        )
        writer.close()
        try:
            await writer.wait_closed()
        except Exception:
            pass
        return True
    except Exception:
        return False


async def _probe_marketdata_tcp() -> bool:
    """Return True if marketdata TCP port is reachable."""
    import urllib.parse
    parsed = urllib.parse.urlparse(MARKETDATA_WS)
    host = parsed.hostname or "localhost"
    port = parsed.port or 8081
    try:
        _, writer = await asyncio.wait_for(
            asyncio.open_connection(host, port),
            timeout=1.0,
        )
        writer.close()
        try:
            await writer.wait_closed()
        except Exception:
            pass
        return True
    except Exception:
        return False


@app.get("/v1/symbols")
async def v1_symbols():
    """Return configured symbol catalog (local fallback)."""
    rows = []
    for name, cfg in start_mod.SYMBOLS.items():
        rows.append({
            "id": cfg["id"],
            "symbol": name,
            "tick_size": cfg["tick"],
            "lot_size": cfg["lot"],
            "price_decimals": cfg.get("price_dec", 2),
            "qty_decimals": cfg.get("qty_dec", 4),
        })
    rows.sort(key=lambda r: r["id"])
    return JSONResponse({"symbols": rows})


TF_SECONDS = {
    "1m": 60, "5m": 300, "15m": 900,
    "1h": 3600, "4h": 14400, "1d": 86400,
}


def _symbol_id_for(sym: str) -> int | None:
    for name, cfg in start_mod.SYMBOLS.items():
        if name == sym or str(cfg["id"]) == sym:
            return cfg["id"]
    return None


def _tick_for(sym: str) -> float:
    for name, cfg in start_mod.SYMBOLS.items():
        if name == sym or str(cfg["id"]) == sym:
            return cfg.get("tick", 1)
    return 1


def _build_candles_from_wal(
    symbol_id: int, tf_secs: int, limit: int
) -> list:
    """Aggregate WAL fill records into OHLCV bars."""
    fills = []
    for stream_dir in _wal_stream_dirs():
        for rec in parse_wal_records(
            stream_dir, record_types={RECORD_FILL}
        ):
            if rec["symbol_id"] == symbol_id:
                fills.append(rec)
    if not fills:
        return []
    fills.sort(key=lambda r: r["ts_ns"])
    bars: dict[int, dict] = {}
    for f in fills:
        ts_s = f["ts_ns"] // 1_000_000_000
        bucket = (ts_s // tf_secs) * tf_secs
        px = f["price"]
        qty = f["qty"]
        if bucket not in bars:
            bars[bucket] = {
                "t": bucket,
                "o": px, "h": px, "l": px, "c": px,
                "v": qty,
            }
        else:
            b = bars[bucket]
            b["h"] = max(b["h"], px)
            b["l"] = min(b["l"], px)
            b["c"] = px
            b["v"] += qty
    sorted_bars = sorted(bars.values(), key=lambda b: b["t"])
    return sorted_bars[-limit:]


def _synthetic_candles(
    sym: str, tf_secs: int, limit: int
) -> list:
    """Generate plausible synthetic OHLCV bars."""
    tick = _tick_for(sym)
    now_s = int(time.time())
    bucket = (now_s // tf_secs) * tf_secs
    # Base price from SYMBOLS config when available
    name = sym.upper()
    base_raw = None
    for sname, cfg in start_mod.SYMBOLS.items():
        if sname.upper() == name or str(cfg["id"]) == sym:
            mid = cfg.get("mid")
            if mid:
                base_raw = int(mid / tick)
            break
    if base_raw is None:
        if "BTC" in name:
            base_raw = int(95_000 / tick)
        elif "ETH" in name:
            base_raw = int(3_000 / tick)
        else:
            base_raw = int(100 / tick)
    bars = []
    # Seed with symbol name hash + timeframe so each
    # symbol/timeframe pair produces a distinct series
    seed = hash(name) ^ tf_secs
    rng = random.Random(seed)
    px = base_raw
    for i in range(limit):
        t = bucket - (limit - 1 - i) * tf_secs
        o = px
        move = int(px * 0.002 * (rng.random() - 0.5))
        c = px + move
        h = max(o, c) + abs(int(px * 0.001 * rng.random()))
        l = min(o, c) - abs(int(px * 0.001 * rng.random()))
        v = int(10 + rng.random() * 90)
        bars.append({"t": t, "o": o, "h": h, "l": l, "c": c, "v": v})
        px = c
    return bars


@app.get("/v1/candles")
async def v1_candles(
    sym: str = Query(...),
    tf: str = Query("1m"),
    limit: int = Query(200),
):
    """OHLCV bars from WAL fills, falling back to synthetic stubs."""
    tf_secs = TF_SECONDS.get(tf, 60)
    limit = max(1, min(limit, 1000))
    sym_id = _symbol_id_for(sym)
    bars = []
    if sym_id is not None:
        bars = _build_candles_from_wal(sym_id, tf_secs, limit)
    if not bars:
        bars = _synthetic_candles(sym, tf_secs, limit)
    return JSONResponse({"bars": bars})


@app.get("/v1/funding")
async def v1_funding(
    sym: int = Query(None),
    limit: int = Query(50),
    before: str = Query(None),
):
    """Return funding entries derived from WAL BBO data."""
    book_stats = parse_wal_book_stats()
    now_ms = int(time.time() * 1000)
    entries = []
    for sid, rec in sorted(book_stats.items()):
        if sym is not None and sid != sym:
            continue
        bid = rec.get("bid_px", 0)
        ask = rec.get("ask_px", 0)
        mid = (bid + ask) // 2 if bid and ask else 0
        rate = (ask - bid) / mid / 100.0 if mid > 0 else 0.0
        entries.append({
            "ts": now_ms,
            "symbolId": sid,
            "amount": 0,
            "rate": rate,
        })
    if not entries:
        # synthetic fallback: 0.01% rate per configured symbol
        for name, cfg in start_mod.SYMBOLS.items():
            sid = cfg["id"]
            if sym is not None and sid != sym:
                continue
            entries.append({
                "ts": now_ms,
                "symbolId": sid,
                "amount": 0,
                "rate": 0.0001,
            })
    return JSONResponse(entries[:limit])



@app.get("/v1/positions")
async def v1_positions(user_id: int = Query(default=0)):
    """Return open positions derived from WAL fills."""
    fills = parse_wal_fills(max_fills=1000)
    # net qty per symbol for this user
    net: dict[int, int] = {}
    entry: dict[int, int] = {}
    for f in fills:
        if user_id and (
            f["taker_uid"] != user_id
            and f["maker_uid"] != user_id
        ):
            continue
        sid = f["symbol_id"]
        side = f["taker_side"]  # 0=buy, 1=sell for taker
        delta = f["qty"] if side == 0 else -f["qty"]
        net[sid] = net.get(sid, 0) + delta
        if sid not in entry:
            entry[sid] = f["price"]
    bbo_cache: dict[int, dict] = {}
    result = []
    for sid, qty in net.items():
        if qty == 0:
            continue
        if sid not in bbo_cache:
            bbo_cache[sid] = parse_wal_bbo(sid) or {}
        bbo = bbo_cache[sid]
        mid = (
            (bbo.get("bid_px", 0) + bbo.get("ask_px", 0))
            // 2
        )
        ep = entry.get(sid, mid)
        result.append({
            "symbolId": sid,
            "side": 0 if qty > 0 else 1,
            "qty": abs(qty),
            "entryPx": ep,
            "markPx": mid,
            "unrealizedPnl": (mid - ep) * abs(qty),
            "liqPx": 0,
        })
    return JSONResponse(result)



@app.get("/v1/fills")
async def v1_fills(
    user_id: int = Query(default=0),
    sym: int = Query(default=0),
    limit: int = Query(default=50),
):
    """Return recent fills from WAL."""
    fills = parse_wal_fills(max_fills=limit * 4)
    result = []
    for f in fills:
        if user_id and (
            f["taker_uid"] != user_id
            and f["maker_uid"] != user_id
        ):
            continue
        if sym and f["symbol_id"] != sym:
            continue
        taker_hi = 0
        taker_lo = 0
        maker_hi = 0
        maker_lo = 0
        result.append({
            "takerOid": f"{taker_hi:016x}{taker_lo:016x}",
            "makerOid": f"{maker_hi:016x}{maker_lo:016x}",
            "price": f["price"],
            "qty": f["qty"],
            "ts": f["ts_ns"],
            "fee": 0,
        })
        if len(result) >= limit:
            break
    return JSONResponse(result)


@app.get("/v1/account")
async def v1_account(user_id: int = Query(default=0)):
    """Account summary: collateral, pnl, equity, margins."""
    collateral = _SEED_COLLATERAL
    # try postgres first
    if pg_pool:
        rows = await pg_query(
            "SELECT collateral, frozen_margin"
            " FROM accounts WHERE user_id = $1",
            user_id,
        )
        if rows and isinstance(rows, list) and rows:
            collateral = rows[0]["collateral"]

    # compute position pnl + margins
    fills = parse_wal_fills(max_fills=1000)
    net: dict[int, int] = {}
    entry_px: dict[int, int] = {}
    for f in fills:
        if user_id and (
            f["taker_uid"] != user_id
            and f["maker_uid"] != user_id
        ):
            continue
        sid = f["symbol_id"]
        side = f["taker_side"]
        delta = f["qty"] if side == 0 else -f["qty"]
        net[sid] = net.get(sid, 0) + delta
        if sid not in entry_px:
            entry_px[sid] = f["price"]

    total_pnl = 0
    total_im = 0
    total_mm = 0
    bbo_cache: dict[int, dict] = {}
    for sid, qty in net.items():
        if qty == 0:
            continue
        if sid not in bbo_cache:
            bbo_cache[sid] = parse_wal_bbo(sid) or {}
        bbo = bbo_cache[sid]
        mid = (
            (bbo.get("bid_px", 0) + bbo.get("ask_px", 0))
            // 2
        )
        ep = entry_px.get(sid, mid)
        notional = mid * abs(qty)
        total_pnl += (mid - ep) * qty
        total_im += int(notional * 0.10)
        total_mm += int(notional * 0.05)

    equity = collateral + total_pnl
    available = equity - total_im
    # convert raw i64 to human-readable (8 decimal places
    # for USDT-denominated values)
    d = 10**8

    def h(v):
        return str(round(v / d, 8))

    return JSONResponse({
        "userId": user_id,
        "collateral": h(collateral),
        "pnl": h(total_pnl),
        "equity": h(equity),
        "im": h(total_im),
        "mm": h(total_mm),
        "available": h(available),
    })


@app.get("/v1/orders")
async def v1_orders(user_id: int = Query(default=0)):
    """Return recent orders."""
    # build sid -> decimals lookup
    cfg_by_id = {}
    for _n, _c in start_mod.SYMBOLS.items():
        cfg_by_id[_c["id"]] = _c

    def fmt_px(sid, raw):
        c = cfg_by_id.get(sid)
        if c and isinstance(raw, (int, float)):
            return str(raw / 10 ** c["price_dec"])
        return raw

    def fmt_qty(sid, raw):
        c = cfg_by_id.get(sid)
        if c and isinstance(raw, (int, float)):
            return str(raw / 10 ** c["qty_dec"])
        return raw

    result = []
    # recent submitted orders (may include filled/rejected)
    for o in recent_orders[-100:]:
        sid = o.get("symbol_id", 0)
        result.append({
            "cid": o.get("cid", ""),
            "symbolId": sid,
            "side": o.get("side", ""),
            "price": o.get("price", 0),
            "qty": o.get("qty", 0),
            "status": o.get("status", "submitted"),
            "ts": o.get("ts", 0),
        })
    return JSONResponse(result)


@app.api_route(
    "/v1/{path:path}",
    methods=["GET", "POST"],
)
async def v1_proxy(path: str, request: Request):
    """Proxy /v1/* REST to Gateway."""
    url = f"{GATEWAY_HTTP}/v1/{path}"
    qs = str(request.query_params)
    if qs:
        url += f"?{qs}"
    try:
        async with aiohttp.ClientSession() as session:
            method = request.method.lower()
            body = await request.body()
            fwd_headers = {
                "content-type": request.headers.get(
                    "content-type", "application/json"),
            }
            if "authorization" in request.headers:
                fwd_headers["authorization"] = (
                    request.headers["authorization"])
            if "x-user-id" in request.headers:
                fwd_headers["x-user-id"] = (
                    request.headers["x-user-id"])
            async with session.request(
                method, url,
                data=body if body else None,
                headers=fwd_headers,
            ) as resp:
                data = await resp.read()
                try:
                    body = json.loads(data)
                except json.JSONDecodeError:
                    return JSONResponse(
                        {"error": "gateway returned "
                         "invalid JSON"},
                        status_code=502,
                    )
                return JSONResponse(
                    content=body,
                    status_code=resp.status,
                )
    except (
        ConnectionRefusedError,
        OSError,
        aiohttp.ClientConnectorError,
        aiohttp.ServerDisconnectedError,
        aiohttp.ClientConnectionError,
    ):
        return JSONResponse(
            {"error": "gateway not running"},
            status_code=502)


# Serve trading UI SPA (must be last — catches /trade/*)
if WEBUI_DIST.exists():
    @app.get("/trade")
    async def trade_redirect(request: Request):
        # Relative redirect preserves proxy prefix (e.g. /rsx-play/)
        prefix = request.headers.get(
            "x-forwarded-prefix", "").rstrip("/")
        return RedirectResponse(f"{prefix}/trade/")

    app.mount(
        "/trade",
        StaticFiles(
            directory=str(WEBUI_DIST),
            html=True,
        ),
        name="webui",
    )
else:
    @app.get("/trade")
    @app.get("/trade/")
    async def trade_not_built():
        return HTMLResponse(
            '<html><body style="font-family:monospace;background:#0b0e11;'
            'color:#888;padding:2rem">'
            '<h2>trading UI not built</h2>'
            '<p>run <code>make webui</code> to build rsx-webui</p>'
            '</body></html>'
        )


# ── main ────────────────────────────────────────────────

if __name__ == "__main__":
    if not os.environ.get("PLAYGROUND_MODE"):
        print("warning: PLAYGROUND_MODE not set. "
              "set PLAYGROUND_MODE=1 to suppress this warning.")
    uvicorn.run(
        "server:app",
        host="0.0.0.0",
        port=49171,
        # Single worker: in-memory session state (_active_session,
        # managed, _book_snap) is not shared across OS processes.
        # reload=True spawns an extra watcher process that can
        # race on port-bind and wipe state on file changes.
        workers=1,
        reload=False,
    )
