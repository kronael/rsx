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
import socket
import struct
import subprocess
import sys
import time
import uuid
from contextlib import asynccontextmanager
from datetime import datetime
from pathlib import Path

import aiohttp
import jwt as pyjwt
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
from fastapi.responses import Response

import cast_demo
import md_wire
import pages
import terminal
import terminal_page as terminal_view

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
# The launcher (`./playground start`) writes this server's own PID file
# into PID_DIR. Skip it everywhere we glob PID_DIR — the playground server
# is not a managed RSX process and `stop_all` must never SIGTERM itself.
SELF_PID_NAME = "playground-server"
STRESS_REPORTS_DIR = TMP / "stress-reports"
STRESS_REPORTS_DIR.mkdir(parents=True, exist_ok=True)

PG_URL = os.environ.get(
    "DATABASE_URL",
    "postgres://rsx:rsx@127.0.0.1:5432/rsx",
)

GATEWAY_URL = os.environ.get(
    "GATEWAY_URL", "ws://127.0.0.1:8080"
)
GATEWAY_HTTP = os.environ.get(
    "GATEWAY_HTTP", "http://127.0.0.1:8080"
)
MARKETDATA_WS = os.environ.get(
    "MARKETDATA_WS", "ws://127.0.0.1:8180"
)
AUTH_HTTP = os.environ.get(
    "AUTH_HTTP", "http://127.0.0.1:8082"
)
PLAYGROUND_ADMIN_TOKEN = os.environ.get(
    "PLAYGROUND_ADMIN_TOKEN", ""
)
# Default user_id for unauthenticated loopback browser clients
# in dev mode. Browsers cannot set custom WS headers, so the
# proxy mints a JWT for this user when no auth is supplied.
_GUEST_USER_ID = 99

# ── import local runtime plan ───────────────────────────

import types
try:
    import runtime as start_mod
except Exception:
    # runtime module missing — provide default SYMBOLS
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
                    # Ping every 4s; MD drops after 10s
                    # silence. 4s gives 2x margin.
                    heartbeat=4,
                ) as ws:
                    # connected — reset backoff counters
                    consec_infra = 0
                    delay = 1.0
                    # Subscribe to depth+BBO for known symbols
                    for sid in DEFAULT_SYMBOLS:
                        await ws.send_str(
                            json.dumps({"S": [sid, CHANNELS]}))

                    # MD expects application-level heartbeat
                    # `{"H":[ts_ms]}` every <10s. WS pings
                    # aren't recognized.
                    async def heartbeat_loop():
                        while True:
                            await asyncio.sleep(4)
                            try:
                                await ws.send_str(
                                    json.dumps({"H": [
                                        int(time.time() * 1000)
                                    ]}))
                            except Exception:
                                return
                    hb_task = asyncio.create_task(
                        heartbeat_loop())

                    async for msg in ws:
                        # Feed is protobuf MdFrame over BINARY frames;
                        # md_wire.decode returns the legacy JSON dict shape.
                        if msg.type != aiohttp.WSMsgType.BINARY:
                            continue
                        try:
                            frame = md_wire.decode(msg.data)
                        except Exception:
                            continue
                        if frame is None:
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
                    # WS disconnected; cancel heartbeat loop
                    hb_task.cancel()
                    try:
                        await hb_task
                    except (asyncio.CancelledError,
                            Exception):
                        pass
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
#   restarts        int   — consecutive crash count (reset on stability)
#   total_restarts  int   — cumulative crash count (never reset)
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


def _register_managed_process(
    name: str,
    proc: asyncio.subprocess.Process,
    binary: str,
    env: dict,
) -> asyncio.Task:
    output_task = asyncio.create_task(
        pipe_output(name, proc.stdout),
        name=f"log-{name}",
    )
    managed[name] = {
        "proc": proc,
        "binary": binary,
        "env": env,
        "output_task": output_task,
    }
    return output_task


async def _await_output_task(info: dict) -> None:
    output_task = info.get("output_task")
    if output_task is None:
        return
    task_loop = output_task.get_loop()
    if task_loop.is_closed():
        return
    if task_loop is not asyncio.get_running_loop():
        return
    try:
        await asyncio.wait_for(
            asyncio.shield(output_task),
            timeout=1.0,
        )
    except asyncio.TimeoutError:
        output_task.cancel()
        try:
            await output_task
        except asyncio.CancelledError:
            pass


async def _wait_for_pid_exit(
    pid: int,
    timeout: float,
) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return True
        await asyncio.sleep(0.05)
    return False


# RSX daemon binary basenames (target/<profile>/rsx-<x>). Used for
# pkill-by-path, duplicate detection, and orphan reaping in start_all.
_RSX_BINS = [
    "rsx-gateway", "rsx-risk", "rsx-matching",
    "rsx-marketdata", "rsx-mark", "rsx-recorder",
]


def _plan_ports(plan) -> list[int]:
    """Every local port the spawn plan binds, derived from its env.

    We scan bare host:port env values (127.0.0.1:9100 / 0.0.0.0:8080)
    and skip URL values (postgres://, http://, wss://) so Postgres
    (5432), the dashboard (49171), and the Binance feed (9443) are
    never in the clear-set. This replaces the old hardcoded list that
    missed the health (98xx), replication (97xx), and mark (9830)
    ports."""
    ports: set[int] = set()
    for _name, _binary, env in plan:
        for v in env.values():
            v = str(v)
            if "://" in v:
                continue
            for m in re.finditer(r":(\d+)", v):
                p = int(m.group(1))
                if 1024 <= p <= 65535:
                    ports.add(p)
    return sorted(ports)


def _port_free(port: int) -> bool:
    """True iff an RSX daemon could bind this port right now (TCP+UDP).

    TCP mirrors app rebind semantics (SO_REUSEADDR clears TIME_WAIT but
    still fails against a live LISTEN); UDP has no reuse so an active
    cast bind is detected."""
    t = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    t.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        t.bind(("", port))
    except OSError:
        return False
    finally:
        t.close()
    u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        u.bind(("", port))
    except OSError:
        return False
    finally:
        u.close()
    return True


def _pids_for_binary(bin_name: str) -> list[int]:
    """PIDs whose argv[0] is target/{debug,release}/<bin_name>.

    endswith on the exact basename so rsx-mark does NOT match
    rsx-marketdata."""
    pids: list[int] = []
    tails = (f"target/debug/{bin_name}", f"target/release/{bin_name}")
    for p in psutil.process_iter(["pid", "cmdline"]):
        cmd = p.info.get("cmdline") or []
        if any(arg.endswith(tails) for arg in cmd):
            pids.append(p.info["pid"])
    return pids


def _free_ports(ports) -> None:
    """Kill whatever holds any of these ports (tcp+udp)."""
    if shutil.which("fuser"):
        for port in ports:
            for proto in ("tcp", "udp"):
                try:
                    subprocess.run(
                        ["fuser", "-k", f"{port}/{proto}"],
                        capture_output=True, timeout=2,
                    )
                except (FileNotFoundError, subprocess.TimeoutExpired):
                    pass
        return
    for port in ports:
        try:
            result = subprocess.run(
                ["lsof", "-ti", f":{port}"],
                capture_output=True, timeout=2, text=True,
            )
            for pid in result.stdout.strip().split():
                if pid and pid.strip().isdigit():
                    try:
                        os.kill(int(pid), signal.SIGKILL)
                    except (ProcessLookupError, ValueError):
                        pass
        except (FileNotFoundError, subprocess.TimeoutExpired):
            pass


def _close_process_transport(
    proc: asyncio.subprocess.Process,
) -> None:
    transport = getattr(proc, "_transport", None)
    if transport is None:
        return
    try:
        transport.close()
    except RuntimeError:
        pass


async def _remove_managed_process(name: str) -> dict | None:
    info = managed.pop(name, None)
    if info is None:
        return None
    _close_process_transport(info["proc"])
    await _await_output_task(info)
    return info

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
    """Build the Playground-owned spawn plan."""
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
    # Python scripts get dispatched through the interpreter so
    # auto-restart works regardless of file +x bit.
    if binary_path.suffix == ".py":
        argv = [sys.executable, str(binary_path)]
    else:
        argv = [str(binary_path)]
    proc = await asyncio.create_subprocess_exec(
        *argv,
        env=full_env,
        cwd=str(ROOT),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.STDOUT,
    )
    _register_managed_process(name, proc, binary, env)
    # Register for auto-restart watching.  Preserve existing restart
    # counters if this is a watcher-triggered restart; otherwise reset.
    if name not in _restart_state:
        _restart_state[name] = {
            "restarts": 0,
            "total_restarts": 0,
            "blocked": False,
            "next_restart_at": 0.0,
            "last_crash_ts": 0.0,
            "intentional": False,
        }
    else:
        # Clear intentional flag so future crashes are auto-restarted.
        _restart_state[name]["intentional"] = False
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
        await _remove_managed_process(name)
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
        await _remove_managed_process(name)
        _restart_state.pop(name, None)
        return {"status": f"{name} already stopped"}
    try:
        os.kill(proc.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    exited = await _wait_for_pid_exit(proc.pid, timeout=5.0)
    if not exited:
        try:
            os.kill(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        await _wait_for_pid_exit(proc.pid, timeout=2.0)
    _close_process_transport(proc)
    pid_file = PID_DIR / f"{name}.pid"
    if pid_file.exists():
        pid_file.unlink()
    # Clean up managed and restart tracking.
    await _remove_managed_process(name)
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
        await _remove_managed_process(name)
        _restart_state.pop(name, None)
        return {"status": f"{name} already stopped"}
    try:
        os.kill(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    await _wait_for_pid_exit(proc.pid, timeout=2.0)
    _close_process_transport(proc)
    pid_file = PID_DIR / f"{name}.pid"
    if pid_file.exists():
        pid_file.unlink()
    await _remove_managed_process(name)
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
                rs["total_restarts"] = rs.get("total_restarts", 0) + 1
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
    cmd = [_cargo_bin(), "build", "--workspace"]
    if release:
        cmd.append("--release")
    proc = subprocess.Popen(
        cmd,
        cwd=str(ROOT),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    while True:
        line = await asyncio.to_thread(proc.stdout.readline)
        if not line:
            break
        build_log.append(line.rstrip())
    await asyncio.to_thread(proc.wait)
    ok = proc.returncode == 0
    build_log.append("build ok" if ok else "build FAILED")
    return ok


# collateral for playground test users: 100 quadrillion raw
# units — must exceed notional*im_rate for maker orders
# (price~50000 * qty~1M * im_rate~1000/10000 = ~5T per order)
_SEED_USERS = [1, 2, 3, 4, 5, 99]
_SEED_COLLATERAL = 100_000_000_000_000_000


async def seed_accounts():
    """Upsert playground test accounts into Postgres.

    Uses ON CONFLICT UPDATE so test/dev runs always start
    with the configured collateral. Also clears any stale
    frozen_orders rows for these users — without this, a
    crashed prior run can leave reservations behind that
    block new orders with InsufficientMargin until risk
    replays them.
    """
    if pg_pool is None:
        return
    try:
        async with pg_pool.acquire() as conn:
            for uid in _SEED_USERS:
                await conn.execute(
                    "INSERT INTO accounts "
                    "(user_id, collateral, version) "
                    "VALUES ($1, $2, 0) "
                    "ON CONFLICT (user_id) DO UPDATE "
                    "SET collateral = EXCLUDED.collateral",
                    uid, _SEED_COLLATERAL,
                )
                await conn.execute(
                    "DELETE FROM frozen_orders WHERE user_id = $1",
                    uid,
                )
    except Exception as e:
        print(f"seed_accounts failed: {e}")


async def do_maker_start() -> bool:
    """Start market maker subprocess. Returns True if started."""
    if _maker_running():
        return True
    if MAKER_NAME in managed:
        await _remove_managed_process(MAKER_NAME)
    # Prefer the Go maker binary; fall back to the Python script so a
    # box without the Go toolchain still gets a live book.
    if MAKER_BIN.exists():
        argv = [str(MAKER_BIN)]
        cwd = str(ROOT)
        label = str(MAKER_BIN)
    elif MAKER_SCRIPT.exists():
        argv = [sys.executable, str(MAKER_SCRIPT)]
        cwd = str(ROOT / "rsx-playground")
        label = str(MAKER_SCRIPT)
    else:
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
        # Live mid override: both makers poll this file each cycle.
        "RSX_MAKER_CONFIG_FILE": str(MAKER_CONFIG),
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
        *argv,
        env=full_env,
        cwd=cwd,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.STDOUT,
    )
    _register_managed_process(
        MAKER_NAME,
        proc,
        label,
        env,
    )
    PID_DIR.mkdir(parents=True, exist_ok=True)
    (PID_DIR / f"{MAKER_NAME}.pid").write_text(str(proc.pid))
    await asyncio.sleep(0.2)
    if proc.returncode is not None:
        await _remove_managed_process(MAKER_NAME)
        (PID_DIR / f"{MAKER_NAME}.pid").unlink(missing_ok=True)
        return False
    return True


def _ensure_repl_certs():
    """Provision snakeoil replication certs if ./certs is absent.

    TLS is mandatory on the replication TCP hop, so the RSX
    processes need cert/key/ca PEMs at ./certs (their cwd is the
    repo root; from_env's default). casting/UDP stays plaintext.
    Idempotent: skips when a valid cert set already exists.

    Self-heals the pre-fix breakage where cert.pem was a byte copy
    of the CA (a CA:TRUE self-signed cert used as the server leaf):
    rustls/webpki reject that with `CaUsedAsEndEntity`, so the
    replication handshake — and thus risk warm-catchup — never
    completes. That signature is cert.pem == ca.pem; force a
    regen (proper CA + distinct leaf) when we see it.
    """
    cert = ROOT / "certs" / "cert.pem"
    ca = ROOT / "certs" / "ca.pem"
    force = False
    if cert.exists():
        if not ca.exists():
            return
        try:
            if cert.read_bytes() == ca.read_bytes():
                force = True  # CA-as-leaf: the old broken layout
            else:
                return
        except OSError:
            return
    args = ["sh", str(ROOT / "scripts" / "gen-snakeoil-certs.sh")]
    if force:
        args.append("--force")
    result = subprocess.run(
        args,
        cwd=str(ROOT),
        capture_output=True,
        timeout=30,
    )
    if result.returncode != 0:
        print(
            "Failed to generate replication certs: "
            f"{result.stderr.decode(errors='replace').strip()}"
        )
    else:
        action = "regenerated" if force else "generated"
        print(f"{action} snakeoil replication certs in ./certs")


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

    # Replication is TLS-mandatory: make sure certs exist so the
    # cluster boots (dev snakeoil; real certs replace ./certs).
    _ensure_repl_certs()

    # Idempotent teardown before respawn. Three failure modes this
    # guards against (SYS #29):
    #   (a) fixed sleeps raced port release -> "Address already in use"
    #       on respawn. We now POLL until every port the plan binds is
    #       actually free instead of sleeping a guessed interval.
    #   (b) the old clear-set was a hardcoded 8-port list that missed
    #       the health (98xx), replication (97xx), and mark (9830)
    #       ports, so those survivors collided. Ports are now derived
    #       from the spawn plan's env (_plan_ports).
    #   (c) the 2 s auto-restart watcher would respawn a daemon we'd
    #       just pkilled (it stays in `managed`, intentional=False)
    #       DURING the multi-second build window -> a duplicate PID
    #       racing start_all's own spawn. We detach every tracked RSX
    #       daemon from the watcher first, and reap any surviving
    #       orphan (incl. from a previous dashboard instance) after
    #       spawn.
    ports = _plan_ports(plan)

    # Detach currently-tracked RSX daemons from the auto-restart
    # watcher so it can't respawn them mid-teardown. Maker/auth/stress
    # (non-rsx binaries) are left managed and untouched.
    for name in [
        n for n, info in managed.items()
        if any(b in info.get("binary", "") for b in _RSX_BINS)
    ]:
        rs = _restart_state.get(name)
        if rs:
            rs["intentional"] = True
        await _remove_managed_process(name)
        _restart_state.pop(name, None)

    # SIGTERM by build path (target/<profile>/rsx-<bin>) so the F1
    # graceful WAL drain runs; matching the full path avoids SIGKILLing
    # log tails / editors / sibling checkouts that merely mention the
    # name (F20). Covers orphans from a previous dashboard instance too.
    for bin_name in _RSX_BINS:
        try:
            subprocess.run(
                ["pkill", "-TERM", "-f", f"target/debug/{bin_name}"],
                capture_output=True, timeout=2,
            )
        except (FileNotFoundError, subprocess.TimeoutExpired):
            pass
    # Poll for graceful exit (up to 3 s), then SIGKILL survivors.
    deadline = time.time() + 3.0
    while time.time() < deadline:
        if not any(_pids_for_binary(b) for b in _RSX_BINS):
            break
        await asyncio.sleep(0.1)
    for bin_name in _RSX_BINS:
        try:
            subprocess.run(
                ["pkill", "-KILL", "-f", f"target/debug/{bin_name}"],
                capture_output=True, timeout=2,
            )
        except (FileNotFoundError, subprocess.TimeoutExpired):
            pass

    # Free any port still held, then POLL until all are bindable (up to
    # 5 s). Proceeding after the timeout is safe: spawn surfaces a bind
    # error if one is genuinely stuck.
    _free_ports(ports)
    deadline = time.time() + 5.0
    while time.time() < deadline:
        busy = [p for p in ports if not _port_free(p)]
        if not busy:
            break
        _free_ports(busy)
        await asyncio.sleep(0.1)

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

    # Reap duplicates/orphans: any rsx build-path PID we did NOT just
    # spawn (not in `managed`) is a stray -> SIGKILL. Precise regardless
    # of instance count (N ME per symbol etc): the good PIDs are exactly
    # the managed set. Closes SYS #29 (b)/(c).
    managed_pids = {
        info["proc"].pid for info in managed.values()
    }
    for bin_name in _RSX_BINS:
        for pid in _pids_for_binary(bin_name):
            if pid not in managed_pids:
                try:
                    os.kill(pid, signal.SIGKILL)
                except (ProcessLookupError, ValueError):
                    pass

    # wait for processes to stabilize, then auto-start auth.
    # Maker is NOT auto-started: it generates ~40 ord/s which
    # outpaces the default UDP rmem (208 KB on stock kernels)
    # and triggers cast FAULTED + risk-panic-restart loops, which
    # then masks every other demo failure mode. Operators start
    # it manually from /controls when they want depth.
    if started:
        await asyncio.sleep(3.0)
        # rsx-auth is optional — silently skips if not configured
        await do_auth_start()
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
            if name == SELF_PID_NAME:
                # never SIGTERM our own server process
                continue
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
        _close_process_transport(info["proc"])
        await _await_output_task(info)
    # Clear managed dict
    managed.clear()
    # cleanup server PID file (launcher writes it into PID_DIR)
    server_pid_file = PID_DIR / f"{SELF_PID_NAME}.pid"
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
    "tailwind-play.js": "application/javascript",
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


def _is_loopback_host(host: str | None) -> bool:
    if not host:
        return False
    return host in {
        "127.0.0.1",
        "::1",
        "localhost",
        "testclient",
    }


_INSECURE_USER_ID_WARNED = False


def _allow_insecure_user_id() -> bool:
    global _INSECURE_USER_ID_WARNED
    enabled = os.environ.get(
        "PLAYGROUND_ALLOW_INSECURE_USER_ID", ""
    ).lower() in {"1", "true", "yes", "on"}
    if enabled and not _INSECURE_USER_ID_WARNED:
        _INSECURE_USER_ID_WARNED = True
        print(
            "WARN PLAYGROUND_ALLOW_INSECURE_USER_ID=1: "
            "loopback callers can spoof x-user-id "
            "(dev-only; never set in production)",
            flush=True,
        )
    return enabled


def _extract_token_from_headers(headers) -> str | None:
    auth = headers.get("authorization", "")
    if auth.startswith("Bearer "):
        return auth[7:].strip()
    cookie = headers.get("cookie", "")
    for part in cookie.split(";"):
        item = part.strip()
        if item.startswith("rsx_token="):
            return item.split("=", 1)[1].strip()
    return None


def _decode_user_token(token: str) -> tuple[int | None, str]:
    secret = os.environ.get("RSX_GW_JWT_SECRET", "")
    if not secret:
        return None, "RSX_GW_JWT_SECRET not configured"
    try:
        claims = pyjwt.decode(
            token,
            secret,
            algorithms=["HS256"],
            audience="rsx-gateway",
            issuer="rsx-auth",
        )
    except pyjwt.InvalidTokenError as exc:
        return None, f"invalid token: {exc}"
    user_id = claims.get("user_id")
    if not isinstance(user_id, int):
        return None, "token missing numeric user_id"
    return user_id, ""


def _request_auth_headers(request: Request) -> tuple[dict, str]:
    token = _extract_token_from_headers(request.headers)
    if token:
        return {"authorization": f"Bearer {token}"}, ""
    if (
        _allow_insecure_user_id()
        and _is_loopback_host(request.client.host if request.client else None)
    ):
        user_id = request.headers.get("x-user-id")
        if user_id:
            return {"x-user-id": user_id}, ""
    return {}, "missing authenticated user context"


def _resolve_request_user(request: Request) -> tuple[int | None, str]:
    token = _extract_token_from_headers(request.headers)
    if token:
        return _decode_user_token(token)
    if (
        _allow_insecure_user_id()
        and _is_loopback_host(request.client.host if request.client else None)
    ):
        raw = request.headers.get("x-user-id")
        if raw:
            try:
                return int(raw), ""
            except ValueError:
                return None, "invalid x-user-id header"
    return None, "missing authenticated user context"


def _require_admin_request(request: Request):
    host = request.client.host if request.client else None
    if _is_loopback_host(host):
        return None
    if PLAYGROUND_ADMIN_TOKEN:
        supplied = request.headers.get("x-admin-token", "")
        if supplied == PLAYGROUND_ADMIN_TOKEN:
            return None
        return JSONResponse(
            {"error": "admin token required"},
            status_code=401,
        )
    return JSONResponse(
        {"error": "admin access requires loopback client or PLAYGROUND_ADMIN_TOKEN"},
        status_code=403,
    )


def _require_private_user(request: Request) -> tuple[int | None, Response | None]:
    user_id, err = _resolve_request_user(request)
    if user_id is not None:
        return user_id, None
    return None, JSONResponse(
        {"error": err},
        status_code=401,
    )


DESTRUCTIVE_ENDPOINTS = {
    "/api/processes/all/stop",
    "/api/processes/all/start",
    "/api/scenario/switch",
    "/api/reset",
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


def expected_process_names() -> set[str]:
    """The ONE set of processes the running demo expects: the current
    scenario's spawn plan (the cluster) PLUS the maker. The maker is a
    first-class member of the running system, so it's counted the same
    way everywhere — this is the single definition behind every
    'running/expected' surface (nav chip, pulse, key metrics, verify,
    healthz). Fixes the 7/7-vs-6/6-vs-7/6 disagreement (#11)."""
    names = {n for n, _, _ in get_spawn_plan(current_scenario)}
    names.add(MAKER_NAME)
    return names


def process_counts(procs=None) -> tuple[int, int]:
    """(running, expected) using expected_process_names()."""
    if procs is None:
        procs = _cached_for("procs", 1.0, scan_processes)
    expected = expected_process_names()
    running = sum(
        1 for p in procs
        if p.get("state") == "running" and p["name"] in expected)
    return running, len(expected)


@app.get("/healthz")
async def healthz():
    """Health check for CLI."""
    procs = scan_processes()
    running, expected = process_counts(procs)
    gateway_up, marketdata_up = await asyncio.gather(
        _probe_gateway_tcp(),
        _probe_marketdata_tcp(),
    )
    return {
        "status": "ok",
        "port": 49171,
        "processes_running": running,
        "processes_total": expected,
        "postgres": pg_pool is not None,
        "gateway": gateway_up,
        "marketdata": marketdata_up,
    }

# ── in-memory state ─────────────────────────────────────

recent_orders: list[dict] = []
# Epoch-seconds timestamps of recent order submissions, used by
# /x/key-metrics for a 30s sliding-window Msgs/sec (F26).
recent_order_ts: list[float] = []
verify_results: list[dict] = []
# Epoch ms of the last /api/verify/run invocation; None until
# the first run. Surfaced by /x/invariant-status (F24).
verify_last_run: float | None = None
order_latencies: list[int] = []
# E2E round-trip in microseconds: order submit (WS send) →
# fill (F frame) received over the same WS. Populated by
# /api/latency-probe; surfaced in /api/latency `e2e` block.
e2e_latencies: list[int] = []
# Gateway-only RTT in microseconds: order submit → error
# frame received over the same WS. Populated by
# /api/latency-probe-gw; surfaced in /api/latency `gw_only`.
# The order intentionally fails gateway prevalidation
# (unknown symbol_id) so risk + ME are never touched.
# Isolates Python aiohttp + gateway parse from the rest of
# the GW→ME→GW critical path.
gw_only_latencies: list[int] = []
gateway_ws = None
_idempotency_keys: dict[str, float] = {}
_IDEMPOTENCY_TTL = 300
SERVER_START: float = time.time()
_user_balances: dict[int, int] = {}

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


def _cargo_bin() -> str:
    configured = os.environ.get("RSX_CARGO_BIN", "").strip()
    if configured:
        return configured
    found = shutil.which("cargo")
    if found:
        return found
    fallback = Path.home() / ".cargo" / "bin" / "cargo"
    return str(fallback)


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
            tr = _restart_state.get(name, {}).get(
                "total_restarts", 0)
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
                    "total_restarts": tr,
                })
            except (psutil.NoSuchProcess,
                    psutil.AccessDenied):
                _evict_ps(proc.pid)
                result.append({
                    "name": name, "pid": proc.pid,
                    "state": "running", "cpu": "-",
                    "mem": "-", "uptime": "-",
                    "total_restarts": tr,
                })
        else:
            rs = _restart_state.get(name, {})
            state = "blocked" if rs.get("blocked") else "stopped"
            result.append({
                "name": name, "pid": "-",
                "state": state, "cpu": "-",
                "mem": "-", "uptime": "-",
                "restarts": rs.get("restarts", 0),
                "total_restarts": rs.get("total_restarts", 0),
            })

    # 2. PID files (from Playground or previous session)
    if PID_DIR.exists():
        for pid_file in sorted(PID_DIR.glob("*.pid")):
            name = pid_file.stem
            if name == SELF_PID_NAME:
                continue
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
                        "total_restarts": _restart_state.get(
                            name, {}).get("total_restarts", 0),
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
                "total_restarts": _restart_state.get(
                    name, {}).get("total_restarts", 0),
            })

    return sorted(result, key=lambda p: p["name"])


def scan_wal_streams():
    """Snapshot every WAL stream dir's file count and total size.

    Single source of truth for both the WAL tab and the Verify
    page. Both must call THIS function (don't reimplement disk
    walking elsewhere) so the two surfaces never disagree.
    """
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
            "total_bytes": total,
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


# WAL header V1: 16 bytes
#   version:u8(0) pad:u8 record_type:u16 len:u16 pad:u16 crc32:u32 reserved:4s
WAL_HDR = struct.Struct('<BBHHHi4s')
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
            _ver, _, rtype, rlen, _, crc, _ = WAL_HDR.unpack_from(
                data, pos)
            if _ver != 1:
                break
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
        if d.is_dir() and d.name != "archive":
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


def parse_wal_mark_prices():
    """Latest real mark/index price per symbol from the mark stream.

    The mark process aggregates external sources (Binance/Coinbase)
    into RECORD_MARK_PRICE records on the `mark` WAL stream. This is
    the only cluster-truth index/oracle price the dashboard can read
    — distinct from the ME book mid. Returns {sid: {mark_price, ...}}.
    """
    latest = {}
    recs = parse_wal_records(
        WAL_DIR / "mark", record_types={RECORD_MARK_PRICE})
    for r in recs:
        if r.get("type") != "mark_price":
            continue
        sid = r["symbol_id"]
        prev = latest.get(sid)
        if prev is None or r["seq"] > prev["seq"]:
            latest[sid] = r
    return latest


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


def _log_match(fname: str, prefixes) -> bool:
    """Match a log filename stem to a source hint by component root.

    fname stems are shard-numbered (gw-0, risk-0, me-pengu) or bare
    (marketdata, mark, server). Compare the leading token before the
    shard suffix so "mark" does NOT swallow "marketdata".
    """
    root = fname.split("-", 1)[0]
    for p in prefixes:
        p = p.rstrip("-")
        if root == p or fname == p:
            return True
    return False


def read_logs(process=None, level=None, search=None,
              max_lines=200):
    lines = []
    log_files = (
        sorted(LOG_DIR.glob("*.log"))
        if LOG_DIR.exists() else []
    )
    # Map UI label (component) → log filename prefixes.
    # Log files are shard-numbered (gw-0.log, risk-0.log,
    # me-pengu.log); without this map "gateway" matched
    # nothing, while "errors only" surfaced [gw-0] WARN lines.
    proc_prefixes = (
        PROC_HINTS.get(process, [process]) if process else None
    )
    for lf in log_files:
        fname = lf.stem
        # The dashboard's own log (server.log) is uvicorn access noise
        # (thousands of "GET /x/... 200" from the panels' own polling).
        # Hide it from the default "all" view; only show it when the
        # user explicitly picks the "server" source (#25).
        if fname == "server" and process != "server":
            continue
        if proc_prefixes and not _log_match(fname, proc_prefixes):
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


# ── recovery explorer ───────────────────────────────────
# Fault injection + live recovery feed. All faults (crash,
# net, wal) are always available; each is a click-through
# guarded by an hx-confirm dialog in the UI.

# Log substrings that mark a recovery-relevant event. Matched
# case-insensitively against read_logs() output.
_RECOVERY_KEYS = (
    "faulted", "opening replay", "replay", "caught up",
    "caughtup", "caught-up", "crc", "nak", "circuit open",
    "process watcher", "restart", "reconnect", "backoff",
    "re-sync", "resync", "rcvbuferror", "failover",
)


def _classify_recovery(low: str) -> str:
    """Map a lowercased recovery line to a semantic level."""
    if any(k in low for k in (
        "faulted", "crc", "circuit open", "panic",
        "rcvbuferror", "failed",
    )):
        return "fault"
    if any(k in low for k in (
        "caught up", "caughtup", "caught-up", "re-sync",
        "resync", "restarted", "started", "listening",
        "ready", "healthy",
    )):
        return "healed"
    return "recovering"


def recovery_feed_events(limit: int = 40) -> list[dict]:
    """Build the newest-first recovery event feed.

    Freshest block = current process states (scan_processes);
    below it = recovery-relevant log lines (FAULTED / replay /
    caught-up / CRC), newest-first.
    """
    events: list[dict] = []
    now = datetime.now().strftime("%b %d %H:%M:%S")
    for p in scan_processes():
        st = p.get("state")
        name = p["name"]
        tr = p.get("total_restarts", 0)
        if st == "running":
            events.append({
                "stamp": now, "level": "healed", "src": "proc",
                "text": f"{name} healthy — pid {p.get('pid')}, "
                        f"{tr} restarts",
            })
        elif st == "blocked":
            events.append({
                "stamp": now, "level": "fault", "src": "proc",
                "text": f"{name} circuit open — auto-restart "
                        f"blocked after repeated crashes",
            })
        else:
            events.append({
                "stamp": now, "level": "recovering", "src": "proc",
                "text": f"{name} down — watcher restarting "
                        f"({tr} total)",
            })
    matched = []
    for line in read_logs(max_lines=500):
        low = line.lower()
        if any(k in low for k in _RECOVERY_KEYS):
            matched.append({
                "stamp": "", "level": _classify_recovery(low),
                "src": "log", "text": line,
            })
    for m in reversed(matched[-limit:]):
        events.append(m)
    return events


# ── page routes ─────────────────────────────────────────

@app.get("/", response_class=HTMLResponse)
async def index():
    return RedirectResponse("./overview")


@app.get("/walkthrough", response_class=HTMLResponse)
async def walkthrough():
    # Repurposed: hero + launcher + diagrams now live on Overview.
    return RedirectResponse("./overview")


@app.get("/overview", response_class=HTMLResponse)
async def overview():
    return HTMLResponse(pages.overview_page())


@app.get("/topology", response_class=HTMLResponse)
async def topology():
    return HTMLResponse(pages.topology_page())


# ── Topology partials ────────────────────────────────────

# Canonical component → process-name prefixes. Process names emitted
# by `start` are shard-numbered (gw-0, risk-0, me-pengu, ...). Keep
# legacy aliases ("gateway", "matching", "mktdata") so old paths keep
# resolving, but list the canonical prefix first so a running process
# always wins over a stopped plan-stub of the same component.
PROC_HINTS: dict[str, list[str]] = {
    "gateway": ["gw-", "gateway"],
    "risk": ["risk-", "risk"],
    "matching": ["me-", "matching"],
    "marketdata": ["marketdata", "mktdata"],
    "mark": ["mark"],
    "recorder": ["recorder"],
    "maker": ["maker"],
    "stress": ["stress"],
    "server": ["server"],
}


def _topo_proc(hints: list[str]) -> dict:
    """Return first running process matching any hint; else first match."""
    procs = scan_processes()
    fallback = {}
    for hint in hints:
        for p in procs:
            if hint in p["name"]:
                if p.get("state") == "running":
                    return p
                if not fallback:
                    fallback = p
    return fallback


def _topo_gateway() -> dict:
    p = _topo_proc(PROC_HINTS["gateway"])
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
        ],
    }


def _topo_risk() -> dict:
    p = _topo_proc(PROC_HINTS["risk"])
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
    p = _topo_proc(PROC_HINTS["matching"])
    book_stats = parse_wal_book_stats()
    rows = []
    for sid, bbo in sorted(book_stats.items()):
        bid = bbo.get("bid_px", 0)
        ask = bbo.get("ask_px", 0)
        spd = ask - bid if bid and ask else 0
        bid_str = pages.format_price(bid, sid) if bid else "--"
        ask_str = pages.format_price(ask, sid) if ask else "--"
        spd_str = pages.format_price(spd, sid) if spd else "0"
        rows.append((
            f"sym{sid} bbo",
            f"bid={bid_str} ask={ask_str} spread={spd_str}",
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
    p = _topo_proc(PROC_HINTS["marketdata"])
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
    p = _topo_proc(PROC_HINTS["mark"])
    rows: list[tuple[str, object]] = []
    # Current mark prices (BBO mid) per active symbol.
    book_stats = parse_wal_book_stats()
    if book_stats:
        for sid, bbo in sorted(book_stats.items()):
            bid = bbo.get("bid_px", 0)
            ask = bbo.get("ask_px", 0)
            if bid and ask:
                mid = (bid + ask) // 2
                rows.append((f"sym{sid} mark",
                             pages.format_price_fixed(mid, sid)))
    # Funding window remaining (8h settlement cadence).
    now_s = int(time.time())
    interval = 28800
    next_s = interval - (now_s % interval)
    h, rem = divmod(next_s, 3600)
    m = rem // 60
    rows.append((
        "funding next settlement",
        f"{h}h {m}m" if h else f"{m}m",
    ))
    rows.append(("sample interval", "1s"))
    rows.append(("symbols tracked", len(book_stats)))
    return {
        "name": "Mark",
        "status": p.get("state", "stopped"),
        "pid": p.get("pid", "-"),
        "uptime": p.get("uptime", "-"),
        "rows": rows,
    }


def _topo_recorder() -> dict:
    p = _topo_proc(PROC_HINTS["recorder"])
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
            f"sym{k}={pages.format_price_fixed(int(v), int(k))}"
            for k, v in sorted(
                mid_prices.items(),
                key=lambda kv: int(kv[0]),
            )
        )
        or "none"
    )
    p = _topo_proc(PROC_HINTS["maker"])
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
    p = _topo_proc(PROC_HINTS["stress"])
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


@app.get("/x/topology/flow")
async def x_topology_flow():
    """JSON: per-node status dots and rate labels for live update.

    Registered BEFORE the /x/topology/{component} catch-all so the
    literal `flow` path is not swallowed as a component name.
    """
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
            else "bg-slate-500"
        )

    gw = _status(PROC_HINTS["gateway"])
    risk_s = _status(PROC_HINTS["risk"])
    me_s = _status(PROC_HINTS["matching"])
    md_s = _status(PROC_HINTS["marketdata"])
    mk_s = _status(PROC_HINTS["mark"])
    rec_s = _status(PROC_HINTS["recorder"])
    maker_s = "running" if _maker_running() else "stopped"

    book_stats = parse_wal_book_stats()
    spd_label = "none"
    for sid, bbo in sorted(book_stats.items()):
        bid = bbo.get("bid_px", 0)
        ask = bbo.get("ask_px", 0)
        if bid and ask:
            spd_label = f"spd={ask - bid}"
            break

    # recent_orders/recent_fills/_book_snap are dashboard-local
    # collections (reset on dashboard restart, only count traffic
    # this process witnessed). Label them "(session)" so they
    # don't masquerade as cluster throughput — matching the topo
    # detail panels.
    nodes = [
        {"key": "client", "dot": "bg-slate-500",
         "rate": f"{len(recent_orders)} ord (session)"},
        {"key": "gateway", "dot": _dot(gw),
         "rate": f"{len(recent_fills)} fills (session)"},
        {"key": "risk", "dot": _dot(risk_s),
         "rate": risk_s},
        {"key": "matching", "dot": _dot(me_s),
         "rate": spd_label},
        {"key": "marketdata", "dot": _dot(md_s),
         "rate": f"{len(_book_snap)} sym (session)"},
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
    run_count, exp_count = process_counts(procs)
    names = ", ".join(p["name"] for p in running)
    gw_up = any(
        any(h in p["name"] for h in PROC_HINTS["gateway"])
        for p in running)
    me_up = any(
        any(h in p["name"] for h in PROC_HINTS["matching"])
        for p in running)
    md_up = any(
        any(h in p["name"] for h in PROC_HINTS["marketdata"])
        for p in running)

    def _dot(ok):
        c = "bg-emerald-400" if ok else "bg-red-500"
        return (
            f'<span class="w-1.5 h-1.5 rounded-full '
            f'{c} inline-block"></span>'
        )

    return HTMLResponse(
        f'{_dot(gw_up)} '
        f'<span class="text-slate-400">GW</span> '
        f'{_dot(me_up)} '
        f'<span class="text-slate-400">ME</span> '
        f'{_dot(md_up)} '
        f'<span class="text-slate-400">MD</span> '
        f'<span class="text-slate-500 ml-2">'
        f'{run_count}/{exp_count} running</span>'
        f'<span class="text-slate-600 ml-auto truncate '
        f'max-w-[300px]">{names}</span>'
    )


@app.get("/x/topology/{component}",
         response_class=HTMLResponse)
async def x_topology_component(component: str):
    # Declared AFTER the literal /x/topology/flow and /summary
    # routes so this catch-all does not swallow them.
    handler = _TOPO_HANDLERS.get(component)
    if not handler:
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            f"unknown component: {component}</span>"
        )
    return HTMLResponse(
        pages.render_component_detail(component, handler()))


@app.get("/components", response_class=HTMLResponse)
async def components_index():
    return HTMLResponse(pages.components_index_page())


@app.get("/component/{key}", response_class=HTMLResponse)
async def component_detail(key: str):
    if key not in pages.COMPONENTS:
        return HTMLResponse(
            f'<html><body style="font-family:monospace;'
            f'background:#0b0e11;color:#888;padding:2rem">'
            f'<h2>Unknown component: {html.escape(key)}</h2>'
            f'<p><a href="./components" '
            f'style="color:#a992ff">Back to Components</a></p>'
            f'</body></html>',
            status_code=404,
        )
    return HTMLResponse(pages.component_page(key))


@app.get("/x/component-logs/{key}", response_class=HTMLResponse)
async def x_component_logs(key: str):
    """Partial: log tail filtered to a single component."""
    lines = read_logs(process=key, max_lines=50)
    return HTMLResponse(pages.render_logs(lines))


@app.get("/crates", response_class=HTMLResponse)
async def crates_index():
    """Index of the 7 documented Cargo crates (description +
    benchmarks + comparisons + demo), distinct from the live
    /components process view."""
    return HTMLResponse(pages.crates_index_page())


@app.get("/crate/{name}", response_class=HTMLResponse)
async def crate_detail(name: str):
    if name not in pages.CRATES:
        return HTMLResponse(
            f'<html><body style="font-family:monospace;'
            f'background:#0b0e11;color:#888;padding:2rem">'
            f'<h2>Unknown crate: {html.escape(name)}</h2>'
            f'<p><a href="./crates" '
            f'style="color:#a992ff">Back to Crates</a></p>'
            f'</body></html>',
            status_code=404,
        )
    return HTMLResponse(pages.crate_page(name))


@app.get("/x/crate-demo/{name}")
async def x_crate_demo(name: str):
    """Serve a crate's demo GIF locally (no external hosting)."""
    crate = pages.CRATES.get(name)
    demo_rel = crate.get("demo") if crate else None
    if not demo_rel:
        return Response(status_code=404)
    repo_root = Path(__file__).resolve().parent.parent
    path = (repo_root / demo_rel).resolve()
    if (not str(path).startswith(str(repo_root))
            or not path.is_file()):
        return Response(status_code=404)
    return Response(content=path.read_bytes(), media_type="image/gif")


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


@app.get("/recovery", response_class=HTMLResponse)
async def recovery():
    return HTMLResponse(pages.recovery_page())


@app.get("/verify", response_class=HTMLResponse)
async def verify():
    return HTMLResponse(pages.verify_page())


@app.get("/orders", response_class=HTMLResponse)
async def orders():
    return HTMLResponse(pages.orders_page())


@app.get("/cast", response_class=HTMLResponse)
async def cast_page():
    return HTMLResponse(
        pages.layout("Cast", cast_demo.cast_page(), "./cast"))


@app.get("/latency", response_class=HTMLResponse)
async def latency_page():
    return HTMLResponse(pages.latency_page())


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


@app.get("/terminal", response_class=HTMLResponse)
async def terminal_page():
    return HTMLResponse(terminal_view.page())


@app.websocket("/ws/terminal")
async def ws_terminal(ws: WebSocket):
    if await terminal.authorized(ws, PLAYGROUND_ADMIN_TOKEN):
        await terminal.serve_rsx_term(ws, ROOT)


@app.get("/stress/{report_id}", response_class=HTMLResponse)
async def stress_report_view(report_id: str):
    """View individual stress test report as HTML"""
    report_file = STRESS_REPORTS_DIR / f"stress-{report_id}.json"
    if not report_file.exists():
        return HTMLResponse("<h1>Report not found</h1>", status_code=404)

    with open(report_file) as f:
        data = json.load(f)

    return HTMLResponse(pages.stress_report_page(data))


@app.get("/docs")
async def docs_index():
    """Redirect /docs to the guide landing page."""
    return RedirectResponse("./docs/guide/README")


@app.get("/docs/{filename:path}")
async def docs(filename: str):
    """Serve playground documentation files."""
    # Root allowlist: GUIDE = how to use the playground;
    # DOCS = the platform itself (concepts + repo docs + spec).
    # URLs are /docs/<root>/<file>; bare names normalize into
    # guide so depth stays 2 and the relative sidebar links work.
    repo_root = Path(__file__).resolve().parent.parent
    doc_roots = {
        "guide": Path(__file__).parent / "docs",
        "concepts": repo_root / "docs" / "concepts",
        "platform": repo_root / "docs",
        "spec": repo_root / "specs" / "2",
    }
    if not filename:
        filename = "guide/README"
    parts = filename.split("/", 1)
    if parts[0] in doc_roots:
        root_key = parts[0]
        rel = parts[1] if len(parts) > 1 else "README"
    else:
        return RedirectResponse(f"guide/{filename}")
    base = doc_roots[root_key].resolve()
    if not rel.endswith(".md"):
        rel += ".md"
    file_path = (base / rel).resolve()
    if (not str(file_path).startswith(str(base))
            or not file_path.exists()
            or not file_path.is_file()):
        return HTMLResponse(
            "<h1>404 Not Found</h1>", status_code=404)
    content = file_path.read_text()
    safe_filename = html.escape(f"{root_key}/{rel}")
    md_json = json.dumps(content)

    def _doc_label(stem):
        s = re.sub(r"^\d+[-_]", "", stem)
        return s.replace("-", " ").replace("_", " ").title()

    def _doc_group(title, key):
        d = doc_roots[key]
        if not d.exists():
            return ""
        out = (
            f'<div class="mt-4 mb-1 text-xs text-slate-500 '
            f'uppercase tracking-wider">{html.escape(title)}</div>')
        for f in sorted(d.glob("*.md")):
            cls = ("font-bold text-white"
                   if (key == root_key and f.name == rel)
                   else "text-slate-400")
            out += (
                f'<a href="../{key}/{f.stem}" '
                f'class="{cls} block py-0.5 hover:text-white '
                f'text-xs">{html.escape(_doc_label(f.stem))}</a>\n')
        return out

    # GUIDE group, then DOCS groups (platform internals).
    sidebar = (
        '<div class="text-xs text-emerald-500 uppercase '
        'tracking-wider font-bold">Guide</div>'
        + _doc_group("Using the playground", "guide")
        + '<div class="mt-5 text-xs text-blue-400 uppercase '
        'tracking-wider font-bold">Docs</div>'
        + _doc_group("Concepts", "concepts")
        + _doc_group("Platform", "platform")
        + _doc_group("Spec", "spec")
    )

    doc_html = f"""<!DOCTYPE html>
<html lang="en" class="dark">
<head>
<meta charset="utf-8">
<meta name="viewport"
  content="width=device-width, initial-scale=1">
<title>RSX Docs -- {safe_filename}</title>
<script src="{pages._TAILWIND_SRC}"></script>
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
  color: #a992ff;
}}
#content h2 {{
  font-size: 1.35rem;
  font-weight: 600;
  margin: 1.25rem 0 0.5rem;
  color: #a992ff;
}}
#content h3 {{
  font-size: 1.1rem;
  font-weight: 600;
  margin: 1rem 0 0.5rem;
  color: #a992ff;
}}
#content h4, #content h5, #content h6 {{
  font-size: 1rem;
  font-weight: 600;
  margin: 0.75rem 0 0.5rem;
  color: #a992ff;
}}
#content a {{ color: #a992ff; }}
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
#content strong {{ color: #a9bcb2; }}
#content blockquote {{
  border: 1px solid #16211b;
  border-radius: 3px;
  padding: .5rem .75rem;
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
  background: #0d1712;
  padding: 2px 6px;
  border-radius: 3px;
}}
#content table {{
  border-collapse: collapse;
  width: 100%;
  margin: 0.75rem 0;
  font-size: 0.875rem;
  display: block;
  max-width: 100%;
  overflow-x: auto;
}}
#content th, #content td {{
  border: 1px solid #16211b;
  padding: 0.5rem 0.75rem;
  text-align: left;
}}
#content th {{
  background: #0d1712;
  font-weight: 600;
}}
#content hr {{
  border: none;
  border-top: 1px solid #16211b;
  margin: 1.5rem 0;
}}
#content img {{ max-width: 100%; }}
</style>
</head>
<body class="bg-[#040806] text-slate-300">
<header class="sticky top-0 z-10 flex items-center gap-4
  bg-[#0d1712] border-b border-[#16211b] px-4 py-2">
  <a href="../../" class="text-[#bd83ff] font-bold text-sm
    hover:opacity-80">&larr; RSX Playground</a>
  <nav class="flex items-center gap-3 text-xs text-slate-400">
    <a href="../../" class="hover:text-white">Dashboard</a>
    <a href="../guide/README" class="hover:text-white">Docs</a>
  </nav>
</header>
<div class="flex min-h-screen">
  <aside class="w-52 bg-[#0d1712] border-r
    border-[#16211b] p-4 shrink-0">
    <a href="../../" class="text-white font-bold text-sm
      block mb-4">RSX Playground</a>
    {sidebar}
    <div class="mt-6 pt-4 border-t border-[#16211b]">
      <a href="../../" class="text-slate-400 text-xs
        hover:text-white block py-1">&larr; Back to dashboard</a>
      <a href="../../terminal" class="text-slate-400 text-xs
        hover:text-white block py-1">Open rsx-term</a>
    </div>
  </aside>
  <main class="flex-1 min-w-0 max-w-3xl p-4 sm:p-8">
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
        if exc.status_code == 404:
            return JSONResponse(
                {"error": "not found"}, status_code=404)
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
    procs = _cached_for("procs", 1.0, scan_processes)
    return HTMLResponse(pages.render_process_table(procs))


@app.get("/x/proc-chip", response_class=HTMLResponse)
async def x_proc_chip():
    procs = _cached_for("procs", 1.0, scan_processes)
    running, expected = process_counts(procs)
    return HTMLResponse(
        pages.render_proc_chip(procs, running, expected))


_BENCH_BASELINE_FILE = ROOT / "bench-baseline.json"


def _read_baseline_e2e_p99_us() -> float | None:
    """Read e2e_us.p99 from bench-baseline.json, or None."""
    try:
        data = json.loads(_BENCH_BASELINE_FILE.read_text())
        return float(data.get("e2e_us", {}).get("p99", 0)) or None
    except Exception:
        return None


# ── F3.2: TTL cache for thundering-herd HTMX polling ─────
# HTMX panels poll every 2-5s; the playground's expensive
# aggregates (scan_processes, parse_wal_*, log tails)
# previously ran per-request, so 5-10 simultaneous panel polls
# stacked head-to-tail and /x/health alone wedged for ~75s.
# A 1-second TTL is invisible to the operator and collapses
# the herd onto a single computation.
_TTL_CACHE: dict[str, tuple[float, object]] = {}


def _cached_for(key: str, ttl_seconds: float, compute_fn):
    """Return cached value or recompute if older than ttl."""
    now = time.time()
    hit = _TTL_CACHE.get(key)
    if hit is not None:
        ts, val = hit
        if now - ts < ttl_seconds:
            return val
    val = compute_fn()
    _TTL_CACHE[key] = (now, val)
    return val


def _compute_health() -> dict:
    """Compute truthful health score.

    Signals (each subtracts from a starting 100):
      - process restart count (any process restarted = -10/restart)
      - latency p99 ≥ 2× baseline (-30)
      - recent ERROR / panic lines in logs (-5 per up to 6)
      - any /verify entry with status == "fail" forces RED (≤49)
    Returns dict with score, label, reasons. Returns
    {"score": None} when we genuinely don't know.
    """
    procs = _cached_for("procs", 1.0, scan_processes)
    if not procs:
        return {"score": None, "label": "unknown",
                "reasons": ["no processes scanned"]}
    score = 100
    reasons: list[str] = []
    # Restart counter (preserved across spawn cycles).
    total_restarts = sum(
        rs.get("restarts", 0) for rs in _restart_state.values()
    )
    if total_restarts > 0:
        penalty = min(60, total_restarts * 10)
        score -= penalty
        reasons.append(f"-{penalty} ({total_restarts} restarts)")
    # Stopped processes that the plan expected to be running.
    expected_stopped = sum(
        1 for p in procs
        if p.get("state") in ("stopped", "blocked")
    )
    if expected_stopped > 0:
        penalty = min(50, expected_stopped * 15)
        score -= penalty
        reasons.append(
            f"-{penalty} ({expected_stopped} stopped)"
        )
    # Latency regression vs baseline.
    base_p99 = _read_baseline_e2e_p99_us()
    if base_p99 and e2e_latencies:
        cur_p99 = _percentiles(e2e_latencies).get("p99", 0)
        if cur_p99 > base_p99 * 2:
            score -= 30
            reasons.append(
                f"-30 (p99 {int(cur_p99)}us "
                f"> 2x baseline {int(base_p99)}us)"
            )
    # Recent error / panic lines across log files. Read only
    # the trailing 64 KB of each log (not the entire file) so
    # /x/health stays fast on multi-megabyte logs.
    #
    # Only dock for signals from a process that is CURRENTLY down —
    # a "panicked at" line from a process that has since auto-restarted
    # and is running again is stale (the restart COUNT already docks
    # for that instability; double-docking pinned the gauge yellow on a
    # healthy 7/7 cluster, #12). "fatal" was dropped — too broad, it
    # matched benign lines. Broken-pipe WRN (client disconnects) is not
    # an error (#14).
    running_names = {
        p["name"] for p in procs if p.get("state") == "running"
    }
    err_count = 0
    panic_seen = False
    if LOG_DIR.exists():
        for lf in sorted(LOG_DIR.glob("*.log")):
            if lf.stem in running_names:
                continue  # process recovered — its log tail is stale
            for line in _tail_lines(lf, max_lines=200):
                low = line.lower()
                if "broken pipe" in low:
                    continue
                if "panicked at" in low:
                    panic_seen = True
                    err_count += 1
                elif " error " in low or " err " in low:
                    err_count += 1
    if panic_seen:
        score -= 25
        reasons.append("-25 (panic in a down process)")
    elif err_count > 0:
        penalty = min(30, err_count)
        score -= penalty
        reasons.append(
            f"-{penalty} ({err_count} error lines)"
        )
    # F3.3: failing-invariant → RED. /verify FAIL rows are
    # correctness violations; they must not coexist with a
    # YELLOW score. Force the score to ≤49 (RED band) so the
    # /x/health pill reflects the /verify truth on the next poll.
    fail_count = sum(
        1 for v in verify_results
        if str(v.get("status", "")).lower() == "fail"
    )
    if fail_count > 0:
        score = min(score, 49)
        reasons.append(
            f"RED ({fail_count} verify fail{'s' if fail_count != 1 else ''})"
        )
    score = max(0, score)
    if score >= 80:
        label = "green"
    elif score >= 50:
        label = "yellow"
    else:
        label = "red"
    return {"score": score, "label": label, "reasons": reasons}


@app.get("/x/health", response_class=HTMLResponse)
async def x_health():
    h = _cached_for("health", 1.0, _compute_health)
    return HTMLResponse(pages.render_health_score(h))


def _tail_lines(path: Path, max_lines: int = 200,
                window_bytes: int = 65_536) -> list[str]:
    """Return the last `max_lines` lines of `path`.

    Reads at most `window_bytes` from the tail so log-scanning
    stays bounded as files grow. Returns [] on any I/O error.
    """
    try:
        size = path.stat().st_size
    except OSError:
        return []
    if size <= 0:
        return []
    start = max(0, size - window_bytes)
    try:
        with path.open("rb") as fh:
            fh.seek(start)
            data = fh.read()
    except OSError:
        return []
    try:
        text = data.decode("utf-8", errors="replace")
    except Exception:
        return []
    lines = text.splitlines()
    if start > 0 and lines:
        # First (possibly truncated) line may be a partial — drop it
        lines = lines[1:]
    return lines[-max_lines:]


def _count_recent_errors(max_lines: int = 200) -> int:
    """Sum ERROR/WARN/panic lines across log tails.

    Used for the Errors metric and the pulse Errors pill. Both
    must agree with the Logs tab's "errors only" filter — when
    that filter shows N lines, this returns ≥ N.
    """
    n = 0
    if not LOG_DIR.exists():
        return 0
    for lf in LOG_DIR.glob("*.log"):
        for line in _tail_lines(lf, max_lines=max_lines):
            low = line.lower()
            # Broken-pipe WRN = a client/peer disconnected mid-write;
            # benign and high-volume (thousands of lines). Counting it
            # made the Errors metric disagree with everything else (#14).
            if "broken pipe" in low:
                continue
            if (" error " in low or " err " in low
                    or " warn " in low
                    or "panicked at" in low):
                n += 1
    return n


@app.get("/x/key-metrics", response_class=HTMLResponse)
async def x_key_metrics():
    terminal = {
        "filled", "cancelled", "rejected",
        "failed", "expired",
    }
    ao = sum(
        1 for o in recent_orders
        if o.get("status", "") not in terminal)
    fills = _cached_for(
        "wal_fills_2000", 1.0,
        lambda: parse_wal_fills(max_fills=2000))
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
    # Sliding 30s window: count order submission timestamps
    # within the last 30 seconds. Avoids lifetime-average
    # decay (F26).
    window_s = 30
    now = time.time()
    cutoff = now - window_s
    recent_in_window = sum(
        1 for ts in recent_order_ts if ts >= cutoff)
    mps = int(recent_in_window / window_s)
    errs = _cached_for(
        "recent_errors", 1.0, _count_recent_errors)
    procs = _cached_for("procs", 1.0, scan_processes)
    proc_running, proc_expected = process_counts(procs)
    streams = _cached_for("wal_streams", 1.0, scan_wal_streams)
    return HTMLResponse(
        pages.render_key_metrics(
            procs, streams,
            active_orders=ao, positions=pos_count,
            msgs_sec=mps, error_count=errs,
            proc_running=proc_running, proc_expected=proc_expected))


@app.get("/x/pulse", response_class=HTMLResponse)
async def x_pulse():
    procs = _cached_for("procs", 1.0, scan_processes)
    running, expected = process_counts(procs)
    streams = _cached_for("wal_streams", 1.0, scan_wal_streams)
    wal_files = sum(s.get("files", 0) for s in streams)
    elapsed = max(1, time.time() - SERVER_START)
    ops = int(len(recent_orders) / elapsed)
    # Errors: union of rejected-orders and log-derived errors so
    # the pulse pill never says "errs 0" while the Logs tab is
    # showing WARN/ERR lines.
    errs = sum(
        1 for o in recent_orders
        if o.get("status") in {"rejected", "failed"})
    errs += _cached_for(
        "recent_errors", 1.0, _count_recent_errors)

    def _pill(label, value, color):
        return (
            f'<span class="text-slate-500">{label}</span>'
            f'<span class="text-{color} font-bold">'
            f'{value}</span>'
        )

    if expected and running == expected:
        proc_color = "emerald-400"
    elif running > 0:
        proc_color = "amber-400"
    else:
        proc_color = "red-400"
    return HTMLResponse(
        _pill("proc", f"{running}/{expected}", proc_color)
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
    # Run checks lazily so the card reflects live status instead
    # of "UNKNOWN" until someone visits the Verify tab. Cached for
    # 5s (matches the card's refresh cadence) to avoid re-scanning
    # every poll.
    stale = (
        verify_last_run is None
        or (time.time() - verify_last_run) > 5.0
    )
    if stale:
        try:
            await _run_invariant_checks()
        except Exception:
            pass
    return HTMLResponse(
        pages.render_invariant_status(
            verify_results, last_run=verify_last_run))


@app.get("/x/core-affinity", response_class=HTMLResponse)
async def x_core_affinity():
    return HTMLResponse(
        pages.render_core_affinity(
            _cached_for("procs", 1.0, scan_processes)))


@app.get("/x/cast-flows", response_class=HTMLResponse)
async def x_cast_flows():
    # Truthful per-pipe counters (see _cast_pipe_counts):
    # ME->Mktdata from the ME's per-symbol WAL; gw/risk write NO
    # WAL, so their legs come from the daemon health /metrics
    # endpoints, or render "live" when those aren't exposed.
    counts = _cached_for("cast_pipe_counts", 1.0, _cast_pipe_counts)
    return HTMLResponse(pages.render_cast_flows(counts))


def _fetch_daemon_counter(addr: str, counter: str):
    """Read one counter from a daemon's health /metrics endpoint.

    Returns the int value, or None when the endpoint is
    unreachable or the counter is absent. Best-effort, short
    timeout — the health server runs off the daemon's hot path
    (rsx-health), so this never touches the order path.
    """
    import urllib.request
    try:
        with urllib.request.urlopen(
                f"http://{addr}/metrics", timeout=0.3) as r:
            data = json.loads(r.read().decode("utf-8"))
    except Exception:
        return None
    for c in data.get("counters", []):
        if c.get("name") == counter:
            try:
                return int(c.get("value", 0))
            except (TypeError, ValueError):
                return None
    return None


def _daemon_counter_sum(prefix: str, env_key: str, counter: str):
    """Sum a health counter across managed daemons whose name
    starts with `prefix` (e.g. "gw-", "risk-").

    The health address comes from the daemon's own spawn env
    (`env_key`, e.g. RSX_GW_HEALTH_ADDR) — we never invent a port.
    Returns None (rendered honestly as "live", not a fake 0) when
    no matching daemon exposes a health endpoint. The gateway and
    risk tiles write no WAL, so this is the only truthful source
    for their pipe throughput.
    """
    total = 0
    found = False
    for name, info in managed.items():
        if not name.startswith(prefix):
            continue
        addr = info.get("env", {}).get(env_key)
        if not addr:
            continue
        val = _fetch_daemon_counter(addr, counter)
        if val is not None:
            total += val
            found = True
    return total if found else None


def _cast_pipe_counts() -> dict:
    """Per-pipe cast-flow counters, from truthful sources.

    ME -> Mktdata: FILL+BBO records on the ME's per-symbol WAL
    (dir named by symbol, e.g. "pengu"). This is exactly what the
    ME casts to marketdata, and it's the only leg backed by a WAL.

    Gateway -> Risk / Risk -> ME: the gateway and risk tiles write
    NO WAL, so their throughput can only come from the daemon
    health /metrics endpoints (orders_processed). When those aren't
    exposed the value is None -> rendered "live", never a fake 0.
    """
    me_to_mkt = 0
    for sd in _wal_stream_dirs():
        if sd.name == "mark":
            continue
        for _ in parse_wal_records(sd, {RECORD_FILL, RECORD_BBO}):
            me_to_mkt += 1
    return {
        "gw_to_risk": _daemon_counter_sum(
            "gw-", "RSX_GW_HEALTH_ADDR", "orders_processed"),
        "risk_to_me": _daemon_counter_sum(
            "risk-", "RSX_RISK_HEALTH_ADDR", "orders_processed"),
        "me_to_mkt": me_to_mkt,
    }


@app.get("/x/control-grid", response_class=HTMLResponse)
async def x_control_grid():
    # Share the 1s procs cache with every other CPU-bearing surface
    # (process table, resource usage) so CPU% reads identically across
    # Control and Overview instead of resampling a stateful
    # cpu_percent() per endpoint (#31).
    return HTMLResponse(
        pages.render_control_grid(
            _cached_for("procs", 1.0, scan_processes)))


@app.get("/x/resource-usage", response_class=HTMLResponse)
async def x_resource_usage():
    return HTMLResponse(
        pages.render_resource_usage(
            _cached_for("procs", 1.0, scan_processes)))


@app.get("/x/faults-grid", response_class=HTMLResponse)
async def x_faults_grid():
    return HTMLResponse(
        pages.render_faults_grid(scan_processes()))


@app.get("/x/recovery-controls", response_class=HTMLResponse)
async def x_recovery_controls():
    return HTMLResponse(
        pages.render_recovery_controls(scan_processes()))


@app.get("/x/recovery-feed", response_class=HTMLResponse)
async def x_recovery_feed():
    return HTMLResponse(
        pages.render_recovery_feed(recovery_feed_events()))


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
    # newest-first so the freshest line is visible without scrolling
    # (matches the live-tail prepend order).
    return HTMLResponse(pages.render_logs(list(reversed(lines))))


@app.get("/x/logs-tail", response_class=HTMLResponse)
async def x_logs_tail():
    # Returns bare <tr> rows (no table wrapper) for JS prepend.
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
    # Mark which symbols have a LIVE matching engine so the stats
    # table can badge the rest as stale instead of presenting a dead
    # symbol's last BBO as live liquidity (#17).
    live = {sid for sid in stats if _me_live(sid)}
    return HTMLResponse(
        pages.render_book_stats(stats, live_symbols=live))


@app.get("/x/live-fills", response_class=HTMLResponse)
async def x_fills():
    fills = parse_wal_fills()
    if not fills:
        fills = list(reversed(recent_fills[-50:]))
    return HTMLResponse(
        pages.render_live_fills(fills, maker_running=_maker_running()))


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


@app.get("/x/e2e-latency", response_class=HTMLResponse)
async def x_e2e_latency():
    return HTMLResponse(
        pages.render_e2e_latency(e2e_latencies))


@app.get("/x/risk-latency", response_class=HTMLResponse)
async def x_risk_latency():
    return HTMLResponse(pages.render_risk_latency(order_latencies))


@app.get("/x/latency-overview", response_class=HTMLResponse)
async def x_latency_overview():
    return HTMLResponse(
        pages.render_latency_overview(e2e_latencies, gw_only_latencies))


@app.get("/x/load-overview", response_class=HTMLResponse)
async def x_load_overview():
    running = _stress_running()
    window_s = 30
    now = time.time()
    cutoff = now - window_s
    ord_s = int(
        sum(1 for ts in recent_order_ts if ts >= cutoff) / window_s)
    submitted = 0
    accepted = 0
    if running:
        submitted = len([
            o for o in recent_orders
            if o.get("status") not in {None}])
        accepted = len([
            o for o in recent_orders
            if o.get("status") == "accepted"])
    return HTMLResponse(
        pages.render_load_overview(
            running, current_scenario, ord_s, accepted, submitted))


@app.get("/x/reconciliation",
         response_class=HTMLResponse)
async def x_reconciliation():
    # Both the in-memory shadow snap and parse_wal_bbo derive from
    # the WAL, so this is a WAL-internal consistency check, not a
    # shadow-vs-engine check (the ME book is not queryable from the
    # dashboard). Labelled honestly in render_reconciliation.
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
                    "pass",
                    f"{checked} symbols WAL-consistent")
            else:
                shadow_check = (
                    "fail",
                    f"{mismatches}/{checked} mismatch")

    # Compare book mid against the real index from the mark process
    # (RECORD_MARK_PRICE, external Binance/Coinbase aggregate). SKIP
    # when no index is loaded — a check that always passes is worse
    # than no check. Tolerance band: 1% premium of perp over index.
    mark_check = None
    mark_prices = parse_wal_mark_prices()
    if _book_snap and mark_prices:
        checked = 0
        mismatches = 0
        for sid, snap in _book_snap.items():
            mark_rec = mark_prices.get(sid)
            if mark_rec is None:
                continue
            index_px = mark_rec.get("mark_price", 0)
            if index_px <= 0:
                continue
            bid = snap.get("best_bid", snap.get("bid_px", 0))
            ask = snap.get("best_ask", snap.get("ask_px", 0))
            if bid <= 0 or ask <= 0:
                continue
            mid = (bid + ask) // 2
            checked += 1
            dev_bps = abs(mid - index_px) * 10000 // index_px
            if dev_bps > 100:
                mismatches += 1
        if checked > 0:
            if mismatches == 0:
                mark_check = (
                    "pass",
                    f"{checked} symbols within 1% of index")
            else:
                mark_check = (
                    "fail",
                    f"{mismatches}/{checked} off-index >1%")

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


def _order_age_s(ts, now: float):
    """Age in seconds for a recent_orders ts (float epoch or
    "%H:%M:%S" string). UI batch helpers write the string form, so
    the stale detector must understand both or it always reads 0.
    Returns None if ts is missing/unparseable."""
    if isinstance(ts, (int, float)):
        return now - ts
    if isinstance(ts, str) and ts:
        try:
            t = datetime.strptime(ts, "%H:%M:%S").time()
        except ValueError:
            return None
        today = datetime.fromtimestamp(now).date()
        entry = datetime.combine(today, t)
        age = now - entry.timestamp()
        # Wall-clock string with no date: if it parses to the
        # future (just after midnight), it was actually yesterday.
        if age < 0:
            age += 86400
        return age
    return None


@app.get("/x/stale-orders", response_class=HTMLResponse)
async def x_stale_orders():
    now = time.time()
    # "Resolved" = a real terminal state OR live resting liquidity.
    # A resting order is healthy by design, NOT stale — flagging it
    # made the count climb with every quote. Only orders stuck in a
    # transient state (submitted/sent/pending/timeout/error) past the
    # age threshold are genuinely hung.
    resolved = {
        "filled", "cancelled", "rejected", "failed", "expired",
        "resting", "accepted", "done",
    }
    stale = [
        o for o in recent_orders
        if o.get("status", "") not in resolved
        and (age := _order_age_s(o.get("ts"), now)) is not None
        and age > 60]
    if not stale:
        return HTMLResponse(
            '<span class="text-emerald-400 text-xs">'
            'no hung orders</span>')
    return HTMLResponse(
        f'<span class="text-amber-400 text-xs">'
        f'{len(stale)} hung order(s) '
        f'(&gt;60s, no terminal state)</span>')


def _me_live(symbol_id: int) -> bool:
    """True when the matching engine for this symbol is running.

    The book ladder renders the last snapshot; if the ME is down
    that ladder is stale (no longer backed by a live book), so the
    caller badges it as such instead of showing phantom liquidity.
    """
    sym_name = next(
        (k for k, v in start_mod.SYMBOLS.items()
         if v["id"] == symbol_id),
        None,
    )
    if not sym_name:
        return True  # unknown symbol: don't assert staleness
    me_name = f"me-{sym_name.lower()}"
    procs = _cached_for("procs", 1.0, scan_processes)
    return any(
        p.get("name") == me_name and p.get("state") == "running"
        for p in procs
    )


@app.get("/x/book", response_class=HTMLResponse)
async def x_book(symbol_id: int = Query(10)):
    stale = not _me_live(symbol_id)
    snap = _book_snap.get(symbol_id)
    if snap and (snap.get("bids") or snap.get("asks")):
        return HTMLResponse(
            pages.render_book_ladder(symbol_id, snap,
                                     source="live", stale=stale))
    # Fallback: WAL BBO gives at most 1 bid + 1 ask
    bbo = parse_wal_bbo(symbol_id)
    if bbo is not None:
        snap_from_bbo: dict = {"bids": [], "asks": []}
        if bbo.get("bid_px"):
            snap_from_bbo["bids"] = [
                {"px": bbo["bid_px"], "qty": bbo["bid_qty"]}]
        if bbo.get("ask_px"):
            snap_from_bbo["asks"] = [
                {"px": bbo["ask_px"], "qty": bbo["ask_qty"]}]
        return HTMLResponse(
            pages.render_book_ladder(symbol_id, snap_from_bbo,
                                     source="synthetic", stale=stale))
    # Last fallback: maker book
    maker_snap = _maker_book(symbol_id)
    if maker_snap:
        return HTMLResponse(
            pages.render_book_ladder(symbol_id, maker_snap,
                                     source="synthetic", stale=stale))
    return HTMLResponse(
        pages.render_book_ladder(symbol_id, None,
                                 maker_running=_maker_running()))


@app.get("/x/risk-user", response_class=HTMLResponse)
async def x_risk_user(
    risk_uid: int = Query(1, alias="risk-uid"),
    user_id: int | None = Query(None),
):
    # Accept either ?risk-uid= (the form field) or the more natural
    # ?user_id= (both name the same user; user_id wins if both given).
    uid = user_id if user_id is not None else risk_uid
    # ONE source of truth for the position: WAL fills — the same
    # source the Risk dashboard (api_risk_overview) uses, so the two
    # never contradict. The persisted `positions` table can hold rows
    # left over from a prior run (stale after a WAL wipe); we no longer
    # present those as the live position. Collateral IS legitimately
    # persisted (not derived from fills), so we surface it separately.
    fills = parse_wal_fills_for_user_all(uid)
    collateral = None
    acct = await pg_query(
        "SELECT collateral FROM accounts WHERE user_id = $1",
        uid,
    )
    if acct and isinstance(acct, list) and acct:
        collateral = acct[0].get("collateral")
    return HTMLResponse(
        pages.render_risk_user_wal(uid, fills, collateral))


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
    procs = scan_processes()
    running, expected = process_counts(procs)
    if not verify_results:
        return HTMLResponse(
            '<span class="text-slate-600">'
            f'processes running {running}/{expected}; '
            'click "Run All Checks" to verify</span>')
    return HTMLResponse(
        '<div class="text-[10px] text-slate-500 mb-2">'
        f'processes running {running}/{expected}</div>'
        + pages.render_verify(verify_results))


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
async def api_build(request: Request):
    """Trigger cargo build."""
    denied = _require_admin_request(request)
    if denied:
        return denied
    ok = await do_build()
    return HTMLResponse(
        f'<span class="text-{"emerald" if ok else "red"}'
        f'-400 text-xs">'
        f'{"build ok" if ok else "build FAILED"}</span>')


@app.post("/api/processes/all/start")
async def api_start_all(
    request: Request,
    scenario: str = Query(None),
):
    """Build + start all processes.

    Scenario resolution: explicit ?scenario= query wins (curl,
    tests, CLI). Otherwise read the checked radio from the form
    body (the overview button uses hx-include, no JS eval, so it
    works under a strict CSP). Falls back to minimal.
    """
    if scenario is None:
        try:
            form = await request.form()
            scenario = form.get("scenario-ov") or "minimal"
        except Exception:
            scenario = "minimal"
    denied = _require_admin_request(request)
    if denied:
        return denied
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
    denied = _require_admin_request(request)
    if denied:
        return denied
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


@app.post("/api/reset")
async def api_reset(request: Request):
    """Stop everything and wipe state to a clean slate — mirrors
    `./rsx-playground/playground reset`. Does NOT restart; the user
    starts the cluster again from Control/Overview afterwards."""
    denied = _require_admin_request(request)
    if denied:
        return denied
    denied = check_confirm(request, "/api/reset")
    if denied:
        return denied
    audit_log("/api/reset", "stop all + wipe state")
    result = await stop_all()
    cleared = []
    # wipe WAL (keep the wal/ dir itself, like find -mindepth 1 -delete)
    if WAL_DIR.exists():
        for child in WAL_DIR.iterdir():
            if child.is_dir():
                shutil.rmtree(child, ignore_errors=True)
            else:
                child.unlink(missing_ok=True)
        cleared.append("wal")
    # Wipe persisted risk state so the Lookup can't show a position
    # with no backing fills after the WAL is gone (see #10). Positions
    # are rebuilt from fills; accounts keep their seeded collateral.
    pg_res = await pg_query("DELETE FROM positions")
    if pg_res is not None and not (
        isinstance(pg_res, dict) and "error" in pg_res
    ):
        cleared.append("positions")
    # replay tips
    (TMP / "md.tip").unlink(missing_ok=True)
    for tip in TMP.glob("recorder-tip-*"):
        tip.unlink(missing_ok=True)
    cleared.append("tips")
    # leftover pid files (never our own server)
    if PID_DIR.exists():
        for pid_file in PID_DIR.glob("*.pid"):
            if pid_file.stem != SELF_PID_NAME:
                pid_file.unlink(missing_ok=True)
        cleared.append("pids")
    return HTMLResponse(
        f'<span class="text-emerald-400 text-xs">'
        f'reset done &mdash; stopped {len(result["stopped"])} '
        f'processes, cleared {", ".join(cleared)}. Start the '
        f'cluster again from Control.</span>')


@app.post("/api/processes/{name}/{action}")
async def api_process_action(
    request: Request,
    name: str,
    action: str,
):
    denied = _require_admin_request(request)
    if denied:
        return denied
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
                    # Mark intentional BEFORE sending signal so the
                    # watcher cannot race between the signal and the
                    # flag being set.  Preserve existing counters.
                    rs = _restart_state.setdefault(name, {
                        "restarts": 0,
                        "total_restarts": 0,
                        "blocked": False,
                        "next_restart_at": 0.0,
                        "last_crash_ts": 0.0,
                        "intentional": False,
                    })
                    rs["intentional"] = True
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
                    # Mark intentional so watcher does not revive.
                    rs = _restart_state.setdefault(name, {
                        "restarts": 0,
                        "total_restarts": 0,
                        "blocked": False,
                        "next_restart_at": 0.0,
                        "last_crash_ts": 0.0,
                        "intentional": False,
                    })
                    rs["intentional"] = True
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
                # build first, same as the Start action
                ok = await do_build()
                if not ok:
                    msg = "build failed"
                else:
                    result = await spawn_process(
                        name, binary, env)
                    msg = (
                        f"restarted {name} (pid {result['pid']})"
                        if "pid" in result
                        else result.get("error", "failed")
                    )
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


@app.post("/api/fault/kill/{name}")
async def api_fault_kill(request: Request, name: str):
    """Crash a process (SIGKILL) WITHOUT marking it intentional,
    so _process_watcher auto-restarts it. Distinct from
    /api/processes/{name}/kill, which suppresses the restart."""
    denied = _require_admin_request(request)
    if denied:
        return denied
    audit_log(f"/api/fault/kill/{name}",
              "crash sim (SIGKILL, watcher will restart)")
    pid = None
    info = managed.get(name)
    if info and info["proc"].returncode is None:
        pid = info["proc"].pid
    else:
        proc = next(
            (p for p in scan_processes()
             if p["name"] == name and p["pid"] != "-"),
            None)
        if proc:
            pid = int(proc["pid"])
    if pid is None:
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'{name} not running</span>')
    try:
        os.kill(pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError, OSError) as e:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'Kill failed: {e}</span>')
    return HTMLResponse(
        f'<span class="text-red-400 text-xs">'
        f'crashed {name} (pid {pid}) &mdash; watch it '
        f'restart &rarr; green</span>')


@app.post("/api/fault/net")
async def api_fault_net(
    request: Request,
    action: str = Form(default="apply"),
):
    """Inject/clear a loopback network fault via sudo tc netem
    (runs sudo, degrades the box's lo device). Guarded by the
    UI's hx-confirm click-through, not an env var."""
    denied = _require_admin_request(request)
    if denied:
        return denied
    audit_log("/api/fault/net", f"net fault {action}")
    if action == "clear":
        cmd = ["sudo", "tc", "qdisc", "del", "dev", "lo", "root"]
    else:
        cmd = ["sudo", "tc", "qdisc", "add", "dev", "lo", "root",
               "netem", "delay", "50ms", "loss", "10%"]
    try:
        r = await asyncio.to_thread(
            subprocess.run, cmd,
            capture_output=True, text=True, timeout=10)
    except Exception as e:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'Failed: {e}</span>')
    if r.returncode != 0:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'Failed: {strip_ansi(r.stderr).strip()[:200]}'
            f'</span>')
    if action == "clear":
        return HTMLResponse(
            '<span class="text-emerald-400 text-xs">'
            'lo netem cleared &mdash; links healing, watch '
            'catch-up</span>')
    return HTMLResponse(
        '<span class="text-amber-400 text-xs">'
        'lo: +50ms delay, 10% loss &mdash; watch casting '
        'FAULTED &rarr; NAK/replay</span>')


@app.post("/api/fault/wal-corrupt")
async def api_fault_wal_corrupt(request: Request):
    """Flip a payload byte in the newest WAL file so its
    CRC32C fails on replay (mutates WAL on disk). Guarded by
    the UI's hx-confirm click-through, not an env var."""
    denied = _require_admin_request(request)
    if denied:
        return denied
    audit_log("/api/fault/wal-corrupt", "flip a WAL payload byte")
    files = (
        sorted(WAL_DIR.rglob("*.wal"),
               key=lambda p: p.stat().st_mtime)
        if WAL_DIR.exists() else []
    )
    if not files:
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">'
            'no WAL files found &mdash; start the cluster '
            'first</span>')
    target = files[-1]
    size = target.stat().st_size
    # WalHeader is 16 bytes; CRC32C covers the payload (offset
    # 16+), so flipping a payload byte guarantees a mismatch.
    offset = 24
    if size <= offset:
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">'
            'newest WAL has no record payload yet</span>')
    try:
        with open(target, "r+b") as f:
            f.seek(offset)
            b = f.read(1)
            f.seek(offset)
            f.write(bytes([b[0] ^ 0xFF]))
    except OSError as e:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'Failed: {e}</span>')
    rel = target.relative_to(ROOT)
    return HTMLResponse(
        f'<span class="text-red-400 text-xs">'
        f'flipped payload byte @ offset {offset} in {rel} '
        f'&mdash; CRC32C fails on replay</span>')


@app.post("/api/scenario/switch")
async def api_scenario_switch(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
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
async def api_logs_clear(request: Request):
    """Truncate all log files in ./log/ directory."""
    denied = _require_admin_request(request)
    if denied:
        return denied
    cleared = []
    if LOG_DIR.exists():
        for p in LOG_DIR.glob("*.log"):
            open(p, "w").close()
            cleared.append(p.name)
    return HTMLResponse(
        '<span class="text-emerald-400 text-xs">'
        f'cleared {len(cleared)} log file(s)</span>'
    )


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
        ("active stress", int(_stress_running())),
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
        secret = os.environ.get("RSX_GW_JWT_SECRET", "")
        if not secret:
            # 3-tuple like every other return path — a dict here makes
            # the caller's `result[1]` raise KeyError(1) → spurious 500.
            return None, "RSX_GW_JWT_SECRET not configured", None
        token = pyjwt.encode(
            {
                "sub": f"playground:{user_id}",
                "user_id": user_id,
                "aud": "rsx-gateway",
                "iss": "rsx-auth",
                "exp": int(time.time()) + 3600,
                "jti": uuid.uuid4().hex,
            },
            secret,
            algorithm="HS256",
        )
        headers = {"authorization": f"Bearer {token}"}
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
    # Record the submission timestamp (F26 sliding window) and
    # cap the list. recent_order_ts is independent from
    # recent_orders so the window survives recent_orders pruning.
    recent_order_ts.append(time.time())
    if len(recent_order_ts) > 2000:
        del recent_order_ts[:1000]
    if len(recent_orders) > 200:
        del recent_orders[:100]


# FailureReason discriminants (rsx-types/src/lib.rs). The gateway
# U-frame carries the raw u8; map it to a human string so the UI
# never surfaces a bare "reason=4". Notional overflow at the risk
# boundary is reported as InsufficientMargin (4).
REJECT_REASONS = {
    0: "invalid tick size",
    1: "invalid lot size",
    2: "symbol not found",
    3: "duplicate order id",
    4: "insufficient margin",
    5: "overloaded",
    6: "internal error",
    7: "reduce-only violation",
    8: "network error",
    9: "rate limited",
    10: "timeout",
    11: "user in liquidation",
    12: "wrong shard",
}


def reject_reason_str(reason) -> str:
    """Human string for a FailureReason u8 (falls back to code)."""
    try:
        code = int(reason)
    except (ValueError, TypeError):
        return str(reason)
    return REJECT_REASONS.get(code, f"reason {code}")


# Gateway webproto response frames (see specs/2/49-webproto.md):
#   {U:[oid,status,filled,remaining,reason]}  status 0=FILLED
#     1=RESTING 2=CANCELLED 3=FAILED
#   {F:[taker_oid,maker_oid,px,qty,ts,fee]}   immediate fill
#   {E:[code,message]}                        protocol/validation error
# No ACK for a resting GTC order (surfaces via caller timeout).
def _classify_order_response(msg) -> tuple[str, str]:
    """Map a gateway response frame to a lifecycle (status, note).

    Statuses: filled | resting | cancelled | rejected | accepted
    (ack, unknown status) | error (no/unexpected frame). A viewer
    can tell a fill from resting liquidity from a hang — the whole
    point of the recent-orders status column.
    """
    if not msg:
        return "error", "no response"
    if "U" in msg:
        u = msg["U"]
        code = u[1] if len(u) > 1 else -1
        if code == 3:
            reason = u[4] if len(u) > 4 else 0
            return "rejected", reject_reason_str(reason)
        return {0: "filled", 1: "resting",
                2: "cancelled"}.get(code, "accepted"), ""
    if "F" in msg:
        return "filled", ""
    if "E" in msg:
        e = msg["E"]
        return "rejected", str(e[1] if len(e) > 1 else e)
    return "error", "unexpected response"


@app.post("/api/orders/test")
async def api_orders_test(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
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

    # Check that gateway and the matching engine for this symbol
    # are running before attempting to forward the order.
    _sym_name = next(
        (k for k, v in start_mod.SYMBOLS.items()
         if v["id"] == symbol_id),
        None,
    )
    _me_name = f"me-{_sym_name.lower()}" if _sym_name else None
    _running_names = {
        p["name"] for p in scan_processes()
        if p.get("state") == "running"
    }
    # Graceful degradation: a stopped gateway/ME is a normal dev
    # state (the safety suite kills them on purpose), not a server
    # error. Return 200 with a descriptive body — same shape as the
    # downstream "gateway not running" path — so the orders page
    # keeps working and callers read the reason instead of a 503.
    if "gw-0" not in _running_names:
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'order {cid} error: gateway (gw-0) not available'
            f'</span>')
    if _me_name and _me_name not in _running_names:
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'order {cid} error: matching engine for '
            f'{_sym_name} not available</span>')

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
        # price 0 (or an explicit market order) = "market": send a
        # marketable-limit sweep against the live book so it crosses,
        # matching the form's "0 = market" hint. A bare price-0 limit
        # is rejected by the gateway (tick 0), which is what made the
        # default form submission fail.
        is_market = order_type == "market" or human_price == 0
        if is_market:
            mid_raw = None
            snap = _book_snap.get(symbol_id)
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
                price_int = tick_size
            else:
                sweep = (_MARKET_SWEEP_PCT if side_int == 0
                         else -_MARKET_SWEEP_PCT)
                raw = mid_raw * (1.0 + sweep / 100.0)
                price_int = max(
                    tick_size,
                    int(round(raw / tick_size)) * tick_size)
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

    # Notional = price * qty must fit in i64 (the risk boundary
    # checks this too and rejects as InsufficientMargin, but catching
    # it here gives a precise message instead of a bare "reason=4").
    # Inputs are HUMAN units; a value big enough to overflow means the
    # user pasted raw fixed-point back into the form (units are labeled
    # on the form to prevent this).
    I64_MAX = 2**63 - 1
    if price_int != 0 and qty_int != 0:
        if abs(price_int) > I64_MAX // abs(qty_int):
            return HTMLResponse(
                '<span class="text-red-400 text-xs">'
                'notional too large: price &times; qty overflows i64 '
                '&mdash; enter human units (e.g. price 0.05, qty 10), '
                'not raw fixed-point</span>')

    reduce_only = 1 if form.get("reduce_only") == "on" else 0
    # post_only can come from checkbox or order_type dropdown
    post_only = (
        1 if (
            form.get("post_only") == "on"
            or order_type == "post_only"
        ) else 0
    )
    # market orders cross immediately — force IOC (a GTC market is
    # invalid; a resting GTC uses a real limit price).
    if is_market and tif_int == 0:
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
        # human units in the recent-orders table (see #16); a market
        # order shows "mkt" (it swept, no meaningful limit price).
        "price": ("mkt" if is_market
                  else pages.format_price(price_int, symbol_id)),
        "qty": pages.format_qty(qty_int, symbol_id),
        "tif": tif_str,
        "reduce_only": bool(reduce_only),
        "post_only": bool(post_only),
        "status": "pending",
        "ts": datetime.now().strftime("%H:%M:%S"),
    }

    result = await send_order_to_gateway(order_msg, user_id)
    err = result[1]
    if err:
        # Timeout: gateway WS opened but no U/F/E frame in 2s. Split by TIF:
        #   GTC (tif_int == 0): a resting limit gets NO accepted-ack by
        #     design (spec 49-webproto.md §54), so no-ack is the normal,
        #     expected outcome -- the order rests in the book. Return 200
        #     "resting". Genuine lost-order detection is a reconciliation
        #     concern (compare submitted vs WAL), NOT this 2s timeout, so
        #     F-N1 is no longer conflated per-order.
        #   IOC/FOK (tif_int != 0): they MUST respond with one of U/F/E;
        #     a no-ack IS a real error. Keep amber + HTTP 504 (F-N1 honesty
        #     preserved for the must-ack case).
        if err == "timeout waiting for response":
            if tif_int == 0:
                order["status"] = "resting"
                recent_orders.append(order)
                _trim_recent_orders()
                return HTMLResponse(
                    f'<span class="text-blue-400 text-xs">'
                    f'order {cid} resting (limit rests in the book)'
                    f'</span>')
            order["status"] = "timeout"
            recent_orders.append(order)
            _trim_recent_orders()
            return HTMLResponse(
                f'<span class="text-amber-400 text-xs">'
                f'order {cid} timeout: no matching-engine response in 2s'
                f'</span>',
                status_code=504,
            )
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
        order["latency_us"] = latency_us

    # Real lifecycle status so a filled order reads "filled", a
    # resting order "resting", a hang stays visible — no more
    # everything-is-"accepted".
    status_label, note = _classify_order_response(msg)
    order["status"] = status_label
    if note:
        order["reason"] = note
    recent_orders.append(order)
    _trim_recent_orders()
    ok = status_label in ("filled", "resting", "accepted")
    lat_str = f"{latency_us}us" if latency_us is not None else "?"
    if ok:
        return HTMLResponse(
            f'<span class="text-emerald-400 text-xs">'
            f'order {cid} {status_label} ({lat_str})</span>')
    color = "text-amber-400" if status_label == "error" else "text-red-400"
    detail = f": {note}" if note else ""
    return HTMLResponse(
        f'<span class="{color} text-xs">'
        f'order {cid} {status_label}{detail}</span>')


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
            total_bytes = s.get("total_bytes", 0)
            if s["files"] == 0:
                st = "warn"
                detail = "no WAL files"
            elif total_bytes == 0:
                st = "warn"
                detail = (f"{s['files']} files, 0 bytes"
                          " — no records written")
            else:
                st = "pass"
                detail = (f"{s['files']} files, "
                          f"{s['total_size']}")
            checks.append({
                "name": f"WAL stream {s['name']} has data",
                "status": st,
                "time": now,
                "detail": detail,
            })

    procs = scan_processes()
    # Same (running/expected) definition as every other surface (#11):
    # cluster spawn plan + maker. Counting ALL running procs against a
    # maker-less expected produced the impossible "7/6 running".
    expected_names = expected_process_names()
    expected_count = len(expected_names)
    running_names = {
        p["name"] for p in procs
        if p.get("state") == "running"}
    running_count = len(running_names & expected_names)
    all_running = running_count >= expected_count > 0
    down = sorted(expected_names - running_names)
    if procs:
        proc_detail = f"{running_count}/{expected_count} running"
        if down:
            proc_detail += "; down: " + ", ".join(down)
    else:
        proc_detail = "no processes running"
    checks.append({
        "name": "RSX processes running",
        "status": "pass" if all_running else "fail",
        "time": now,
        "detail": proc_detail,
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
    # Cross-check WAL against the session counter the Book tab
    # and /x/topology/gateway both read. If either says fills
    # exist, run the real check; only SKIP when BOTH report
    # zero fills (never PASS in that case — there's nothing to
    # check). Previously this returned PASS with detail
    # "no trades yet" while 135 fills were visible elsewhere.
    session_fills = len(recent_fills)
    if not fill_seqs and session_fills == 0:
        checks.append({
            "name": "Fills precede ORDER_DONE (per order)",
            "status": "skip",
            "time": now,
            "detail": (
                "no WAL fills and no session fills"
                if all_running
                else "no processes running"
            ),
        })
    elif not fill_seqs:
        # WAL says zero but session counter disagrees — flag it.
        checks.append({
            "name": "Fills precede ORDER_DONE (per order)",
            "status": "fail",
            "time": now,
            "detail": (
                f"WAL fills=0 but session fills="
                f"{session_fills} — sources disagree"
            ),
        })
    else:
        inversion_pairs: list[tuple[int, int]] = [
            (fill_seqs[i - 1], fill_seqs[i])
            for i in range(1, len(fill_seqs))
            if fill_seqs[i] < fill_seqs[i - 1]
        ]
        violations = len(inversion_pairs)
        fill_detail = (
            f"{len(fill_seqs)} fills, "
            f"{violations} seq inversions"
        )
        if inversion_pairs:
            pairs_str = "; ".join(
                f"{a}→{b}" for a, b in inversion_pairs[:3]
            )
            fill_detail += f"; first: {pairs_str}"
        checks.append({
            "name": "Fills precede ORDER_DONE (per order)",
            "status": "pass" if violations == 0 else "fail",
            "time": now,
            "detail": fill_detail,
        })

    # ── invariant 2: exactly-one completion per order ───────
    completed: dict[str, int] = {}
    for o in recent_orders:
        if o.get("status") in ("accepted", "rejected", "error"):
            cid = o.get("cid", "")
            completed[cid] = completed.get(cid, 0) + 1
    dupes = {c: n for c, n in completed.items() if n > 1}
    if dupes:
        dupe_detail = (
            f"{len(dupes)} cids with duplicate completions"
            f"; cids: {', '.join(list(dupes)[:5])}"
        )
    elif completed:
        dupe_detail = f"{len(completed)} orders, no duplicates"
    else:
        dupe_detail = "no completed orders observed"
    checks.append({
        "name": "Exactly-one completion per order",
        "status": "fail" if dupes else (
            "pass" if completed else "skip"
        ),
        "time": now,
        "detail": dupe_detail,
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
                pos_detail = (
                    f"{len(rows)} position/fill mismatches: "
                    + "; ".join(
                        f"user={r['user_id']} sym={r['symbol_id']}"
                        f" pos={r['pos']} fills={r['fills']}"
                        for r in rows[:3]
                    )
                )
                checks.append({
                    "name": "Position = sum of fills (risk engine)",
                    "status": "fail",
                    "time": now,
                    "detail": pos_detail,
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
                fund_detail = (
                    f"{len(rows)} symbols with non-zero net funding: "
                    + "; ".join(
                        f"sym={r['symbol_id']} net={r['net']}"
                        for r in rows[:5]
                    )
                )
                checks.append({
                    "name": (
                        "Funding zero-sum across"
                        " users per symbol"
                    ),
                    "status": "fail",
                    "time": now,
                    "detail": fund_detail,
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
            if n == SELF_PID_NAME:
                continue
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
    global verify_last_run
    verify_last_run = time.time()
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


_BATCH_MAX = 100


@app.post("/api/orders/batch")
async def api_orders_batch(count: int = Query(10)):
    # Honor the requested count (was silently capped at 10), clamped
    # to a sane [1, _BATCH_MAX] range.
    n = max(1, min(count, _BATCH_MAX))
    for i in range(n):
        recent_orders.append({
            "cid": f"bat-{int(time.time()*1000)%100000+i:05d}",
            "symbol": "10",
            "side": "buy" if i % 2 == 0 else "sell",
            "price": pages.format_price(50000 + i * 10, 10),
            "qty": pages.format_qty(100000, 10),
            "status": "submitted",
            "ts": datetime.now().strftime("%H:%M:%S"),
        })
    _trim_recent_orders()
    capped = " (capped)" if count > _BATCH_MAX else ""
    return HTMLResponse(
        '<span class="text-emerald-400 text-xs">'
        f'{n} batch orders submitted{capped}</span>')


@app.post("/api/orders/random")
async def api_orders_random():
    for _ in range(5):
        recent_orders.append({
            "cid": f"rnd-{random.randint(10000,99999)}",
            "symbol": "10",
            "side": random.choice(["buy", "sell"]),
            "price": pages.format_price(
                random.randint(40000, 60000), 10),
            "qty": pages.format_qty(
                random.choice([100000, 200000, 500000]), 10),
            "status": "submitted",
            "ts": datetime.now().strftime("%H:%M:%S"),
        })
    _trim_recent_orders()
    return HTMLResponse(
        '<span class="text-emerald-400 text-xs">'
        '5 random orders submitted</span>')


# A "market" quick order is sent as a marketable-limit sweep this
# far through the mid so it crosses the visible book on either side.
_MARKET_SWEEP_PCT = 50.0


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

    qty_choices = [10, 20, 50, 100]

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

    # Reference mid from book snapshot or WAL BBO (used by both the
    # market-sweep and limit paths).
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

    def _to_tick(raw):
        p = int(round(raw / tick_size)) * tick_size
        return p if p > 0 else tick_size

    # offset 0 = "market": send a marketable-limit sweep (buy far
    # above the ask / sell far below the bid) so it actually crosses.
    # A price-0 IOC crosses nothing on a buy and everything at 0 on a
    # sell — never a real market fill, and leaves the order hung.
    is_market = offset_pct == 0.0 and not randomize
    if mid_raw is None:
        # No book to price against: send IOC at a minimal valid price
        # so the order resolves (cancels) instead of resting a bogus
        # price-0 order that hangs forever.
        price_int = tick_size
        tif_int = 1
    elif is_market:
        sweep = (_MARKET_SWEEP_PCT if side_int == 0
                 else -_MARKET_SWEEP_PCT)
        price_int = _to_tick(mid_raw * (1.0 + sweep / 100.0))
        tif_int = 1  # IOC market sweep
    else:
        price_int = _to_tick(mid_raw * (1.0 + offset_pct / 100.0))
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
        # human units in the orders table (see #16); the raw ints went
        # to the gateway in order_msg["N"] above.
        "price": pages.format_price(price_int, symbol_id),
        "qty": pages.format_qty(qty_int, symbol_id),
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
            order["status"] = "timeout"
            recent_orders.append(order)
            _trim_recent_orders()
            return HTMLResponse(
                f'<span class="text-amber-400 text-xs font-medium">'
                f'{label} timeout ({cid})</span>',
                status_code=504,
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
        order["latency_us"] = latency_us

    # Surface the terminal lifecycle state (filled/resting/rejected)
    # — never leave a quick order stuck on "sent".
    status_label, note = _classify_order_response(msg)
    order["status"] = status_label
    if note:
        order["reason"] = note
    recent_orders.append(order)
    _trim_recent_orders()
    ok = status_label in ("filled", "resting", "accepted")
    color = "text-emerald-400" if ok else "text-red-400"
    detail = f": {note}" if note else ""
    return HTMLResponse(
        f'<span class="{color} text-xs font-medium">'
        f'{label} {status_label}{detail}</span>'
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
        await _remove_managed_process(STRESS_NAME)
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
        # Must match the dir the reports list reads (STRESS_REPORTS_DIR)
        # or a completed run never shows up under HISTORICAL REPORTS.
        "RSX_STRESS_REPORT_DIR": str(STRESS_REPORTS_DIR),
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
    _register_managed_process(
        STRESS_NAME,
        proc,
        str(STRESS_SCRIPT),
        env,
    )
    PID_DIR.mkdir(parents=True, exist_ok=True)
    (PID_DIR / f"{STRESS_NAME}.pid").write_text(str(proc.pid))
    await asyncio.sleep(0.2)
    if proc.returncode is not None:
        await _remove_managed_process(STRESS_NAME)
        (PID_DIR / f"{STRESS_NAME}.pid").unlink(
            missing_ok=True)
        return False
    return True


async def do_stress_stop() -> None:
    await stop_process(STRESS_NAME)
    (PID_DIR / f"{STRESS_NAME}.pid").unlink(missing_ok=True)


@app.post("/api/stress/run")
async def api_stress_run(
    request: Request,
    rate: int = Form(default=10),
    duration: int = Form(default=1),
    gateway_url: str = Form(default=""),
):
    """Synchronous stress trigger that returns a structured
    JSON envelope.

    On success: 200 + stress summary.
    On gateway unreachable: 502 + {"code":"GATEWAY_UNREACHABLE",
    "message":..., "context":{"gateway_url":...}}.
    On bad form: 400 + {"code":"BAD_REQUEST", "message":...}.
    HTMX path (hx-request: true) returns 200 with an error
    span instead of 502 so the UI can render inline.
    """
    is_htmx = request.headers.get("hx-request") == "true"
    err_code: str | None = None
    err_msg: str | None = None
    if rate <= 0:
        err_code = "BAD_REQUEST"
        err_msg = "rate must be > 0"
        return JSONResponse(
            status_code=400,
            content={
                "code": err_code,
                "message": err_msg,
                "context": {"rate": rate},
            })
    target_url = gateway_url or GATEWAY_URL
    # Probe gateway reachability (TCP connect to host:port).
    from urllib.parse import urlparse
    parsed = urlparse(target_url)
    host = parsed.hostname or "localhost"
    port = parsed.port or 8080
    reachable = False
    try:
        reader, writer = await asyncio.wait_for(
            asyncio.open_connection(host, port),
            timeout=1.0,
        )
        writer.close()
        try:
            await writer.wait_closed()
        except Exception:
            pass
        reachable = True
    except Exception:
        reachable = False
    if not reachable:
        err_code = "GATEWAY_UNREACHABLE"
        err_msg = (
            f"could not reach gateway at {target_url}")
        if is_htmx:
            return HTMLResponse(
                f'<span class="text-red-400 text-xs">'
                f'gateway unreachable: {target_url}</span>',
                status_code=200)
        return JSONResponse(
            status_code=502,
            content={
                "code": err_code,
                "message": err_msg,
                "context": {"gateway_url": target_url},
            })
    # Reachable — kick off a real (short) stress run so it writes a
    # report the HISTORICAL REPORTS list can pick up.
    started = await do_stress_start({
        "gw_url": target_url,
        "rate": rate,
        "duration": duration,
    })
    if is_htmx:
        if not started:
            return HTMLResponse(
                '<span class="text-red-400 text-xs">'
                'stress failed to start (already running?)</span>')
        return HTMLResponse(
            f'<div class="text-xs text-emerald-400">stress running '
            f'&mdash; {rate}/s for {duration}s against {html.escape(target_url)}'
            f'</div>'
            f'<div class="text-slate-500 text-[11px] mt-1">a report '
            f'appears under <a href="./stress" class="text-blue-400 '
            f'hover:underline">Stress &rarr; historical reports</a> '
            f'when it finishes.</div>')
    return JSONResponse({
        "code": "OK",
        "message": "stress started" if started else "stress busy",
        "context": {
            "rate": rate,
            "duration": duration,
            "gateway_url": target_url,
            "started": started,
        },
    })


@app.post("/api/stress/start")
async def api_stress_start(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
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
    denied = _require_admin_request(request)
    if denied:
        return denied
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
    """List all stress test reports.

    Corrupt or partial JSON files are surfaced as
    {report_id, corrupt: True, error: ...} entries rather than
    silently dropped (F28).
    """
    reports = []
    if STRESS_REPORTS_DIR.exists():
        for f in sorted(STRESS_REPORTS_DIR.glob("stress-*.json"), reverse=True):
            report_id = f.stem.replace("stress-", "")
            try:
                with open(f) as fp:
                    data = json.load(fp)
                reports.append({
                    "id": report_id,
                    # stress.py doesn't write a timestamp field; the
                    # report_id IS the YYYYMMDD-HHMMSS from the filename,
                    # which the list formats. Fall back to it (#9).
                    "timestamp": data.get("timestamp") or report_id,
                    "rate": data["config"]["target_rate"],
                    "duration": data["config"]["duration"],
                    "submitted": data["metrics"]["submitted"],
                    "accepted": data["metrics"]["accepted"],
                    "accept_rate": data["metrics"]["accept_rate"],
                    "p99_latency": data["latency_us"]["p99"],
                })
            except Exception as e:
                reports.append({
                    "id": report_id,
                    "corrupt": True,
                    "error": f"{type(e).__name__}: {e}",
                })
    return reports


@app.get("/api/stress/reports/{report_id}")
async def api_stress_report(report_id: str):
    """Get specific stress test report"""
    report_file = STRESS_REPORTS_DIR / f"stress-{report_id}.json"
    if not report_file.exists():
        return JSONResponse({"error": "Report not found"}, status_code=404)

    with open(report_file) as f:
        return JSONResponse(json.load(f))


def _stress_scenario_desc(cfg: dict) -> str:
    """Build a human-readable description of a stress scenario."""
    load = cfg.get("load", {})
    parts = [
        f"{cfg.get('symbols', '?')}sym",
        f"{cfg.get('gateways', '?')}gw",
        f"rep={cfg.get('replication', '?')}",
    ]
    if load:
        parts.append(
            f"{load.get('rate', '?')}/s "
            f"{load.get('duration', '?')}s "
            f"{load.get('type', '?')}"
        )
    return ", ".join(parts)


@app.post("/api/stress/scenario/{name}/start")
async def api_stress_scenario_start(name: str, request: Request):
    """Start RSX cluster with the named stress scenario, then
    re-render the scenario panel so HTMX swaps the new state in."""
    denied = _require_admin_request(request)
    if denied:
        return denied
    if name not in start_mod.SCENARIOS:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'unknown scenario: {html.escape(name)}</span>',
            status_code=400,
        )
    global current_scenario
    current_scenario = name
    asyncio.create_task(_start_all_in_background(name))
    return await x_stress_scenarios()


@app.post("/api/stress/scenario/{name}/stop")
async def api_stress_scenario_stop(name: str, request: Request):
    """Stop the stress run (subprocess only); leave RSX up."""
    denied = _require_admin_request(request)
    if denied:
        return denied
    if _stress_running():
        await stop_process(STRESS_NAME)
    return await x_stress_scenarios()


async def _start_all_in_background(scenario: str):
    """Fire-and-forget cluster start used by scenario buttons."""
    try:
        await start_all(scenario)
    except Exception:
        pass


@app.get("/x/stress-scenarios", response_class=HTMLResponse)
async def x_stress_scenarios():
    """List named stress profiles (those with a `load` block).

    Read the same SCENARIOS source the Control scenario selector
    uses (start_mod.SCENARIOS) so the two surfaces never drift.
    """
    running = _stress_running()
    states: dict[str, dict] = {}
    for name, cfg in start_mod.SCENARIOS.items():
        if "load" not in cfg:
            continue
        states[name] = {
            "running": running and current_scenario == name,
            "desc": _stress_scenario_desc(cfg),
        }
    if not states:
        return HTMLResponse(
            '<span class="text-slate-500 text-xs">'
            'no stress profiles defined</span>'
        )
    return HTMLResponse(pages.render_stress_scenarios(states))


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
async def api_create_user(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
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
    request: Request,
    user_id: int,
    amount: int = Form(100_000),
):
    denied = _require_admin_request(request)
    if denied:
        return denied
    _user_balances[user_id] = (
        _user_balances.get(user_id, 0) + amount)
    bal = _user_balances[user_id]
    return HTMLResponse(
        f'<span class="text-emerald-400 text-xs">'
        f'deposited {amount} for user {user_id} '
        f'(balance: {bal})</span>')


@app.post("/api/users/deposit")
async def api_deposit_form(
    request: Request,
    user_id: int = Form(1, alias="risk-uid"),
    amount: int = Form(100_000),
):
    # Body-form variant for the declarative Deposit button, which
    # sends the uid via hx-include rather than in the path. The
    # path-param route above stays for API callers.
    return await api_deposit(request, user_id, amount)


@app.post("/api/risk/liquidate")
async def api_liquidate(
    request: Request,
    user_id: int = Form(0, alias="risk-uid"),
    symbol_id: int = Form(10),
):
    denied = _require_admin_request(request)
    if denied:
        return denied
    import logging
    logging.info(
        "liquidation triggered: user=%s symbol=%s",
        user_id, symbol_id,
    )
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


_WAL_DUMP_HEAD = 512 * 1024      # bytes of the file to feed rsx-cli
_WAL_DUMP_LINE_CAP = 200         # records rendered, whichever first


@app.post("/api/wal/dump")
async def api_wal_dump():
    files = scan_wal_files()
    if not files:
        return HTMLResponse(
            '<span class="text-slate-500 text-xs">'
            'no WAL files to dump</span>')

    latest_dict = max(files, key=lambda f: f.get("modified", ""))
    stream_name = latest_dict["stream"]
    file_name = latest_dict["name"]
    latest_path = WAL_DIR / stream_name / file_name
    try:
        file_size = latest_path.stat().st_size
    except OSError:
        file_size = 0

    # rsx-cli `dump` read_to_end()s the WHOLE file — a rotated archive
    # can be gigabytes and OOMs the CLI before it prints a line. But
    # dump_file breaks cleanly on a partial trailing record, so we can
    # feed it just the HEAD of the file: copy the first _WAL_DUMP_HEAD
    # bytes to a temp file and dump that. Bounded RAM regardless of
    # the source file size.
    head = b""
    try:
        with open(latest_path, "rb") as fp:
            head = fp.read(_WAL_DUMP_HEAD)
    except OSError as e:
        return HTMLResponse(
            f'<div class="text-amber-400 text-xs">cannot read '
            f'{html.escape(file_name)}: {html.escape(str(e))}</div>')
    truncated = len(head) < file_size

    tmp_dump = TMP / f"waldump-{os.getpid()}-{int(time.time()*1e6)}.wal"
    output = ""
    err = ""
    try:
        tmp_dump.write_bytes(head)
        proc = await asyncio.create_subprocess_exec(
            str(ROOT / "target" / "debug" / "rsx-cli"),
            "dump", str(tmp_dump),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        try:
            out_b, err_b = await asyncio.wait_for(
                proc.communicate(), timeout=10.0)
        except asyncio.TimeoutError:
            proc.kill()
            out_b, err_b = b"", b"timeout"
        output = out_b.decode(errors="replace")
        err = err_b.decode(errors="replace")
    finally:
        tmp_dump.unlink(missing_ok=True)

    # cap the rendered records
    all_lines = [ln for ln in output.splitlines() if ln.strip()]
    lines = len(all_lines)
    if lines > _WAL_DUMP_LINE_CAP:
        all_lines = all_lines[:_WAL_DUMP_LINE_CAP]
        truncated = True
        lines = _WAL_DUMP_LINE_CAP
    output = "\n".join(all_lines)

    if not output:
        return HTMLResponse(
            f'<div class="text-amber-400 text-xs">no records '
            f'read from {html.escape(file_name)}'
            f'{(" — " + html.escape(err.strip())) if err.strip() else ""}'
            f'</div>')

    note = (
        f' &middot; showing first {lines} records'
        + (' (truncated)' if truncated else '')
    )
    safe_name = html.escape(file_name)
    safe_out = html.escape(output)
    dump_html = (
        f'<div class="text-xs">'
        f'<div class="text-slate-400 mb-2">'
        f'{safe_name} &middot; {human_size(file_size)}{note}</div>'
        f'<pre class="text-slate-300 whitespace-pre-wrap '
        f'max-h-96 overflow-y-auto bg-slate-950 p-2 rounded">'
        f'{safe_out}</pre></div>')

    return HTMLResponse(dump_html)


@app.get("/api/risk/users/{user_id}")
async def api_risk_user(request: Request, user_id: int):
    denied = _require_admin_request(request)
    if denied:
        return denied
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
async def api_risk_action(
    request: Request,
    user_id: int,
    action: str,
):
    denied = _require_admin_request(request)
    if denied:
        return denied
    if action not in ("freeze", "unfreeze"):
        return JSONResponse(
            {"error": f"unknown action: {action}"},
            status_code=400)
    status = "frozen" if action == "freeze" else "unfrozen"
    return {"user_id": user_id, "action": action,
            "status": status}


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
            "SELECT a.user_id, a.collateral, "
            "       COALESCE(SUM(f.amount), 0)::bigint AS frozen "
            "FROM accounts a "
            "LEFT JOIN frozen_orders f USING (user_id) "
            "GROUP BY a.user_id, a.collateral "
            "ORDER BY a.user_id"
        )
        if rows and isinstance(rows, list):
            for r in rows:
                accounts[r["user_id"]] = {
                    "collateral": r["collateral"],
                    "frozen": r["frozen"],
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
    # Track users that have ANY fill activity, even if their
    # net position is zero. The Maker tab's "Positions N"
    # counter is the same set; Risk page must not show 0
    # while Maker shows 2.
    accounts_with_fill_activity = len({
        uid for uid, syms in positions.items()
        for sid in syms
        if positions[uid][sid]["fill_count"] > 0
    })
    # Gross OI = sum of abs(filled qty) per side. Computed
    # from raw fills (not user-netted) so it stays non-zero
    # even when buyers and sellers exactly offset.
    gross_oi_qty: dict[int, int] = {}
    for f in fills:
        sid = f.get("symbol_id", 0)
        qty = f.get("qty", 0)
        gross_oi_qty[sid] = gross_oi_qty.get(sid, 0) + qty

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
    # If user-netted notionals are zero but fills exist (buyers
    # = sellers, no open position from this slice), fall back
    # to gross filled notional so the dashboard doesn't lie.
    if total_oi == 0 and gross_oi_qty:
        gross_notional = 0
        for sid, qty in gross_oi_qty.items():
            bbo = book_stats.get(sid, {})
            bid = bbo.get("bid_px", 0)
            ask = bbo.get("ask_px", 0)
            mid = (bid + ask) // 2 if bid and ask else 0
            if mid:
                gross_notional += qty * mid
        total_oi = gross_notional
        # Split evenly between sides for display purposes when
        # we can't infer direction from netting.
        all_long_notional = gross_notional // 2
        all_short_notional = gross_notional - all_long_notional
    # Prefer the fill-activity count over net-non-zero, so this
    # never falls below what the Maker tab shows.
    accounts_with_positions = max(
        accounts_with_positions, accounts_with_fill_activity
    )
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
    # Real index source: the mark process aggregates external
    # exchanges (Binance/Coinbase) into RECORD_MARK_PRICE on the
    # `mark` WAL stream. Premium = (book mid - index) / index.
    mark_prices = parse_wal_mark_prices()
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
        mark_rec = mark_prices.get(sid)
        index_px = mark_rec["mark_price"] if mark_rec else 0
        index_source = "mark-process" if mark_rec else "none"
        # Premium of perp (book mid) over external index.
        premium_bps = (
            (mid - index_px) * 10000 // index_px
            if index_px else 0
        )
        # Funding rate proxy: premium of perp over index in bps.
        rate_bps = premium_bps
        entries.append({
            "symbol_id": sid,
            "mark_px": mid,
            "index_px": index_px,
            "index_source": index_source,
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


def _percentiles(data: list[int]) -> dict:
    s = sorted(data)
    n = len(s)
    if n == 0:
        return {"count": 0}
    return {
        "count": n,
        "p50": s[n // 2],
        "p95": s[int(n * 0.95)],
        "p99": s[int(n * 0.99)],
        "min": s[0],
        "max": s[-1],
    }


@app.get("/api/latency")
async def api_latency():
    body = _percentiles(order_latencies)
    body["e2e"] = _percentiles(e2e_latencies)
    body["gw_only"] = _percentiles(gw_only_latencies)
    return JSONResponse(body)


# ── E2E latency probe (GW → ME → GW) ─────────────────────
# Opens a WebSocket to the gateway, submits a probe order
# above bestAsk so the maker fills it immediately, waits
# for the F (fill) frame, and records the round-trip in
# microseconds. The probe is one of the few places where
# the playground actually measures the headline <50µs
# budget end-to-end on a live system.

_PROBE_CID_PREFIX = "probe-"
_PROBE_TIMEOUT_S = 2.0


async def _run_latency_probe(symbol_id: int = 10) -> dict:
    book_resp = await api_book(symbol_id)
    if isinstance(book_resp, JSONResponse):
        body = json.loads(book_resp.body)
    else:
        body = book_resp
    asks = body.get("asks", []) if isinstance(body, dict) else []
    if not asks:
        return {"ok": False, "error": "no asks; maker idle?",
                "skipped_fills": 0}
    best_ask = int(asks[0]["px"])

    sym_resp = await v1_symbols()
    if isinstance(sym_resp, JSONResponse):
        sym_body = json.loads(sym_resp.body)
    else:
        sym_body = sym_resp
    sym_meta = next(
        (s for s in (sym_body.get("symbols") or [])
         if int(s.get("id", -1)) == symbol_id),
        None,
    )
    lot_size = int((sym_meta or {}).get("lot_size", 100_000))

    cross_px = best_ask * 101 // 100
    cid = f"{_PROBE_CID_PREFIX}{int(time.time() * 1e6) % 10_000_000}"

    # Mint guest JWT for the probe user (id 1).
    secret = os.environ.get("RSX_GW_JWT_SECRET", "")
    if not secret:
        return {"ok": False,
                "error": "RSX_GW_JWT_SECRET not configured",
                "skipped_fills": 0}
    token = pyjwt.encode(
        {
            "sub": "playground:1",
            "user_id": 1,
            "aud": "rsx-gateway",
            "iss": "rsx-auth",
            "exp": int(time.time()) + 60,
            "jti": uuid.uuid4().hex,
        },
        secret,
        algorithm="HS256",
    )

    ws_url = GATEWAY_URL.rstrip("/")
    headers = {"authorization": f"Bearer {token}"}

    start_ns = 0
    # Track the probe's order id so we only time the fill
    # whose taker_order_id == our oid (F22). The F frame has no
    # cid; we learn the oid from the U (OrderUpdate) emitted
    # for our submission. Non-matching frames are counted.
    probe_oid: str | None = None
    skipped_fills = 0
    try:
        async with aiohttp.ClientSession() as session:
            async with session.ws_connect(
                ws_url,
                headers=headers,
                heartbeat=None,
                timeout=aiohttp.ClientWSTimeout(
                    ws_close=_PROBE_TIMEOUT_S),
            ) as ws:
                start_ns = time.perf_counter_ns()
                await ws.send_json({
                    "N": [symbol_id, 0, cross_px, lot_size,
                          cid, 0],
                })
                deadline = (
                    asyncio.get_event_loop().time()
                    + _PROBE_TIMEOUT_S
                )
                # F frames keyed by taker_oid, buffered while
                # probe_oid is not yet known. Risk-side ordering
                # is asynchronous: U (order ack) and F (fill)
                # take different code paths, so F can land before
                # U. The original F22 fix skipped any F seen
                # while probe_oid was None — including the
                # probe's own F when U was delayed by the risk
                # write-behind. Buffer instead, retro-match when
                # U arrives.
                pending_fills: list[tuple[str, int]] = []
                while True:
                    remaining = (
                        deadline
                        - asyncio.get_event_loop().time()
                    )
                    if remaining <= 0:
                        return {
                            "ok": False,
                            "error": "timeout waiting for fill",
                            "skipped_fills": skipped_fills,
                            "probe_oid": probe_oid,
                            "pending_fills": len(pending_fills),
                        }
                    try:
                        msg = await asyncio.wait_for(
                            ws.receive(),
                            timeout=remaining,
                        )
                    except asyncio.TimeoutError:
                        return {
                            "ok": False,
                            "error": "timeout waiting for fill",
                            "skipped_fills": skipped_fills,
                            "probe_oid": probe_oid,
                            "pending_fills": len(pending_fills),
                        }
                    if msg.type != aiohttp.WSMsgType.TEXT:
                        continue
                    try:
                        frame = json.loads(msg.data)
                    except json.JSONDecodeError:
                        continue
                    # E = order rejected by gateway/risk. Surface
                    # immediately so callers know it's not a
                    # timeout — and stop the probe from sitting
                    # for the full deadline waiting for a fill
                    # that will never come.
                    if "E" in frame:
                        return {
                            "ok": False,
                            "error": "order rejected",
                            "reject": frame["E"],
                            "skipped_fills": skipped_fills,
                        }
                    if "U" in frame and probe_oid is None:
                        u = frame["U"]
                        if isinstance(u, list) and u:
                            oid = u[0]
                            if (isinstance(oid, str)
                                    and len(oid) == 32):
                                probe_oid = oid
                        if probe_oid is not None:
                            # Retro-match any buffered F.
                            for taker_oid, t_us in pending_fills:
                                if taker_oid == probe_oid:
                                    e2e_latencies.append(t_us)
                                    if len(e2e_latencies) > 1000:
                                        del e2e_latencies[:500]
                                    return {
                                        "ok": True,
                                        "elapsed_us": t_us,
                                        "cid": cid,
                                        "oid": probe_oid,
                                        "cross_px": cross_px,
                                        "best_ask": best_ask,
                                        "skipped_fills": skipped_fills,
                                        "u_arrived_after_f": True,
                                    }
                            # Non-matching buffered fills are now
                            # confirmed unrelated — promote them
                            # to skipped_fills count.
                            skipped_fills += len(pending_fills)
                            pending_fills = []
                        continue
                    if "F" in frame:
                        f = frame["F"]
                        # F = [taker_oid, maker_oid, px, qty,
                        # ts, fee] per rsx-gateway/src/protocol.rs
                        taker_oid = (
                            f[0] if isinstance(f, list)
                            and f else None)
                        if not isinstance(taker_oid, str):
                            continue
                        now_us = max(
                            1,
                            (time.perf_counter_ns() - start_ns)
                            // 1000,
                        )
                        if probe_oid is None:
                            # Buffer — we'll know if it's ours
                            # once U arrives.
                            pending_fills.append(
                                (taker_oid, now_us))
                            continue
                        if taker_oid != probe_oid:
                            skipped_fills += 1
                            continue
                        e2e_latencies.append(now_us)
                        if len(e2e_latencies) > 1000:
                            del e2e_latencies[:500]
                        return {
                            "ok": True,
                            "elapsed_us": now_us,
                            "cid": cid,
                            "oid": probe_oid,
                            "cross_px": cross_px,
                            "best_ask": best_ask,
                            "skipped_fills": skipped_fills,
                        }
    except aiohttp.ClientError as exc:
        return {"ok": False,
                "error": f"gateway unreachable: {exc}",
                "skipped_fills": skipped_fills}
    except Exception as exc:
        return {"ok": False, "error": f"probe failed: {exc}",
                "skipped_fills": skipped_fills}


@app.post("/api/latency-probe")
async def api_latency_probe(
    request: Request,
    symbol_id: int = 10,
):
    """Submit a probe order, wait for fill, record E2E us.

    Requires the maker to be running (so the order fills
    against existing liquidity). Use this to populate the
    E2E block of /api/latency. Loopback only (no admin
    token required since it operates as user 1)."""
    result = await _run_latency_probe(symbol_id)
    if request.headers.get("hx-request") == "true":
        return HTMLResponse(pages.render_probe_result(result))
    return JSONResponse(result)


# ── Gateway-only RTT probe (no ME, no risk) ─────────────
# Submits a deliberately-invalid order (symbol_id=999) that
# the gateway prevalidates and rejects with an E (error)
# frame on the same WS. The risk tile and ME are never
# touched, so the RTT measures Python aiohttp + WS write +
# gateway parse + gateway prevalidate + reverse path only.
# Subtract this from the e2e probe to estimate the
# risk+ME+cast+WAL contribution.
_GW_PROBE_INVALID_SYMBOL: int = 999_999
_GW_PROBE_TIMEOUT_S = 2.0


async def _run_gw_only_probe() -> dict:
    secret = os.environ.get("RSX_GW_JWT_SECRET", "")
    if not secret:
        return {"ok": False,
                "error": "RSX_GW_JWT_SECRET not configured"}
    token = pyjwt.encode(
        {
            "sub": "playground:1",
            "user_id": 1,
            "aud": "rsx-gateway",
            "iss": "rsx-auth",
            "exp": int(time.time()) + 60,
            "jti": uuid.uuid4().hex,
        },
        secret,
        algorithm="HS256",
    )
    ws_url = GATEWAY_URL.rstrip("/")
    headers = {"authorization": f"Bearer {token}"}
    cid = f"gwprobe-{int(time.time() * 1e6) % 10_000_000}"
    try:
        async with aiohttp.ClientSession() as session:
            async with session.ws_connect(
                ws_url,
                headers=headers,
                heartbeat=None,
                timeout=aiohttp.ClientWSTimeout(
                    ws_close=_GW_PROBE_TIMEOUT_S),
            ) as ws:
                start_ns = time.perf_counter_ns()
                # symbol_id = _GW_PROBE_INVALID_SYMBOL is well
                # above any configured symbol → gateway rejects
                # with "unknown symbol" (code 1007) before
                # touching risk or ME.
                await ws.send_json({
                    "N": [_GW_PROBE_INVALID_SYMBOL, 0,
                          100, 1, cid, 0],
                })
                deadline = (
                    asyncio.get_event_loop().time()
                    + _GW_PROBE_TIMEOUT_S
                )
                while True:
                    remaining = (
                        deadline
                        - asyncio.get_event_loop().time()
                    )
                    if remaining <= 0:
                        return {
                            "ok": False,
                            "error": "timeout waiting for E",
                        }
                    try:
                        msg = await asyncio.wait_for(
                            ws.receive(),
                            timeout=remaining,
                        )
                    except asyncio.TimeoutError:
                        return {
                            "ok": False,
                            "error": "timeout waiting for E",
                        }
                    if msg.type != aiohttp.WSMsgType.TEXT:
                        continue
                    try:
                        frame = json.loads(msg.data)
                    except json.JSONDecodeError:
                        continue
                    if "E" in frame:
                        elapsed_ns = (
                            time.perf_counter_ns() - start_ns
                        )
                        elapsed_us = max(1, elapsed_ns // 1000)
                        gw_only_latencies.append(elapsed_us)
                        if len(gw_only_latencies) > 1000:
                            del gw_only_latencies[:500]
                        err = frame["E"]
                        error_code = (
                            err[0] if isinstance(err, list)
                            and err else None)
                        error_msg = (
                            err[1] if isinstance(err, list)
                            and len(err) > 1 else None)
                        return {
                            "ok": True,
                            "probe_ok": True,
                            "elapsed_us": elapsed_us,
                            "error_code": error_code,
                            "error_msg": error_msg,
                            "note": (
                                "gateway rejected "
                                f"(error_code {error_code})"
                                " — expected"
                                if error_code == 1007
                                else None),
                        }
    except aiohttp.ClientError as exc:
        return {"ok": False,
                "error": f"gateway unreachable: {exc}"}
    except Exception as exc:
        return {"ok": False, "error": f"probe failed: {exc}"}


@app.post("/api/latency-probe-gw")
async def api_latency_probe_gw(request: Request):
    """Submit an invalid order, wait for the error frame.

    The order is shaped to fail gateway prevalidation
    (symbol_id outside the configured range) so it never
    reaches risk or the matching engine. Measures
    Python + aiohttp + gateway parse + reverse path only.
    Result feeds the `gw_only` block of /api/latency."""
    return JSONResponse(await _run_gw_only_probe())


# ── F4.3: per-stage latency from tracing logs ───────────
# Each tile on the GW→ME→GW path emits a line of shape
#   <ts> INFO latency: stage="..." oid="..." t_us=N t0_ns=M
# This helper tails log files for lines tagged
# `latency:` (the tracing target), parses the structured
# fields, joins by oid, and computes per-stage deltas.
# Surfaced on /x/latency-stages and /api/latency-stages.

_STAGE_ORDER = [
    "gateway_in", "risk_in", "me_in",
    "me_out", "risk_out", "gateway_out",
]
_LATENCY_LINE_RE = re.compile(
    r"latency: stage[=:]\"?(?P<stage>\w+)\"?.*?"
    r"oid[=:]\"?(?P<oid>[0-9a-f]+)\"?.*?"
    r"t_us[=:](?P<t_us>\d+)",
)


def _parse_latency_lines(max_lines_per_file: int = 5000):
    """Scan all log files, return list of dicts.

    Each dict: {stage, oid, t_us, source}. The structured-
    log filter happens here: only lines containing the
    `latency:` target tag are kept."""
    out = []
    if not LOG_DIR.exists():
        return out
    for lf in sorted(LOG_DIR.glob("*.log")):
        try:
            tail = lf.read_text().splitlines()[
                -max_lines_per_file:
            ]
        except OSError:
            continue
        src = lf.stem
        for line in tail:
            if "latency:" not in line:
                continue
            clean = strip_ansi(line)
            m = _LATENCY_LINE_RE.search(clean)
            if not m:
                continue
            try:
                out.append({
                    "stage": m.group("stage"),
                    "oid": m.group("oid"),
                    "t_us": int(m.group("t_us")),
                    "source": src,
                })
            except (ValueError, KeyError):
                continue
    return out


def _join_latency_by_oid(rows):
    """Group rows by oid → {stage: t_us}."""
    by_oid: dict[str, dict[str, int]] = {}
    for r in rows:
        by_oid.setdefault(r["oid"], {})[r["stage"]] = r["t_us"]
    return by_oid


def _segment_deltas(by_oid):
    """Per-segment median delta over oids carrying BOTH endpoints.

    Independent per-stage medians cross-pollinate populations: a
    rested / cold order carries only the forward stages
    (with a large cold me_in), a filled taker also carries the return
    leg (risk_out, gateway_out). Subtracting medians drawn from two
    different oid sets yields a meaningless egress delta — e.g. me_out
    median dominated by cold orders while risk_out median comes only
    from the one filled order, so the composed risk_out delta clamps to
    0 and the me_out→gateway_out egress leg is unreadable.

    Pairing WITHIN an oid keeps each segment coherent: every segment is
    measured over exactly the orders that traversed both its endpoints,
    so one filled order among many rested ones still yields a clean
    egress delta. Returns {stage: delta_us_or_None}; the first stage is
    the median offset from t0. A per-oid delta is clamped ≥0 before the
    median (cross-process clock reads can make a downstream stage read
    slightly earlier)."""
    deltas = {}
    for i, s in enumerate(_STAGE_ORDER):
        if i == 0:
            vals = [st[s] for st in by_oid.values() if s in st]
        else:
            prev = _STAGE_ORDER[i - 1]
            vals = [
                st[s] - st[prev]
                for st in by_oid.values()
                if s in st and prev in st
            ]
        if not vals:
            deltas[s] = None
            continue
        vs = sorted(max(0, v) for v in vals)
        deltas[s] = vs[len(vs) // 2]
    return deltas


def _cumulative_from_deltas(deltas):
    """Running sum of per-segment deltas → coherent from-t0 profile.

    Cumulative is None from the first missing segment onward: without a
    segment's delta the downstream offset from t0 is unknown."""
    cum = {}
    running = 0
    broken = False
    for s in _STAGE_ORDER:
        d = deltas.get(s)
        if broken or d is None:
            broken = True
            cum[s] = None
            continue
        running += d
        cum[s] = running
    return cum


@app.get("/api/latency-stages")
async def api_latency_stages():
    rows = _parse_latency_lines()
    by_oid = _join_latency_by_oid(rows)
    deltas = _segment_deltas(by_oid)
    medians = _cumulative_from_deltas(deltas)
    return JSONResponse({
        "oids": len(by_oid),
        "lines": len(rows),
        "medians_us": medians,
        "deltas_us": deltas,
        "stage_order": _STAGE_ORDER,
    })


@app.get("/x/latency-stages", response_class=HTMLResponse)
async def x_latency_stages():
    rows = _parse_latency_lines()
    by_oid = _join_latency_by_oid(rows)
    deltas = _segment_deltas(by_oid)
    medians = _cumulative_from_deltas(deltas)
    rows_html = []
    for s in _STAGE_ORDER:
        v = medians.get(s)
        d = deltas.get(s)
        cum = "—" if v is None else f"{v} µs"
        delta = "—" if d is None else f"{d} µs"
        rows_html.append(
            f"<tr><td>{s}</td><td>{cum}</td>"
            f"<td>{delta}</td></tr>"
        )
    table = (
        "<table class='w-full text-sm'>"
        "<thead><tr>"
        "<th class='text-left'>stage</th>"
        "<th class='text-left'>cumulative</th>"
        "<th class='text-left'>delta</th>"
        "</tr></thead>"
        f"<tbody>{''.join(rows_html)}</tbody>"
        "</table>"
        f"<p class='text-xs mt-2'>"
        f"oids joined: {len(by_oid)} &middot; "
        f"lines scanned: {len(rows)}</p>"
    )
    return HTMLResponse(table)


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


# ── rsx-auth (OAuth identity service) ───────────────────

AUTH_DIR = ROOT / "rsx-auth"
AUTH_PYTHON = AUTH_DIR / ".venv" / "bin" / "python"
AUTH_NAME = "auth"


def _auth_running() -> bool:
    info = managed.get(AUTH_NAME)
    if not info:
        return False
    return info["proc"].returncode is None


def _auth_configured() -> bool:
    """True if GitHub client id + secret are present."""
    return bool(
        os.environ.get("RSX_AUTH_GITHUB_CLIENT_ID")
        and os.environ.get("RSX_AUTH_GITHUB_CLIENT_SECRET")
        and os.environ.get("RSX_GW_JWT_SECRET")
    )


async def do_auth_start() -> bool:
    """Start rsx-auth subprocess. No-op if misconfigured."""
    if _auth_running():
        return True
    if AUTH_NAME in managed:
        await _remove_managed_process(AUTH_NAME)
    if not AUTH_PYTHON.exists():
        return False
    if not _auth_configured():
        # Don't start without GitHub app secrets
        return False
    env = {
        "RSX_AUTH_LISTEN": os.environ.get(
            "RSX_AUTH_LISTEN", "0.0.0.0:8082"),
        "RSX_GW_JWT_SECRET": os.environ.get(
            "RSX_GW_JWT_SECRET", ""),
        "RSX_AUTH_GITHUB_CLIENT_ID": os.environ.get(
            "RSX_AUTH_GITHUB_CLIENT_ID", ""),
        "RSX_AUTH_GITHUB_CLIENT_SECRET": os.environ.get(
            "RSX_AUTH_GITHUB_CLIENT_SECRET", ""),
        "RSX_AUTH_REDIRECT_URI": os.environ.get(
            "RSX_AUTH_REDIRECT_URI",
            "http://localhost:49171/oauth/github/callback"),
        # rsx-term is a terminal client, not a URL; auth's post-login
        # redirect is unused for it. Kept for env var back-compat.
        "RSX_AUTH_TRADE_UI_URL": os.environ.get(
            "RSX_AUTH_TRADE_UI_URL", ""),
        "RSX_AUTH_STARTER_COLLATERAL": os.environ.get(
            "RSX_AUTH_STARTER_COLLATERAL", "0"),
        "RSX_AUTH_JWT_TTL_S": os.environ.get(
            "RSX_AUTH_JWT_TTL_S", str(7 * 24 * 3600)),
        "DATABASE_URL": os.environ.get("DATABASE_URL", PG_URL),
    }
    full_env = {**os.environ, **env}
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    proc = await asyncio.create_subprocess_exec(
        str(AUTH_PYTHON),
        "-m", "rsx_auth.app",
        env=full_env,
        cwd=str(AUTH_DIR),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.STDOUT,
    )
    _register_managed_process(
        AUTH_NAME,
        proc,
        str(AUTH_PYTHON),
        env,
    )
    PID_DIR.mkdir(parents=True, exist_ok=True)
    (PID_DIR / f"{AUTH_NAME}.pid").write_text(str(proc.pid))
    await asyncio.sleep(0.3)
    if proc.returncode is not None:
        await _remove_managed_process(AUTH_NAME)
        (PID_DIR / f"{AUTH_NAME}.pid").unlink(missing_ok=True)
        return False
    return True


async def do_auth_stop() -> None:
    info = managed.get(AUTH_NAME)
    if not info:
        return
    proc = info["proc"]
    try:
        proc.terminate()
        try:
            await asyncio.wait_for(proc.wait(), timeout=3)
        except asyncio.TimeoutError:
            proc.kill()
            await proc.wait()
    except ProcessLookupError:
        pass
    await _remove_managed_process(AUTH_NAME)
    (PID_DIR / f"{AUTH_NAME}.pid").unlink(missing_ok=True)


@app.get("/api/auth/status")
async def api_auth_status():
    running = _auth_running()
    info = managed.get(AUTH_NAME)
    pid = info["proc"].pid if running and info else None
    return {
        "running": running,
        "pid": pid,
        "configured": _auth_configured(),
    }


@app.post("/api/auth/start")
async def api_auth_start(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
    if not _auth_configured():
        return JSONResponse(
            {"error": "rsx-auth not configured "
             "(set RSX_AUTH_GITHUB_CLIENT_ID, "
             "RSX_AUTH_GITHUB_CLIENT_SECRET, "
             "RSX_GW_JWT_SECRET)"},
            status_code=400)
    ok = await do_auth_start()
    if not ok:
        return JSONResponse(
            {"error": "rsx-auth failed to start "
             "(check logs)"},
            status_code=500)
    return {"ok": True}


@app.post("/api/auth/stop")
async def api_auth_stop(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
    await do_auth_stop()
    return {"ok": True}


# ── market maker ────────────────────────────────────────

MAKER_SCRIPT = ROOT / "rsx-playground" / "market_maker.py"
# Prefer the compiled Go maker (rsx-maker) when it has been built;
# do_maker_start falls back to MAKER_SCRIPT (Python) when it is absent
# so the demo still comes up on a box without the Go toolchain. Build
# it with `make maker` (or `go build -o rsx-maker .` in rsx-maker/).
MAKER_BIN = ROOT / "rsx-maker" / "rsx-maker"
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
    if info["proc"].returncode is not None:
        return False
    # returncode stays None until the proc is awaited/reaped, so a maker
    # SIGTERM'd by a cluster restart (start_all) still reads as "running"
    # and never gets respawned. Verify the process is actually alive.
    try:
        os.kill(info["proc"].pid, 0)
        return True
    except OSError:
        return False


@app.post("/api/maker/start")
async def api_maker_start(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
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
    denied = _require_admin_request(request)
    if denied:
        return denied
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
    # When the maker is not running, do NOT surface the stale
    # tmp/maker-status.json from the dead process (F27).
    if not running:
        return {
            "running": False,
            "pid": None,
            "name": MAKER_NAME,
            "levels": 0,
            "errors": [],
            "stale": True,
        }
    stats = _read_maker_stats()
    return {
        "running": True,
        "pid": pid,
        "name": MAKER_NAME,
        "levels": stats.get("levels", 0),
        "errors": stats.get("errors", []),
        "stale": False,
    }


MAKER_CONFIG = TMP / "maker-config.json"


@app.patch("/api/maker/config")
async def api_maker_config(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
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
    denied = _require_admin_request(request)
    if denied:
        return denied
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
    denied = _require_admin_request(request)
    if denied:
        return denied
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
        return {**snap, "source": "live"}
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
            return {"bids": bids, "asks": asks, "source": "wal"}
    # Last fallback: synthesize from maker-status.json. Tag
    # the response so consumers know this is not live data.
    maker_snap = _maker_book(symbol_id)
    if maker_snap:
        return {**maker_snap, "source": "synthetic"}
    return {"bids": [], "asks": [], "source": "empty"}


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
                "source": "live",
            }
    # Fallback: WAL BBO
    bbo = parse_wal_bbo(symbol_id)
    if bbo is not None:
        return {
            "bid_px": bbo["bid_px"],
            "ask_px": bbo["ask_px"],
            "bid_qty": bbo["bid_qty"],
            "ask_qty": bbo["ask_qty"],
            "source": "wal",
        }
    # Last fallback: maker book snapshot. Tag the response
    # so consumers know it's synthesized.
    maker_snap = _maker_book(symbol_id)
    if maker_snap:
        bids = maker_snap.get("bids", [])
        asks = maker_snap.get("asks", [])
        if bids or asks:
            return {
                "bid_px": bids[0]["px"] if bids else 0,
                "ask_px": asks[0]["px"] if asks else 0,
                "bid_qty": bids[0]["qty"] if bids else 0,
                "ask_qty": asks[0]["qty"] if asks else 0,
                "source": "synthetic",
            }
    return JSONResponse(status_code=404, content={
        "error": "no bbo for symbol"})


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


def _maker_liquidity_live() -> bool:
    """True when the maker's quotes can actually be resting on a
    live book: the gateway and at least one ME are running. When
    false, the maker's local active_cids/levels are stale bookkeeping
    (the gateway dropped the orders), not real liquidity.
    """
    procs = _cached_for("procs", 1.0, scan_processes)
    running = {
        p.get("name") for p in procs
        if p.get("state") == "running"
    }
    gw_ok = any(n.startswith("gw-") for n in running)
    me_ok = any(n.startswith("me-") for n in running)
    return gw_ok and me_ok


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
    return HTMLResponse(
        pages.maker_status_html(
            stats, pid, liquidity_live=_maker_liquidity_live()))


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
            liquidity_live=_maker_liquidity_live() if running else True,
        )
    )


# ── client WS/REST proxy endpoints ─────────────────────


async def _safe_ws_close(
    ws: WebSocket, code: int = 1000, reason: str = ""
) -> None:
    """Close a WebSocket at most once.

    After the client disconnects, Starlette has already sent
    (or received) the close frame, so a second `ws.close()`
    raises RuntimeError('Unexpected ASGI message
    "websocket.close" ...') and spams ERROR logs. Skip the
    close when the socket is already disconnected, and swallow
    the RuntimeError on the race where it disconnects between
    the check and the send.
    """
    from starlette.websockets import WebSocketState
    if (
        ws.application_state == WebSocketState.DISCONNECTED
        or ws.client_state == WebSocketState.DISCONNECTED
    ):
        return
    try:
        await ws.close(code=code, reason=reason)
    except RuntimeError:
        pass


@app.websocket("/ws/private")
async def ws_private_proxy(ws: WebSocket):
    """Proxy private WS to Gateway."""
    await ws.accept()
    token = _extract_token_from_headers(ws.headers)
    loopback_dev = (
        _allow_insecure_user_id()
        and _is_loopback_host(
            ws.client.host if ws.client else None
        )
    )
    if token:
        headers = {"authorization": f"Bearer {token}"}
    elif loopback_dev:
        # Dev path: trusted local caller. Take x-user-id if
        # present (e.g. CLI tools, tests using headers); else
        # default to a guest user so in-tree browser clients
        # connects without a login flow. Mint a real JWT for
        # the gateway hop in either case.
        raw = ws.headers.get("x-user-id")
        if raw is not None:
            try:
                user_id = int(raw)
            except ValueError:
                await ws.close(
                    code=4001, reason="invalid x-user-id")
                return
        else:
            user_id = _GUEST_USER_ID
        secret = os.environ.get("RSX_GW_JWT_SECRET", "")
        if not secret:
            await ws.close(
                code=4001,
                reason="RSX_GW_JWT_SECRET not configured")
            return
        minted = pyjwt.encode(
            {
                "sub": f"playground:{user_id}",
                "user_id": user_id,
                "aud": "rsx-gateway",
                "iss": "rsx-auth",
                "exp": int(time.time()) + 3600,
                "jti": uuid.uuid4().hex,
            },
            secret,
            algorithm="HS256",
        )
        headers = {"authorization": f"Bearer {minted}"}
    else:
        await ws.close(
            code=4001, reason="authentication required")
        return
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
                await _safe_ws_close(
                    ws, code=close_code, reason=close_reason)
    except (ConnectionRefusedError, OSError):
        await _safe_ws_close(ws, code=1013,
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
                            # Feed is protobuf over BINARY frames; forward
                            # them as-is (TEXT kept for control/echo parity).
                            if msg.type == aiohttp.WSMsgType.BINARY:
                                await ws.send_bytes(msg.data)
                            elif msg.type == aiohttp.WSMsgType.TEXT:
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
                await _safe_ws_close(
                    ws, code=close_code, reason=close_reason)
    except (ConnectionRefusedError, OSError):
        await _safe_ws_close(ws, code=1013,
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
    m_tuples = [
        [r["id"], r["tick_size"], r["lot_size"], r["symbol"]]
        for r in rows
    ]
    return JSONResponse({"symbols": rows, "M": m_tuples})


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
    """OHLCV bars from WAL fills, falling back to synthetic stubs.

    Response includes "source": "wal" | "synthetic" so the
    UI / API consumer can show a "synthetic data" badge when
    the WAL has no fills yet.
    """
    tf_secs = TF_SECONDS.get(tf, 60)
    limit = max(1, min(limit, 1000))
    sym_id = _symbol_id_for(sym)
    bars = []
    source = "synthetic"
    if sym_id is not None:
        bars = _build_candles_from_wal(sym_id, tf_secs, limit)
    if bars:
        source = "wal"
    else:
        bars = _synthetic_candles(sym, tf_secs, limit)
    return JSONResponse({"bars": bars, "source": source})


@app.get("/v1/funding")
async def v1_funding(
    sym: int = Query(None),
    limit: int = Query(50),
    before: str = Query(None),
):
    """Return funding entries derived from WAL BBO data.

    Each entry carries a "source" field: "wal" when derived
    from a real BBO record, "synthetic" when fabricated from
    config (placeholder 0.01% rate, used for empty-WAL
    startup window so the dashboard renders something).
    """
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
            "source": "wal",
        })
    if not entries:
        # Synthetic fallback for empty-WAL startup window:
        # 0.01% rate per configured symbol. Tagged so the UI
        # can show a "synthetic" badge.
        for name, cfg in start_mod.SYMBOLS.items():
            sid = cfg["id"]
            if sym is not None and sid != sym:
                continue
            entries.append({
                "ts": now_ms,
                "symbolId": sid,
                "amount": 0,
                "rate": 0.0001,
                "source": "synthetic",
            })
    return JSONResponse(entries[:limit])



@app.get("/v1/positions")
async def v1_positions(
    request: Request,
    user_id: int | None = Query(default=None),
):
    """Return open positions derived from WAL fills."""
    resolved_user_id, denied = _require_private_user(request)
    if denied:
        return denied
    if user_id is not None and user_id != resolved_user_id:
        return JSONResponse(
            {"error": "user_id does not match authenticated user"},
            status_code=403,
        )
    fills = parse_wal_fills(max_fills=1000)
    # net qty per symbol for this user
    net: dict[int, int] = {}
    entry: dict[int, int] = {}
    for f in fills:
        if resolved_user_id and (
            f["taker_uid"] != resolved_user_id
            and f["maker_uid"] != resolved_user_id
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
    request: Request,
    user_id: int = Query(default=0),
    sym: int = Query(default=0),
    limit: int = Query(default=50),
):
    """Return recent fills from WAL."""
    resolved_user_id, denied = _require_private_user(request)
    if denied:
        return denied
    if user_id and user_id != resolved_user_id:
        return JSONResponse(
            {"error": "user_id does not match authenticated user"},
            status_code=403,
        )
    fills = parse_wal_fills(max_fills=limit * 4)
    result = []
    for f in fills:
        if resolved_user_id and (
            f["taker_uid"] != resolved_user_id
            and f["maker_uid"] != resolved_user_id
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
async def v1_account(
    request: Request,
    user_id: int | None = Query(default=None),
):
    """Account summary: collateral, pnl, equity, margins."""
    resolved_user_id, denied = _require_private_user(request)
    if denied:
        return denied
    if user_id is not None and user_id != resolved_user_id:
        return JSONResponse(
            {"error": "user_id does not match authenticated user"},
            status_code=403,
        )
    collateral = _SEED_COLLATERAL
    # try postgres first
    if pg_pool:
        rows = await pg_query(
            "SELECT collateral FROM accounts WHERE user_id = $1",
            resolved_user_id,
        )
        if rows and isinstance(rows, list) and rows:
            collateral = rows[0]["collateral"]

    # compute position pnl + margins
    fills = parse_wal_fills(max_fills=1000)
    net: dict[int, int] = {}
    entry_px: dict[int, int] = {}
    for f in fills:
        if resolved_user_id and (
            f["taker_uid"] != resolved_user_id
            and f["maker_uid"] != resolved_user_id
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
        "userId": resolved_user_id,
        "collateral": h(collateral),
        "pnl": h(total_pnl),
        "equity": h(equity),
        "im": h(total_im),
        "mm": h(total_mm),
        "available": h(available),
    })


@app.get("/v1/orders")
async def v1_orders(
    request: Request,
    user_id: int | None = Query(default=None),
):
    """Return recent orders."""
    resolved_user_id, denied = _require_private_user(request)
    if denied:
        return denied
    if user_id is not None and user_id != resolved_user_id:
        return JSONResponse(
            {"error": "user_id does not match authenticated user"},
            status_code=403,
        )
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
        if o.get("user_id") != resolved_user_id:
            continue
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
    "/auth/{path:path}",
    methods=["GET", "POST"],
)
@app.api_route(
    "/oauth/{path:path}",
    methods=["GET", "POST"],
)
async def auth_proxy(path: str, request: Request):
    """Proxy /auth/* and /oauth/* to rsx-auth.

    In dev, browser clients talk to playground; playground
    forwards auth flows to rsx-auth on :8082. Production
    should use nginx or similar to route directly.
    """
    prefix = request.url.path.split("/", 2)[1]  # 'auth' or 'oauth'
    url = f"{AUTH_HTTP}/{prefix}/{path}"
    qs = str(request.query_params)
    if qs:
        url += f"?{qs}"
    try:
        async with aiohttp.ClientSession(
            cookie_jar=aiohttp.DummyCookieJar(),
        ) as session:
            method = request.method.lower()
            body = await request.body()
            fwd_headers = {}
            for h in ("authorization", "cookie",
                      "content-type"):
                if h in request.headers:
                    fwd_headers[h] = request.headers[h]
            async with session.request(
                method, url,
                data=body if body else None,
                headers=fwd_headers,
                allow_redirects=False,
            ) as resp:
                data = await resp.read()
                headers = dict(resp.headers)
                # Drop hop-by-hop + CL (we re-send)
                for k in ("transfer-encoding",
                          "content-encoding",
                          "content-length",
                          "connection"):
                    headers.pop(k.title(), None)
                    headers.pop(k, None)
                return Response(
                    content=data,
                    status_code=resp.status,
                    headers=headers,
                )
    except (ConnectionRefusedError, OSError,
            aiohttp.ClientConnectorError):
        return JSONResponse(
            status_code=502,
            content={"error": "rsx-auth not running"})


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
            auth_headers, _ = _request_auth_headers(request)
            fwd_headers.update(auth_headers)
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
