"""RSX Playground — dev dashboard.

Usage: cd rsx-playground && uv run server.py
"""

import asyncio
import dataclasses
import html
import json
import logging
import os
import random
import re
import signal
import time
import uuid
from collections import Counter
from contextlib import asynccontextmanager
from dataclasses import asdict
from datetime import datetime
from decimal import Decimal
from decimal import InvalidOperation
from pathlib import Path
from typing import cast

import aiohttp
import jwt as pyjwt
import psutil
import uvicorn
from fastapi import FastAPI
from fastapi import Form
from fastapi import Query
from fastapi import Request
from fastapi.responses import HTMLResponse
from fastapi.responses import JSONResponse
from fastapi.responses import RedirectResponse
from fastapi.responses import Response

import cast_demo
import cluster_cli
import fault_cli
import invariants
import md_wire
import notional
import order_cli
import pages
import risk_cli
import rsx_cli
import stress_cli
import symbols
import wal_feed

log = logging.getLogger(__name__)

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
stress_run = stress_cli.Stress(STRESS_REPORTS_DIR, LOG_DIR)

PG_URL = os.environ.get(
    "DATABASE_URL",
    "postgres://rsx:rsx@127.0.0.1:5432/rsx",
)

# Gateway on :8088, not :8080 — the arizuko_routd docker container owns
# host :8080 (rsx-cli `GATEWAY_WS_PORT`). Must match the gateway's listen.
GATEWAY_URL = os.environ.get(
    "GATEWAY_URL", "ws://127.0.0.1:8088"
)
ORDER_GATEWAY = order_cli.Gateway(GATEWAY_URL, PG_URL)
GATEWAY_HTTP = os.environ.get(
    "GATEWAY_HTTP", "http://127.0.0.1:8088"
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

    MAX_RETRIES = 20       # hard cap on total attempts
    CIRCUIT_AT = 8         # consecutive infra failures → open
    delay = 1.0
    max_delay = 30.0
    consec_infra = 0       # consecutive ConnectionRefused/OSError
    attempt = 0

    while attempt < MAX_RETRIES:
        attempt += 1
        await refresh_symbols(force=True)
        if not symbols.table:
            await asyncio.sleep(delay)
            delay = min(delay * 2, max_delay)
            continue
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
                    for symbol in list(symbols.table.values()):
                        await ws.send_str(
                            json.dumps({"S": [symbol.id, CHANNELS]}))

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
                                    level for level in levels
                                    if level["px"] != px
                                ]
                            else:
                                existing = next(
                                    (level for level in levels
                                     if level["px"] == px), None)
                                if existing:
                                    existing["qty"] = qty
                                else:
                                    levels.append(
                                        {"px": px, "qty": qty})
                                # re-sort: bids desc, asks asc
                                if side == 0:
                                    snap[key].sort(
                                        key=lambda level: -level["px"])
                                else:
                                    snap[key].sort(
                                        key=lambda level: level["px"])
                        # BBO: {"BBO":[sym,bid_px,bid_qty,bid_cnt,
                        #              ask_px,ask_qty,...]}
                        elif "BBO" in frame:
                            arr = frame["BBO"]
                            sid = int(arr[0])
                            bid_px = int(arr[1])
                            bid_qty = int(arr[2])
                            ask_px = int(arr[4])
                            ask_qty = int(arr[5])
                            # The BBO is the AUTHORITATIVE touch. Depth deltas
                            # drift (a dropped 'D' leaves a stale level that
                            # crosses), so reconcile every BBO: drop levels that
                            # cross the real touch, then upsert the top of each
                            # side. Deeper levels still come from the deltas.
                            snap = _book_snap.setdefault(
                                sid, {"bids": [], "asks": []})
                            if ask_px:
                                snap["bids"] = [
                                    level for level in snap["bids"]
                                    if level["px"] < ask_px
                                ]
                            if bid_px:
                                snap["asks"] = [
                                    level for level in snap["asks"]
                                    if level["px"] > bid_px
                                ]
                            for key, px, qty, desc in (
                                ("bids", bid_px, bid_qty, True),
                                ("asks", ask_px, ask_qty, False),
                            ):
                                if not px:
                                    continue
                                lvl = next(
                                    (level for level in snap[key]
                                     if level["px"] == px), None)
                                if lvl:
                                    lvl["qty"] = qty
                                else:
                                    snap[key].append(
                                        {"px": px, "qty": qty})
                                snap[key].sort(
                                    key=lambda level: -level["px"] if desc
                                    else level["px"])
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

current_scenario = "minimal"

_restart_state: dict[str, dict] = {}
_RESTART_MAX = 5
_RESTART_INIT_DELAY = 2.0
_RESTART_MAX_DELAY = 60.0

_watcher_task: asyncio.Task | None = None
_restart_tasks: set[asyncio.Task] = set()
_bulk_depth = 0


def _restart_entry(name: str) -> dict:
    return _restart_state.setdefault(name, {
        "restarts": 0,
        "total_restarts": 0,
        "blocked": False,
        "next_restart_at": 0.0,
        "last_crash_ts": 0.0,
        "intentional": False,
        "restarting": False,
    })


_symbols_loaded_at = 0.0
_SYMBOL_REFRESH_S = 5.0

plan_table: dict[str, list[cluster_cli.PlanProcess]] = {}
_plan_tried_at = 0.0
_PLAN_RETRY_S = 5.0

_scenario_list: list[cluster_cli.Scenario] = []

RISK_TTL_S = 1.0


async def read_config() -> list[risk_cli.SymbolConfig]:
    """Every symbol's config from `rsx-cli config show`, read once a second."""
    return cast(
        list[risk_cli.SymbolConfig],
        await _cached_for_async(
            "config_show", RISK_TTL_S,
            lambda: risk_cli.config_show(PG_URL)))


def bbo_marks() -> dict[int, int]:
    """Raw mid of each symbol's BBO in the WAL, for the CLI's uPnL."""
    marks = {}
    for symbol_id, bbo in wal_records.book_stats().items():
        bid = bbo.get("bid_px", 0)
        ask = bbo.get("ask_px", 0)
        if bid and ask:
            marks[symbol_id] = (bid + ask) // 2
    return marks


async def read_risk() -> risk_cli.Overview:
    """Every account, the insurance fund and the liquidations from
    `rsx-cli risk overview`, read once a second."""
    return cast(
        risk_cli.Overview,
        await _cached_for_async(
            "risk_overview", RISK_TTL_S,
            lambda: risk_cli.risk_overview(PG_URL, bbo_marks())))


async def refresh_symbols(force: bool = False) -> None:
    global _symbols_loaded_at
    now = time.monotonic()
    if not force and now - _symbols_loaded_at < _SYMBOL_REFRESH_S:
        return
    _symbols_loaded_at = now
    try:
        configs = await read_config()
    except rsx_cli.CliError as err:
        log.warning("symbol config unavailable: %s", err)
        return
    fresh = {}
    for config in configs:
        row = config.active()
        if row is None or config.name is None:
            continue
        fresh[config.name] = symbols.Symbol(
            config.symbol_id, config.name, row.tick_size, row.lot_size,
            row.price_decimals, row.qty_decimals)
    symbols.table.clear()
    symbols.table.update(fresh)


async def refresh_plan() -> None:
    global _plan_tried_at
    if current_scenario in plan_table:
        return
    now = time.monotonic()
    if now - _plan_tried_at < _PLAN_RETRY_S:
        return
    _plan_tried_at = now
    try:
        plan_table[current_scenario] = await cluster_cli.plan(
            current_scenario)
    except rsx_cli.CliError as err:
        log.warning("spawn plan of %s unavailable: %s",
                    current_scenario, err)


async def refresh_catalog() -> None:
    await refresh_plan()
    await refresh_symbols()


def plan_processes() -> list[cluster_cli.PlanProcess]:
    return plan_table.get(current_scenario, [])


async def get_scenarios() -> list[cluster_cli.Scenario]:
    if not _scenario_list:
        _scenario_list.extend(await cluster_cli.scenarios())
    return _scenario_list


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


def forget_procs() -> None:
    _TTL_CACHE.pop("procs", None)


async def _running_pid(name: str) -> int | None:
    for proc in await scan_processes():
        if proc["name"] == name and proc["state"] == "running":
            return int(proc["pid"])
    return None


@asynccontextmanager
async def bulk_operation():
    """The watcher leaves the processes alone while all of them are being
    started, stopped or wiped, and a restart already in flight finishes
    first."""
    global _bulk_depth
    _bulk_depth += 1
    try:
        await asyncio.gather(*list(_restart_tasks))
        yield
    finally:
        _bulk_depth -= 1


async def start_all(scenario="minimal"):
    """Seed the test accounts, then `cluster up` the scenario (building
    its binaries). A cluster of another scenario is stopped first."""
    global current_scenario
    try:
        async with bulk_operation():
            view = await cluster_cli.ps()
            if view.scenario not in (None, scenario) and any(
                    row.state == "running" for row in view.rows):
                await cluster_cli.down()
            await risk_cli.account_seed(PG_URL)
            report = await cluster_cli.up(scenario, build=True)
    except rsx_cli.CliError as err:
        forget_procs()
        return {"error": str(err)}
    current_scenario = scenario
    plan_table.pop(scenario, None)
    for entry in _restart_state.values():
        entry["intentional"] = False
    forget_procs()
    await refresh_catalog()
    return {
        "started": report.started,
        "count": len(report.started),
        "skipped": sum(
            1 for skipped in report.skipped
            if skipped.reason == "already running"),
    }


async def stop_all():
    """`cluster down`, and the stress run."""
    async with bulk_operation():
        report = await cluster_cli.down()
    stopped = list(report.stopped)
    if stress_run.is_running():
        await stress_run.stop()
        stopped.append(STRESS_NAME)
    forget_procs()
    return {"stopped": stopped}


async def restart_exited(name: str) -> None:
    try:
        await cluster_cli.restart(name, current_scenario)
    except rsx_cli.CliError as err:
        log.warning("process watcher: restart of %s failed: %s", name, err)
    finally:
        _restart_entry(name)["restarting"] = False
        forget_procs()


def watch_crashes(view: cluster_cli.View) -> None:
    """Restart the processes that exited without being stopped, with
    bounded exponential backoff; open the circuit after
    _RESTART_MAX consecutive crashes."""
    if _bulk_depth:
        return
    now = time.time()
    for row in view.rows:
        entry = _restart_entry(row.name)
        if row.state == "running":
            entry["restarts"] = 0
            continue
        if row.state == "absent":
            entry["intentional"] = False
            continue
        if (entry["intentional"] or entry["blocked"]
                or entry["restarting"] or now < entry["next_restart_at"]):
            continue
        entry["restarts"] += 1
        entry["total_restarts"] += 1
        entry["last_crash_ts"] = now
        attempt = entry["restarts"]
        if attempt > _RESTART_MAX:
            entry["blocked"] = True
            log.warning(
                "process watcher: %s circuit open — %d consecutive "
                "crashes; marking blocked", row.name, attempt - 1)
            continue
        delay = min(
            _RESTART_INIT_DELAY * (2 ** (attempt - 1)),
            _RESTART_MAX_DELAY,
        )
        entry["next_restart_at"] = now + delay
        entry["restarting"] = True
        log.info(
            "process watcher: restarting %s (attempt %d/%d, "
            "backoff %.0fs)", row.name, attempt, _RESTART_MAX, delay)
        task = asyncio.create_task(
            restart_exited(row.name), name=f"restart-{row.name}")
        _restart_tasks.add(task)
        task.add_done_callback(_restart_tasks.discard)


async def _process_watcher():
    """Every 2 s: refresh the plan and symbols, restart crashed
    processes (watch_crashes) and keep the marketdata subscriber alive."""
    global _md_ws_task
    while True:
        try:
            await asyncio.sleep(2.0)
            await refresh_catalog()
            watch_crashes(await cluster_cli.ps())
            if _md_ws_task is None or _md_ws_task.done():
                _md_ws_task = asyncio.create_task(
                    _md_ws_subscriber(), name="md-ws-subscriber")
        except asyncio.CancelledError:
            break
        except Exception as exc:
            log.error("process watcher: unexpected error: %s", exc)


_reaper_task: asyncio.Task | None = None
_wal_feed_task: asyncio.Task | None = None


async def wal_feed_loop():
    while True:
        await wal_records.refresh(WAL_DIR)
        await asyncio.sleep(WAL_FEED_POLL_S)


@asynccontextmanager
async def lifespan(app):
    global _md_ws_task, _watcher_task, _reaper_task, _wal_feed_task
    await rsx_cli.check_binary()
    rsx_cli.reap_stale()
    await scan_processes()
    _md_ws_task = asyncio.create_task(_md_ws_subscriber())
    _watcher_task = asyncio.create_task(_process_watcher())
    _reaper_task = asyncio.create_task(_stale_session_reaper())
    _wal_feed_task = asyncio.create_task(wal_feed_loop())
    yield
    for task in (
            _md_ws_task, _watcher_task, _reaper_task, _wal_feed_task):
        if task:
            task.cancel()
            try:
                await task
            except asyncio.CancelledError:
                pass
    await wal_records.reset()
    await stress_run.stop()
    # cleanup server PID file (launcher writes it into PID_DIR)
    server_pid_file = PID_DIR / f"{SELF_PID_NAME}.pid"
    if server_pid_file.exists():
        server_pid_file.unlink()


app = FastAPI(
    title="RSX Playground",
    lifespan=lifespan,
    docs_url=None,
    redoc_url=None,
)


@app.exception_handler(rsx_cli.CliError)
async def rsx_cli_failed(request: Request, err: rsx_cli.CliError):
    if request.url.path.startswith("/x/"):
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            f'Failed: rsx-cli: {html.escape(str(err))}</span>')
    return JSONResponse({"error": f"rsx-cli: {err}"}, status_code=502)


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
    return {}, "missing authenticated user context"


def _resolve_request_user(request: Request) -> tuple[int | None, str]:
    token = _extract_token_from_headers(request.headers)
    if token:
        return _decode_user_token(token)
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


async def _caller_role(request: Request) -> str | None:
    """Caller's own management role (viewer/operator/admin/auditor),
    read fresh from rsx-auth's /roles/me using the caller's own
    forwarded auth headers -- never trust a role embedded in a token
    the playground itself decodes locally, and never a role for a
    different user_id. Returns None on any failure (no auth header,
    rsx-auth unreachable, non-200, no role): callers treat None as
    "not privileged", never as "trust anyway". This is a UI-gating
    convenience only -- rsx-auth's own `require(action)` dependencies
    are the actual authority on every write this proxies through to.
    """
    auth_headers, _ = _request_auth_headers(request)
    if "authorization" not in auth_headers:
        return None
    try:
        async with aiohttp.ClientSession() as session:
            async with session.get(
                f"{AUTH_HTTP}/roles/me",
                headers=auth_headers,
                timeout=aiohttp.ClientTimeout(total=1.5),
            ) as resp:
                if resp.status != 200:
                    return None
                data = await resp.json()
                return data.get("role")
    except (aiohttp.ClientError, asyncio.TimeoutError, OSError):
        return None


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


def process_counts(procs: list[dict]) -> tuple[int, int]:
    """(running, expected) over the scenario's processes, the maker
    included; the single definition behind every running/expected
    surface (nav chip, pulse, key metrics, verify, healthz). The
    stress run is not one of them."""
    cluster = [p for p in procs if p["name"] != STRESS_NAME]
    running = sum(1 for p in cluster if p.get("state") == "running")
    return running, len(cluster)


@app.get("/healthz")
async def healthz():
    """Health check for CLI."""
    procs = await scan_processes()
    running, expected = process_counts(procs)
    gateway_up, marketdata_up, postgres_up = await asyncio.gather(
        _probe_gateway_tcp(),
        _probe_marketdata_tcp(),
        _probe_postgres_tcp(),
    )
    return {
        "status": "ok",
        "port": 49171,
        "processes_running": running,
        "processes_total": expected,
        "postgres": postgres_up,
        "gateway": gateway_up,
        "marketdata": marketdata_up,
    }

# ── in-memory state ─────────────────────────────────────

recent_orders: list[dict] = []
# Epoch-seconds timestamps of recent order submissions, used by
# /x/key-metrics for a 30s sliding-window Msgs/sec (F26).
recent_order_ts: list[float] = []
verifier = invariants.Verifier()
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


def _check_idempotency(idem_key: str | None) -> bool:
    """True if the caller may proceed (key unused or absent); False if
    this is a duplicate submission (key seen within _IDEMPOTENCY_TTL).
    Records `idem_key` as used when returning True. Shared by the
    admin harness and the authenticated trade path.
    """
    if not idem_key:
        return True
    now = time.time()
    expired = [
        k for k, t in _idempotency_keys.items()
        if now - t > _IDEMPOTENCY_TTL
    ]
    for k in expired:
        del _idempotency_keys[k]
    if idem_key in _idempotency_keys:
        return False
    _idempotency_keys[idem_key] = now
    return True
SERVER_START: float = time.time()

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


def _stopped_entry(name: str) -> dict:
    entry = _restart_state.get(name, {})
    return {
        "name": name, "pid": "-",
        "state": "blocked" if entry.get("blocked") else "stopped",
        "cpu": "-", "mem": "-", "uptime": "-",
        "restarts": entry.get("restarts", 0),
        "total_restarts": entry.get("total_restarts", 0),
    }


def _running_entry(name: str, pid: int) -> dict:
    total = _restart_state.get(name, {}).get("total_restarts", 0)
    try:
        ps = _get_ps(pid)
        return {
            "name": name, "pid": pid, "state": "running",
            "cpu": f"{ps.cpu_percent():.1f}%",
            "mem": human_size(ps.memory_info().rss),
            "uptime": human_uptime(ps.create_time()),
            "total_restarts": total,
        }
    except (psutil.NoSuchProcess, psutil.AccessDenied):
        _evict_ps(pid)
        return {
            "name": name, "pid": pid, "state": "running",
            "cpu": "-", "mem": "-", "uptime": "-",
            "total_restarts": total,
        }


async def _scan_processes() -> list[dict]:
    global current_scenario
    await refresh_catalog()
    view = await cluster_cli.ps()
    if view.scenario:
        current_scenario = view.scenario
    result = []
    listed = set()
    for row in view.rows:
        listed.add(row.name)
        if row.state == "running" and row.pid is not None:
            result.append(_running_entry(row.name, row.pid))
        else:
            result.append(_stopped_entry(row.name))
    for proc in plan_processes():
        if proc.name not in listed:
            result.append(_stopped_entry(proc.name))
    if stress_run.pid is not None:
        if stress_run.is_running():
            result.append(_running_entry(STRESS_NAME, stress_run.pid))
        else:
            result.append(_stopped_entry(STRESS_NAME))
    return sorted(result, key=lambda p: p["name"])


async def scan_processes() -> list[dict]:
    """The scenario's processes from `rsx-cli cluster ps` (the plan's
    names when they have not started), one row each, and the stress run;
    cached for a second, as every dashboard poll shares it."""
    return cast(
        list[dict],
        await _cached_for_async("procs", 1.0, _scan_processes))


WAL_LISTING_TTL_S = 1.0


async def wal_listings() -> list[wal_feed.Listing]:
    return await _cached_for_async(
        f"wal_listings:{WAL_DIR}", WAL_LISTING_TTL_S,
        lambda: wal_feed.list_streams(WAL_DIR))


async def list_wal_streams() -> list[dict]:
    """One row per top-level WAL directory, from `rsx-cli wal ls`.

    Single source of truth for the WAL tab, the Verify page and the
    key-metrics card, so the surfaces never disagree.
    """
    rows = []
    for listing in await wal_listings():
        segments = [
            seg for stream in listing.streams
            for seg in stream.segments]
        total = sum(seg.bytes for seg in segments)
        newest = ""
        if segments:
            newest = datetime.fromtimestamp(
                max(seg.modified_ms for seg in segments) / 1000
            ).strftime("%H:%M:%S")
        rows.append({
            "name": listing.name,
            "files": len(segments),
            "total_size": human_size(total),
            "total_bytes": total,
            "newest": newest,
        })
    return rows


async def list_wal_files() -> list[dict]:
    files = []
    for listing in await wal_listings():
        for stream in listing.streams:
            label = str(stream.stream_id)
            if stream.wal_dir != WAL_DIR:
                label = f"{stream.label}/{stream.stream_id}"
            for seg in stream.segments:
                files.append({
                    "stream": label,
                    "name": seg.name,
                    "size": human_size(seg.bytes),
                    "modified": datetime.fromtimestamp(
                        seg.modified_ms / 1000).strftime("%H:%M:%S"),
                })
    return files


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


async def recovery_feed_events(limit: int = 40) -> list[dict]:
    """Build the newest-first recovery event feed.

    Freshest block = current process states (scan_processes);
    below it = recovery-relevant log lines (FAULTED / replay /
    caught-up / CRC), newest-first.
    """
    events: list[dict] = []
    now = datetime.now().strftime("%b %d %H:%M:%S")
    for p in await scan_processes():
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


def find_running(procs: list[dict], name: str) -> dict | None:
    return next(
        (p for p in procs
         if p["name"] == name and p["state"] == "running"),
        None,
    )


def _topo_proc(hints: list[str], procs: list[dict]) -> dict:
    """Return first running process matching any hint; else first match."""
    fallback = {}
    for hint in hints:
        for p in procs:
            if hint in p["name"]:
                if p.get("state") == "running":
                    return p
                if not fallback:
                    fallback = p
    return fallback


def _topo_gateway(procs: list[dict]) -> dict:
    p = _topo_proc(PROC_HINTS["gateway"], procs)
    book_stats = wal_records.book_stats()
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


def _topo_risk(procs: list[dict]) -> dict:
    p = _topo_proc(PROC_HINTS["risk"], procs)
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
        ],
    }


def _topo_matching(procs: list[dict]) -> dict:
    p = _topo_proc(PROC_HINTS["matching"], procs)
    book_stats = wal_records.book_stats()
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


def _topo_marketdata(procs: list[dict]) -> dict:
    p = _topo_proc(PROC_HINTS["marketdata"], procs)
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


def _topo_mark(procs: list[dict]) -> dict:
    p = _topo_proc(PROC_HINTS["mark"], procs)
    rows: list[tuple[str, object]] = []
    # Current mark prices (BBO mid) per active symbol.
    book_stats = wal_records.book_stats()
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


def _topo_recorder(procs: list[dict]) -> dict:
    p = _topo_proc(PROC_HINTS["recorder"], procs)
    return {
        "name": "Recorder",
        "status": p.get("state", "stopped"),
        "pid": p.get("pid", "-"),
        "uptime": p.get("uptime", "-"),
        "rows": [("WAL files found", wal_records.file_count())],
    }


def _topo_maker(procs: list[dict]) -> dict:
    maker = find_running(procs, MAKER_NAME)
    running = maker is not None
    pid = maker["pid"] if maker else "-"
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
    p = _topo_proc(PROC_HINTS["maker"], procs)
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


def _topo_client(procs: list[dict]) -> dict:
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


def _topo_stress(procs: list[dict]) -> dict:
    running = stress_run.is_running()
    pid = stress_run.pid if running else "-"
    p = _topo_proc(PROC_HINTS["stress"], procs)
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
    procs = await scan_processes()

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
    maker_s = "running" if find_running(procs, MAKER_NAME) else "stopped"

    book_stats = wal_records.book_stats()
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
    procs = await scan_processes()
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
    return HTMLResponse(pages.render_component_detail(
        component, handler(await scan_processes())))


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


@app.get("/demos", response_class=HTMLResponse)
async def demos_gallery():
    """Gallery of every crate's recorded demo GIF (served locally)."""
    return HTMLResponse(pages.demos_page())


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


@app.get("/gateway", response_class=HTMLResponse)
async def gateway():
    return HTMLResponse(pages.gateway_page())


@app.get("/matching", response_class=HTMLResponse)
async def matching():
    return HTMLResponse(pages.matching_page())


async def _fetch_admin_users(request: Request):
    """List users+roles via rsx-auth's GET /roles/users, using the
    caller's OWN forwarded auth headers (their own admin token) --
    rsx-auth's own `read_user_state` RBAC check is what actually
    authorizes this read. Returns (users, error)."""
    auth_headers, _ = _request_auth_headers(request)
    try:
        async with aiohttp.ClientSession() as session:
            async with session.get(
                f"{AUTH_HTTP}/roles/users",
                headers=auth_headers,
                params={"limit": "200"},
                timeout=aiohttp.ClientTimeout(total=3.0),
            ) as resp:
                if resp.status != 200:
                    return [], f"rsx-auth returned {resp.status}"
                data = await resp.json()
                return data.get("users", []), None
    except (aiohttp.ClientError, asyncio.TimeoutError, OSError) as exc:
        return [], f"rsx-auth unreachable: {exc}"


async def _fetch_user_collateral() -> dict[int, int]:
    """user_id -> collateral, from risk's accounts (balances are
    risk-owned, not something rsx-auth's roles table carries)."""
    overview = await read_risk()
    return {
        account.user_id: account.collateral for account in overview.users}


@app.get("/users", response_class=HTMLResponse)
async def users(request: Request):
    """Admin-only user + role management. Gated on the CALLER's own
    live role claim (_caller_role, re-checked against rsx-auth on
    every load -- never a cached/local decision), distinct from the
    loopback/PLAYGROUND_ADMIN_TOKEN gate the ops harness uses."""
    role = await _caller_role(request)
    if role != "admin":
        content = (
            '<div class="text-sm text-red-400 border '
            'border-red-500/60 rounded-[3px] bg-slate-800/50 p-4">'
            'admin access required &mdash; sign in with an '
            'admin-role account (top-right of the nav) to manage '
            'users.</div>'
        )
        return HTMLResponse(
            pages.layout("Users", content, "./users"),
            status_code=403)
    users_list, list_err = await _fetch_admin_users(request)
    collateral = await _fetch_user_collateral()
    return HTMLResponse(
        pages.users_page(users_list, list_err, collateral))


async def fetch_active_config() -> tuple[list[dict], str]:
    """The row risk runs on for each symbol, as page rows, and the
    reason the CLI could not read them."""
    try:
        configs = await read_config()
    except rsx_cli.CliError as err:
        return [], str(err)
    rows = []
    for config in configs:
        row = config.active()
        if row is not None:
            rows.append(
                dataclasses.asdict(row) | {"symbol_name": config.name})
    return rows, ""


@app.get("/config", response_class=HTMLResponse)
async def config_view(request: Request):
    """Symbol risk parameters. Admin-gated on the caller's own live
    role claim, same as /users -- margin rates decide who can trade
    how much, so this is not a read-only ops panel."""
    role = await _caller_role(request)
    if role != "admin":
        content = (
            '<div class="text-sm text-red-400 border '
            'border-red-500/60 rounded-[3px] bg-slate-800/50 p-4">'
            'admin access required &mdash; sign in with an '
            'admin-role account to view or change risk config.</div>'
        )
        return HTMLResponse(
            pages.layout("Config", content, "./config"),
            status_code=403)
    configs, err = await fetch_active_config()
    return HTMLResponse(pages.config_page(configs, err))


def _schedule_failure(status: int, text: str) -> HTMLResponse:
    return HTMLResponse(
        f'<span class="text-red-400">{html.escape(text)}</span>',
        status_code=status)


@app.post("/api/config/schedule", response_class=HTMLResponse)
async def api_config_schedule(request: Request):
    """Insert a NEW config version for a symbol (`rsx-cli config
    schedule`). Never updates the active row -- the schedule table is
    the audit trail."""
    role = await _caller_role(request)
    if role != "admin":
        return HTMLResponse(
            '<span class="text-red-400">admin role required</span>',
            status_code=403)
    form = await request.form()

    def field(name):
        raw = form.get(name)
        return int(raw) if isinstance(raw, str) and raw.strip() else None

    try:
        symbol_id = field("symbol_id")
        initial = field("initial_margin_rate_bps")
        maint = field("maintenance_margin_rate_bps")
        leverage = field("max_leverage")
        effective_in = field("effective_in_sec") or 0
    except ValueError:
        return _schedule_failure(400, "all fields must be whole numbers")

    if (symbol_id is None or initial is None
            or maint is None or leverage is None):
        return _schedule_failure(400, "missing field")

    config = next(
        (config for config in await read_config()
         if config.symbol_id == symbol_id), None)
    if config is None:
        return _schedule_failure(404, f"unknown symbol {symbol_id}")

    settings = {
        "initial_margin_rate_bps": initial,
        "maintenance_margin_rate_bps": maint,
        "max_leverage": leverage,
    }
    # Blank keeps the symbol's current minimum; a value is USD, turned
    # into the raw integer with that symbol's own decimals.
    min_order_usd = None
    usd_text = form.get("min_notional_usd")
    if isinstance(usd_text, str) and usd_text.strip():
        newest = max(config.scheduled, key=lambda row: row.config_version)
        try:
            usd = notional.parse_usd(usd_text)
            settings["min_notional"] = notional.raw_from_usd(
                usd, newest.price_decimals, newest.qty_decimals)
        except ValueError as exc:
            return _schedule_failure(400, str(exc))
        min_order_usd = notional.usd_from_raw(
            settings["min_notional"],
            newest.price_decimals, newest.qty_decimals)

    effective_at = int(time.time() * 1000) + effective_in * 1000
    try:
        scheduled = await risk_cli.config_schedule(
            PG_URL, symbol_id, settings, effective_at)
    except rsx_cli.CliError as err:
        status = 400 if err.code == risk_cli.EXIT_MISUSE else 502
        return _schedule_failure(status, f"schedule failed: {err}")
    _TTL_CACHE.pop("config_show", None)

    min_order = (
        f", min order {min_order_usd} USD" if min_order_usd else "")
    return HTMLResponse(
        f'<span class="text-emerald-400">scheduled '
        f'v{scheduled.config_version} for symbol {symbol_id}, '
        f'effective in {effective_in}s{min_order}</span>')


@app.get("/wal", response_class=HTMLResponse)
async def wal():
    return HTMLResponse(pages.wal_page())


@app.get("/logs", response_class=HTMLResponse)
async def logs_page():
    return HTMLResponse(pages.logs_page())


@app.get("/control", response_class=HTMLResponse)
async def control():
    return HTMLResponse(pages.control_page(await get_scenarios()))


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
    pid = await _running_pid(MAKER_NAME)
    running = pid is not None
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

    try:
        data = stress_cli.read_report(report_file)
        page = pages.stress_report_page(report_id, data)
    except (OSError, ValueError, KeyError, TypeError) as exc:
        return HTMLResponse(
            f"<h1>Report unreadable: {html.escape(str(exc))}</h1>",
            status_code=422)
    return HTMLResponse(page)


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
    procs = await scan_processes()
    return HTMLResponse(pages.render_process_table(procs))


@app.get("/x/proc-chip", response_class=HTMLResponse)
async def x_proc_chip():
    procs = await scan_processes()
    running, expected = process_counts(procs)
    return HTMLResponse(
        pages.render_proc_chip(procs, running, expected))


@app.get("/x/session-chip", response_class=HTMLResponse)
async def x_session_chip(request: Request):
    """Nav identity chip: caller's own resolved user, never an admin
    impersonation. Login/role come from rsx-auth's /auth/me, called with
    the caller's own forwarded auth headers (`_request_auth_headers`) --
    the same source of truth Gateway itself uses, not a second decoder.
    """
    user_id, _ = _resolve_request_user(request)
    if user_id is None:
        return HTMLResponse(pages.render_session_chip(None))
    login, role = None, None
    auth_headers, _ = _request_auth_headers(request)
    try:
        async with aiohttp.ClientSession() as session:
            async with session.get(
                f"{AUTH_HTTP}/auth/me",
                headers=auth_headers,
                timeout=aiohttp.ClientTimeout(total=1.5),
            ) as resp:
                if resp.status == 200:
                    data = await resp.json()
                    login = data.get("login")
                    role = data.get("role")
    except (aiohttp.ClientError, asyncio.TimeoutError, OSError):
        pass  # rsx-auth unreachable -- fall back to the bare user_id chip
    return HTMLResponse(
        pages.render_session_chip(user_id, login, role))


_BENCH_BASELINE_FILE = ROOT / "bench-baseline.json"


def _read_baseline_e2e_p99_us() -> float | None:
    """Read e2e_us.p99 from bench-baseline.json, or None."""
    try:
        data = json.loads(_BENCH_BASELINE_FILE.read_text())
        return float(data.get("e2e_us", {}).get("p99", 0)) or None
    except Exception:
        return None


# HTMX panels poll every 2-5s; the playground's expensive
# aggregates share a one-second TTL so concurrent polls collapse
# onto one computation.
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


async def _cached_for_async(key: str, ttl_seconds: float, compute_fn):
    """_cached_for for an async compute_fn (a Postgres read)."""
    now = time.time()
    hit = _TTL_CACHE.get(key)
    if hit is not None:
        ts, val = hit
        if now - ts < ttl_seconds:
            return val
    val = await compute_fn()
    _TTL_CACHE[key] = (now, val)
    return val


async def _compute_health() -> dict:
    """Compute truthful health score.

    Signals (each subtracts from a starting 100):
      - process restart count (any process restarted = -10/restart)
      - latency p99 ≥ 2× baseline (-30)
      - recent ERROR / panic lines in logs (-5 per up to 6)
      - any /verify entry with status == "fail" forces RED (≤49)
    Returns dict with score, label, reasons. Returns
    {"score": None} when we genuinely don't know.
    """
    procs = await scan_processes()
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
    # A failing invariant forces the health score into the red band.
    fail_count = (
        len(verifier.report.failed_checks()) if verifier.report else 0
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
    h = await _cached_for_async("health", 1.0, _compute_health)
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
        "filled", "partial", "cancelled", "rejected",
        "failed", "expired",
    }
    ao = sum(
        1 for order in recent_orders
        if order.get("status", "") not in terminal)
    fills = _cached_for(
        "wal_fills_2000", 1.0,
        lambda: wal_records.fills(2000))
    pairs = set()
    for f in fills:
        taker = f.get("taker_user_id", 0)
        maker = f.get("maker_user_id", 0)
        sid = f.get("symbol_id", 0)
        if taker:
            pairs.add((taker, sid))
        if maker:
            pairs.add((maker, sid))
    pos_count = len(pairs)
    # Count submissions over a sliding 30-second window.
    window_s = 30
    now = time.time()
    cutoff = now - window_s
    recent_in_window = sum(
        1 for ts in recent_order_ts if ts >= cutoff)
    mps = int(recent_in_window / window_s)
    errs = _cached_for(
        "recent_errors", 1.0, _count_recent_errors)
    procs = await scan_processes()
    proc_running, proc_expected = process_counts(procs)
    streams = await list_wal_streams()
    return HTMLResponse(
        pages.render_key_metrics(
            procs, streams,
            active_orders=ao, positions=pos_count,
            msgs_sec=mps, error_count=errs,
            proc_running=proc_running, proc_expected=proc_expected))


@app.get("/x/pulse", response_class=HTMLResponse)
async def x_pulse():
    procs = await scan_processes()
    running, expected = process_counts(procs)
    streams = await list_wal_streams()
    wal_files = sum(s.get("files", 0) for s in streams)
    elapsed = max(1, time.time() - SERVER_START)
    ops = int(len(recent_orders) / elapsed)
    # Errors: union of rejected-orders and log-derived errors so
    # the pulse pill never says "errs 0" while the Logs tab is
    # showing WARN/ERR lines.
    errs = sum(
        1 for order in recent_orders
        if order.get("status") in {"rejected", "failed"})
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
        pages.render_ring_pressure(await list_wal_streams()))


@app.get("/x/invariant-status",
         response_class=HTMLResponse)
async def x_invariant_status():
    if not verifier.running and not verifier.is_fresh(5.0):
        await run_verify_now()
    return HTMLResponse(
        pages.render_invariant_status(verifier.report))


@app.get("/x/core-affinity", response_class=HTMLResponse)
async def x_core_affinity():
    return HTMLResponse(
        pages.render_core_affinity(await scan_processes()))


@app.get("/x/cast-flows", response_class=HTMLResponse)
async def x_cast_flows():
    # Truthful per-pipe counters (see _cast_pipe_counts):
    # ME->Mktdata from the ME's per-symbol WAL; gw/risk write NO
    # WAL, so their legs come from the daemon health /metrics
    # endpoints, or render "live" when those aren't exposed.
    await refresh_catalog()
    counts = _cached_for("cast_pipe_counts", 1.0, _cast_pipe_counts)
    return HTMLResponse(pages.render_cast_flows(counts))


def _fetch_daemon_metrics(addr: str) -> dict:
    """Read a daemon's health /metrics JSON.

    Raises OSError (unreachable, timeout, HTTP error) or
    ValueError (not JSON). Short timeout — the health server runs
    off the daemon's hot path (rsx-health), so this never touches
    the order path.
    """
    import urllib.request
    with urllib.request.urlopen(
            f"http://{addr}/metrics", timeout=0.3) as r:
        return json.loads(r.read().decode("utf-8"))


def _fetch_daemon_counter(addr: str, counter: str):
    """Read one counter from a daemon's health /metrics endpoint.

    Returns the int value, or None when the endpoint is
    unreachable or the counter is absent. Best-effort display
    helper: a caller that must report why uses
    _fetch_daemon_metrics.
    """
    try:
        data = _fetch_daemon_metrics(addr)
    except (OSError, ValueError):
        return None
    for c in data.get("counters", []):
        if c.get("name") == counter:
            try:
                return int(c.get("value", 0))
            except (TypeError, ValueError):
                return None
    return None


def _daemon_counter_sum(prefix: str, env_key: str, counter: str):
    """Sum a health counter across the plan's daemons whose name
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
    for proc in plan_processes():
        if not proc.name.startswith(prefix):
            continue
        addr = proc.env.get(env_key)
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
    me_to_mkt = wal_records.count_seen({"fill", "bbo"}, "mark")
    return {
        "gw_to_risk": _daemon_counter_sum(
            "gw-", "RSX_GW_HEALTH_ADDR", "orders_processed"),
        "risk_to_me": _daemon_counter_sum(
            "risk-", "RSX_RISK_HEALTH_ADDR", "orders_processed"),
        "me_to_mkt": me_to_mkt,
    }


# ── Gateway API ─────────────────────────────────────────

# FailureReason wire codes by their spec 18 constant. The gateway's
# /metrics names each reject counter `rejects_<constant, lowercased>`.
FAILURE_REASON_NAMES = {
    0: "INVALID_TICK_SIZE",
    1: "INVALID_LOT_SIZE",
    2: "SYMBOL_NOT_FOUND",
    3: "DUPLICATE_ORDER_ID",
    4: "INSUFFICIENT_MARGIN",
    5: "OVERLOADED",
    6: "INTERNAL_ERROR",
    7: "REDUCE_ONLY_VIOLATION",
    8: "NETWORK_ERROR",
    9: "RATE_LIMIT",
    10: "TIMEOUT",
    11: "USER_IN_LIQUIDATION",
    12: "WRONG_SHARD",
    13: "FILL_OR_KILL",
    14: "ORDER_TOO_SMALL",
}

# `circuit_state` counter values of the gateway's circuit breaker.
CIRCUIT_STATES = {0: "closed", 1: "open", 2: "half-open"}


def _gateway_health_targets() -> list[tuple[str, str]]:
    """(name, health addr) of every gateway in the scenario, from
    the spawn env, so a stopped gateway is still listed."""
    return [
        (proc.name, proc.env["RSX_GW_HEALTH_ADDR"])
        for proc in plan_processes()
        if proc.name.startswith("gw-")
    ]


def _gateway_pending(data: dict) -> tuple[int, int]:
    for queue in data["queues"]:
        if queue["name"] == "pending":
            return int(queue["used"]), int(queue["cap"])
    raise ValueError("no pending queue in /metrics")


def _collect_gateway_metrics() -> list[dict]:
    gateways = []
    for name, addr in _gateway_health_targets():
        entry = {
            "name": name,
            "health_addr": addr,
            "live": False,
            "ready": False,
            "state": "",
            "pending": {"used": 0, "cap": 0},
            "circuit": "",
            "rejects": [],
            "counters": {},
            "error": "",
        }
        try:
            data = _fetch_daemon_metrics(addr)
            counters = {
                c["name"]: int(c["value"])
                for c in data["counters"]
            }
            used, cap = _gateway_pending(data)
            circuit_state = counters["circuit_state"]
            entry.update(
                live=bool(data["live"]),
                ready=bool(data["ready"]),
                state=str(data["state"]),
                pending={"used": used, "cap": cap},
                circuit=CIRCUIT_STATES.get(
                    circuit_state, f"unknown_{circuit_state}"),
                rejects=[
                    {
                        "code": code,
                        "name": reason,
                        "count": counters[f"rejects_{reason.lower()}"],
                    }
                    for code, reason in FAILURE_REASON_NAMES.items()
                ],
                counters=counters,
            )
        except (OSError, ValueError, KeyError, TypeError) as exc:
            entry["error"] = (
                f"{addr}/metrics: {type(exc).__name__}: {exc}")
        gateways.append(entry)
    return gateways


@app.get("/api/gateway/metrics")
async def api_gateway_metrics():
    """Per-gateway counters from each gateway's own /metrics (1s
    cache). A single unreachable gateway carries its reason in its
    entry; none reachable is a 503."""
    await refresh_catalog()
    gateways = _cached_for(
        "gateway_metrics", 1.0, _collect_gateway_metrics)
    if all(gateway["error"] for gateway in gateways):
        return JSONResponse(
            {"error": "no gateway /metrics reachable",
             "gateways": gateways},
            status_code=503)
    return {"gateways": gateways, "ts_ns": time.time_ns()}


@app.get("/x/gateway-counters", response_class=HTMLResponse)
async def x_gateway_counters():
    await refresh_catalog()
    gateways = _cached_for(
        "gateway_metrics", 1.0, _collect_gateway_metrics)
    return HTMLResponse(pages.render_gateway_counters(gateways))


@app.get("/x/gateway-rejects", response_class=HTMLResponse)
async def x_gateway_rejects():
    await refresh_catalog()
    gateways = _cached_for(
        "gateway_metrics", 1.0, _collect_gateway_metrics)
    return HTMLResponse(pages.render_gateway_rejects(gateways))


@app.post("/api/gateway/duplicate-demo")
async def api_gateway_duplicate_demo(request: Request):
    """Send one IOC order twice with the same client order id, over
    the real gateway as `user_id`. The gateway answers the second
    send itself, with the `U` the ME gives a duplicate order id,
    without forwarding it. Non-2xx when the venue is down, the
    gateway does not answer, or the second answer is not that `U`."""
    denied = _require_admin_request(request)
    if denied:
        return denied
    form = await request.form()
    try:
        user_id = int(form.get("user_id", "1"))
        symbol_id = int(form.get("symbol_id", "10"))
    except (ValueError, TypeError):
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            'Failed: invalid user_id or symbol_id</span>',
            status_code=400)
    venue_err = await _execution_venue_error(symbol_id)
    if venue_err:
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">Failed: '
            f'{html.escape(venue_err[1])}</span>',
            status_code=503)
    price = _decimal_field(form.get("price", "0.049"), "0.049")
    qty = _decimal_field(form.get("qty", "30"), "30")
    if price is None or price <= 0 or qty is None or qty <= 0:
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            'Failed: invalid price or qty</span>',
            status_code=400)
    new_order = order_cli.NewOrder(
        user_id=user_id,
        symbol_id=symbol_id,
        side="buy",
        tif="ioc",
        price=format(price, "f"),
        qty=format(qty, "f"),
    )
    answers = []
    cid = None
    for _ in range(2):
        try:
            outcome = await ORDER_GATEWAY.place(
                dataclasses.replace(new_order, cid=cid))
        except rsx_cli.CliError as err:
            return HTMLResponse(
                f'<span class="text-red-400 text-xs">Failed: '
                f'{html.escape(str(err))}</span>',
                status_code=502 if err.code != order_cli.EXIT_MISUSE else 400)
        cid = outcome.cid
        answers.append({
            "frame": json.dumps(outcome.frames[-1]) if outcome.frames else "-",
            "outcome": (
                f"{outcome.status}: {outcome.reason}"
                if outcome.reason else outcome.status),
            "status": outcome.status,
            "note": outcome.reason or "",
        })
    audit_log("/api/gateway/duplicate-demo", f"cid={cid} user={user_id}")
    duplicate = (
        answers[1]["status"] == "rejected"
        and answers[1]["note"] == "DUPLICATE_ORDER_ID")
    body = pages.render_gateway_duplicate(cid, user_id, answers, duplicate)
    return HTMLResponse(body, status_code=200 if duplicate else 502)


# ── Matching engine API ─────────────────────────────────

def _me_health_targets() -> list[tuple[str, int, str]]:
    """(name, symbol_id, health addr) of every ME in the scenario,
    from the spawn env -- the same source the other daemon
    counters use, so a stopped ME is still listed."""
    targets = []
    for proc in plan_processes():
        if proc.name.startswith("me-"):
            targets.append((
                proc.name,
                int(proc.env["RSX_ME_SYMBOL_ID"]),
                proc.env["RSX_ME_HEALTH_ADDR"],
            ))
    return targets


def _collect_me_metrics() -> list[dict]:
    mes = []
    for name, symbol_id, addr in _me_health_targets():
        entry = {
            "name": name,
            "symbol_id": symbol_id,
            "health_addr": addr,
            "live": False,
            "ready": False,
            "state": "",
            "counters": {},
            "error": "",
        }
        try:
            data = _fetch_daemon_metrics(addr)
            counters = {
                c["name"]: int(c["value"])
                for c in data["counters"]
            }
            entry.update(
                live=bool(data["live"]),
                ready=bool(data["ready"]),
                state=str(data["state"]),
                counters=counters,
            )
        except (OSError, ValueError, KeyError, TypeError) as exc:
            entry["error"] = (
                f"{addr}/metrics: {type(exc).__name__}: {exc}")
        mes.append(entry)
    return mes


@app.get("/api/matching/counters")
async def api_matching_counters():
    """Per-ME counters from each ME's own /metrics (1s cache).
    A single unreachable ME carries its reason in its entry; none
    reachable is a 503."""
    await refresh_catalog()
    mes = _cached_for("me_metrics", 1.0, _collect_me_metrics)
    if all(me["error"] for me in mes):
        return JSONResponse(
            {"error": "no matching engine /metrics reachable",
             "mes": mes},
            status_code=503)
    return {"mes": mes, "ts_ns": time.time_ns()}


@app.get("/api/matching/wal")
async def api_matching_wal(
    record_type: str = Query("", alias="type"),
    symbol_id: int | None = Query(None),
    stream: str = Query(""),
    limit: int = Query(100, ge=1, le=1000),
):
    """Decoded records of the ME WAL streams, newest first."""
    types = await wal_feed.list_record_types()
    if record_type and record_type not in types:
        return JSONResponse(
            {"error": f"unknown record type: {record_type}",
             "types": sorted(types)},
            status_code=400)
    streams = {
        listing.name for listing in await wal_listings()
        if listing.name != "mark"}
    if stream and stream not in streams:
        return JSONResponse(
            {"error": f"stream not found: {stream}"},
            status_code=404)
    matched = [
        rec for rec in wal_records.records(record_type or None)
        if rec["stream"] != "mark"
        and (not stream or rec["stream"] == stream)
        and (symbol_id is None or rec.get("symbol_id") == symbol_id)
    ]
    matched.sort(
        key=lambda rec: (rec["ts_ns"], rec["seq"]), reverse=True)
    return {
        "records": matched[:limit],
        "count": min(len(matched), limit),
        "matched": len(matched),
    }


def classify_scheduled(
    applied_version: int, rows: list[dict], now_ms: int,
) -> list[dict]:
    """Tag each schedule row with what the ME's poll does with it:
    it applies a row whose version is above the applied one once
    effective_at_ms has passed."""
    out = []
    for row in rows:
        version = row["config_version"]
        if version == applied_version:
            state = "applied"
        elif version < applied_version:
            state = "superseded"
        elif row["effective_at_ms"] <= now_ms:
            state = "due"
        else:
            state = "pending"
        out.append({**row, "state": state})
    return out


ME_COLUMNS = (
    "config_version", "effective_at_ms", "tick_size", "lot_size",
    "price_decimals", "qty_decimals", "status",
)


def _me_row(
    config: risk_cli.SymbolConfig,
    row: risk_cli.ConfigColumns,
    extra: str,
) -> dict:
    return {
        "symbol_id": config.symbol_id,
        "symbol_name": config.name,
        **{column: getattr(row, column) for column in (*ME_COLUMNS, extra)},
    }


@app.get("/api/matching/config")
async def api_matching_config():
    """Per symbol: the config row the ME applied
    (symbol_config_applied) and the scheduled rows
    (symbol_config_schedule) with their state. Risk owns the
    schema. Matching columns only; margin and fee columns stay on
    the admin Config tab."""
    configs = await read_config()
    now_ms = int(time.time() * 1000)
    out = []
    for config in sorted(configs, key=lambda config: config.symbol_id):
        applied = config.applied
        scheduled = [
            _me_row(config, row, "created_at_ms")
            for row in config.scheduled]
        out.append({
            "symbol_id": config.symbol_id,
            "symbol_name": config.name,
            "applied": (
                None if applied is None
                else _me_row(config, applied, "applied_at_ns")),
            "scheduled": classify_scheduled(
                applied.config_version if applied else 0,
                scheduled, now_ms),
        })
    return {"symbols": out, "now_ms": now_ms}


@app.get("/x/matching-counters", response_class=HTMLResponse)
async def x_matching_counters():
    await refresh_catalog()
    mes = _cached_for("me_metrics", 1.0, _collect_me_metrics)
    return HTMLResponse(pages.render_matching_counters(mes))


@app.get("/x/matching-recovery", response_class=HTMLResponse)
async def x_matching_recovery():
    await refresh_catalog()
    mes = _cached_for("me_metrics", 1.0, _collect_me_metrics)
    return HTMLResponse(pages.render_matching_recovery(mes))


MATCHING_FEED_LIMIT = 15


@app.get("/x/matching-rejects", response_class=HTMLResponse)
async def x_matching_rejects():
    """The engine-made rejects and cancels, newest first; a user's own
    cancel is not listed, a quoting maker would bury everything else."""
    rejects = [
        rec for rec in wal_records.records("order_failed")
        + wal_records.records("order_cancelled")
        if rec["stream"] != "mark"
        and (rec["type"] == "order_failed" or rec["reason"] != 0)]
    newest = sorted(
        rejects, key=lambda rec: (rec["ts_ns"], rec["seq"]),
        reverse=True)
    return HTMLResponse(
        pages.render_matching_rejects(newest[:MATCHING_FEED_LIMIT]))


CRASH_RECOVERY_TIMEOUT_S = 30.0


def _me_counters(addr: str) -> dict[str, int]:
    data = _fetch_daemon_metrics(addr)
    if not data["ready"]:
        raise ValueError("not ready")
    return {c["name"]: int(c["value"]) for c in data["counters"]}


@app.post("/api/matching/crash/{name}")
async def api_matching_crash(request: Request, name: str):
    """SIGKILL one ME, wait for the watcher to bring a new process up
    ready, and return its WAL tip, checkpoint and config version from
    before the crash and after the recovery."""
    denied = _require_admin_request(request)
    if denied:
        return denied
    await refresh_catalog()
    target = next(
        (t for t in _me_health_targets() if t[0] == name), None)
    if target is None:
        return JSONResponse(
            {"error": f"unknown matching engine: {name}"},
            status_code=404)
    addr = target[2]
    old_pid = await _running_pid(name)
    if old_pid is None:
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'{name} not running</span>')
    try:
        before = _me_counters(addr)
    except (OSError, ValueError, KeyError) as exc:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">Failed: {name} '
            f'/metrics before the crash: '
            f'{html.escape(str(exc))}</span>')
    audit_log(f"/api/matching/crash/{name}",
              "crash sim (SIGKILL, watcher will restart)")
    started = time.monotonic()
    try:
        await fault_cli.kill(name)
    except rsx_cli.CliError as err:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">Failed: '
            f'{html.escape(str(err))}</span>')
    while time.monotonic() - started < CRASH_RECOVERY_TIMEOUT_S:
        await asyncio.sleep(0.25)
        forget_procs()
        new_pid = await _running_pid(name)
        if new_pid is None or new_pid == old_pid:
            continue
        try:
            after = _me_counters(addr)
        except (OSError, ValueError, KeyError):
            continue
        if after.get("wal_flushed_seq", 0) == 0:
            continue
        return HTMLResponse(pages.render_matching_crash(
            name, before, after, time.monotonic() - started))
    return HTMLResponse(
        f'<span class="text-red-400 text-xs">Failed: {name} did not '
        f'come back within {CRASH_RECOVERY_TIMEOUT_S:.0f}s (is the '
        f'playground watcher running?)</span>')


duplicate_demo_lock = asyncio.Lock()


@app.post("/api/matching/duplicate-demo")
async def api_matching_duplicate_demo(request: Request):
    """Cast one order twice to a scratch ME and return its WAL records
    for that order id, ending in ORDER_FAILED."""
    denied = _require_admin_request(request)
    if denied:
        return denied
    if duplicate_demo_lock.locked():
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">'
            'a duplicate demo is already running</span>')
    audit_log("/api/matching/duplicate-demo", "scratch ME, one order twice")
    async with duplicate_demo_lock:
        try:
            demo = await fault_cli.dup_order()
        except rsx_cli.CliError as err:
            return HTMLResponse(
                f'<div class="text-red-400 text-xs">Failed: '
                f'<pre class="whitespace-pre-wrap">'
                f'{html.escape(str(err))}</pre></div>')
    records = [
        wal_records.to_record(line, "me-demo") for line in demo.records]
    return HTMLResponse(
        pages.render_matching_duplicate(demo.order_id, records))


@app.get("/x/control-grid", response_class=HTMLResponse)
async def x_control_grid():
    # Share the 1s procs cache with every other CPU-bearing surface
    # (process table, resource usage) so CPU% reads identically across
    # Control and Overview instead of resampling a stateful
    # cpu_percent() per endpoint (#31).
    return HTMLResponse(
        pages.render_control_grid(await scan_processes()))


@app.get("/x/resource-usage", response_class=HTMLResponse)
async def x_resource_usage():
    return HTMLResponse(
        pages.render_resource_usage(await scan_processes()))


@app.get("/x/faults-grid", response_class=HTMLResponse)
async def x_faults_grid():
    return HTMLResponse(
        pages.render_faults_grid(await scan_processes()))


@app.get("/x/recovery-controls", response_class=HTMLResponse)
async def x_recovery_controls():
    return HTMLResponse(
        pages.render_recovery_controls(await scan_processes()))


@app.get("/x/recovery-feed", response_class=HTMLResponse)
async def x_recovery_feed():
    return HTMLResponse(
        pages.render_recovery_feed(await recovery_feed_events()))


@app.get("/x/wal-status", response_class=HTMLResponse)
async def x_wal_status():
    return HTMLResponse(
        pages.render_wal_status(await list_wal_streams()))


@app.get("/x/wal-detail", response_class=HTMLResponse)
async def x_wal_detail():
    return HTMLResponse(
        pages.render_wal_detail(await list_wal_streams()))


@app.get("/x/wal-files", response_class=HTMLResponse)
async def x_wal_files():
    return HTMLResponse(
        pages.render_wal_files(await list_wal_files()))


@app.get("/x/wal-lag", response_class=HTMLResponse)
async def x_wal_lag():
    streams = await list_wal_streams()
    return HTMLResponse(pages.render_wal_lag(streams))


@app.get("/x/wal-rotation", response_class=HTMLResponse)
async def x_wal_rotation():
    streams = await list_wal_streams()
    return HTMLResponse(pages.render_wal_rotation(streams))


@app.get("/x/wal-timeline", response_class=HTMLResponse)
async def x_wal_timeline(
    filter: str = Query("", alias="filter"),
):
    all_records = wal_records.newest(50, filter.lower() or None)
    return HTMLResponse(
        pages.render_wal_timeline(all_records))


@app.get("/x/wal-followers", response_class=HTMLResponse)
async def x_wal_followers():
    return HTMLResponse(
        pages.render_wal_followers(
            wal_records.follower_rows(), wal_records.refresh_error))


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
    stats = wal_records.book_stats()
    # supplement with live snaps for symbols not in WAL
    snap_copy = dict(_book_snap)
    for sid, snap in snap_copy.items():
        if sid not in stats:
            bbo = _snap_to_bbo(sid, snap)
            if bbo:
                stats[sid] = bbo
    # fallback: maker book or re-seed for configured symbols
    await refresh_symbols()
    for symbol in symbols.table.values():
        sid = symbol.id
        if sid in stats:
            continue
        mb = await _maker_book(sid)
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
    procs = await scan_processes()
    live = {sid for sid in stats if _me_live(sid, procs)}
    return HTMLResponse(
        pages.render_book_stats(stats, live_symbols=live))


@app.get("/x/live-fills", response_class=HTMLResponse)
async def x_fills():
    fills = wal_records.fills()
    if not fills:
        fills = list(reversed(recent_fills[-50:]))
    return HTMLResponse(
        pages.render_live_fills(
            fills, maker_running=await _maker_running()))


@app.get("/x/trade-agg", response_class=HTMLResponse)
async def x_trade_agg():
    procs = await scan_processes()
    running = [p for p in procs
               if p["state"] == "running"]
    if not running:
        return HTMLResponse(
            '<span class="text-slate-500 text-xs">'
            'no processes running</span>')
    fills = wal_records.fills()
    return HTMLResponse(
        pages.render_trade_agg(fills))


@app.get("/x/position-heatmap",
         response_class=HTMLResponse)
async def x_position_heatmap():
    data = await api_risk_overview()
    return HTMLResponse(pages.render_position_heatmap(data["users"]))


@app.get("/x/margin-ladder", response_class=HTMLResponse)
async def x_margin_ladder():
    data = await api_risk_overview()
    return HTMLResponse(pages.render_margin_ladder(data["users"]))


@app.get("/x/funding", response_class=HTMLResponse)
async def x_funding():
    stats = wal_records.book_stats()
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
    running = stress_run.is_running()
    window_s = 30
    now = time.time()
    cutoff = now - window_s
    ord_s = int(
        sum(1 for ts in recent_order_ts if ts >= cutoff) / window_s)
    submitted = 0
    accepted = 0
    if running:
        submitted = len([
            order for order in recent_orders
            if order.get("status") not in {None}])
        accepted = len([
            order for order in recent_orders
            if order.get("status") == "accepted"])
    return HTMLResponse(
        pages.render_load_overview(
            running, current_scenario, ord_s, accepted, submitted))


MD_SNAPSHOT_TIMEOUT_MS = 1000
MD_SNAPSHOT_CACHE_S = 2.0


async def _md_snapshot_bbos() -> tuple[dict[int, dict], str | None]:
    """The best bid and ask of each catalogued symbol with a book, from
    `rsx-cli md snapshot`; the error of the first snapshot that failed."""
    await refresh_symbols()
    ids = sorted(symbol.id for symbol in symbols.table.values())
    snapshots = await asyncio.gather(
        *(rsx_cli.run(
            "md", "snapshot", str(symbol_id), "-u", MARKETDATA_WS,
            "-t", str(MD_SNAPSHOT_TIMEOUT_MS), "--json")
          for symbol_id in ids),
        return_exceptions=True)
    bbos: dict[int, dict] = {}
    error = None
    for symbol_id, snapshot in zip(ids, snapshots):
        if isinstance(snapshot, BaseException):
            error = error or str(snapshot)
        elif snapshot["bbo"] is not None:
            bbos[symbol_id] = snapshot["bbo"]
    return bbos, error


@app.get("/x/reconciliation",
         response_class=HTMLResponse)
async def x_reconciliation():
    bbos, error = await _cached_for_async(
        "md_snapshot_bbos", MD_SNAPSHOT_CACHE_S, _md_snapshot_bbos)

    # Marketdata's book against the matching engine's BBO records in
    # the WAL.
    shadow_check = None
    if error is not None and not bbos:
        shadow_check = ("skip", html.escape(error))
    else:
        mismatches = 0
        checked = 0
        for sid, bbo in bbos.items():
            wal_bbo = wal_records.bbo(sid)
            if wal_bbo is None:
                continue
            checked += 1
            if (bbo["bid_px"] != wal_bbo.get("bid_px", 0)
                    or bbo["ask_px"] != wal_bbo.get("ask_px", 0)):
                mismatches += 1
        if checked > 0:
            if mismatches == 0:
                shadow_check = (
                    "pass", f"{checked} symbols match the WAL")
            else:
                shadow_check = (
                    "fail", f"{mismatches}/{checked} mismatch")

    # Compare book mid against the real index from the mark process
    # (RECORD_MARK_PRICE, external Binance/Coinbase aggregate). SKIP
    # when no index is loaded — a check that always passes is worse
    # than no check. Tolerance band: 1% premium of perp over index.
    mark_check = None
    mark_prices = wal_records.mark_prices()
    if bbos and mark_prices:
        checked = 0
        mismatches = 0
        for sid, bbo in bbos.items():
            mark_rec = mark_prices.get(sid)
            if mark_rec is None:
                continue
            index_px = mark_rec.get("mark_price", 0)
            if index_px <= 0:
                continue
            bid = bbo["bid_px"]
            ask = bbo["ask_px"]
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
        (order for order in recent_orders if order.get("cid") == oid),
        None,
    )
    if order is None:
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'order {html.escape(oid)} not found in session'
            f'</span>')
    user_id = order.get("user_id", 0)
    symbol_id = int(order.get("symbol", "10"))
    fills = wal_records.fills_for_user(user_id, symbol_id)
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
        "filled", "partial", "cancelled", "rejected", "failed",
        "expired", "resting", "accepted", "done",
    }
    stale = [
        order for order in recent_orders
        if order.get("status", "") not in resolved
        and (age := _order_age_s(order.get("ts"), now)) is not None
        and age > 60]
    if not stale:
        return HTMLResponse(
            '<span class="text-emerald-400 text-xs">'
            'no hung orders</span>')
    return HTMLResponse(
        f'<span class="text-amber-400 text-xs">'
        f'{len(stale)} hung order(s) '
        f'(&gt;60s, no terminal state)</span>')


def _me_live(symbol_id: int, procs: list[dict]) -> bool:
    """True when the matching engine for this symbol is running.

    The book ladder renders the last snapshot; if the ME is down
    that ladder is stale (no longer backed by a live book), so the
    caller badges it as such instead of showing phantom liquidity.
    """
    symbol = symbols.by_id(symbol_id)
    if symbol is None:
        return True  # unknown symbol: don't assert staleness
    return find_running(procs, f"me-{symbol.name.lower()}") is not None


@app.get("/x/book", response_class=HTMLResponse)
async def x_book(symbol_id: int = Query(10)):
    await refresh_symbols()
    stale = not _me_live(symbol_id, await scan_processes())
    snap = _book_snap.get(symbol_id)
    if snap and (snap.get("bids") or snap.get("asks")):
        return HTMLResponse(
            pages.render_book_ladder(symbol_id, snap,
                                     source="live", stale=stale))
    # Fallback: WAL BBO gives at most 1 bid + 1 ask
    bbo = wal_records.bbo(symbol_id)
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
    maker_snap = await _maker_book(symbol_id)
    if maker_snap:
        return HTMLResponse(
            pages.render_book_ladder(symbol_id, maker_snap,
                                     source="synthetic", stale=stale))
    return HTMLResponse(
        pages.render_book_ladder(symbol_id, None,
                                 maker_running=await _maker_running()))


@app.get("/x/risk-user", response_class=HTMLResponse)
async def x_risk_user(
    risk_uid: int = Query(1, alias="risk-uid"),
    user_id: int | None = Query(None),
):
    # Accept either ?risk-uid= (the form field) or the more natural
    # ?user_id= (both name the same user; user_id wins if both given).
    uid = user_id if user_id is not None else risk_uid
    try:
        account = await risk_cli.account_show(PG_URL, uid, bbo_marks())
    except risk_cli.NoAccount:
        return HTMLResponse(
            '<span class="text-slate-500 text-xs">'
            f'user {uid} has no account</span>')
    return HTMLResponse(pages.render_risk_user(account))


@app.get("/x/liquidations", response_class=HTMLResponse)
async def x_liquidations():
    data = (await read_risk()).liquidations[:20]
    if data:
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
    liqns = wal_records.liquidations()
    if liqns:
        return HTMLResponse(
            pages.render_liquidations_wal(liqns))
    return HTMLResponse(
        '<span class="text-slate-600">'
        'no active liquidations</span>')


@app.get("/x/verify", response_class=HTMLResponse)
async def x_verify():
    procs = await scan_processes()
    running, expected = process_counts(procs)
    report = verifier.report
    if report is None:
        return HTMLResponse(
            '<span class="text-slate-600">'
            f'processes running {running}/{expected}; '
            'click "Run All Checks" to verify</span>')
    return HTMLResponse(
        '<div class="text-[10px] text-slate-500 mb-2">'
        f'processes running {running}/{expected}</div>'
        + pages.render_verify(report))


@app.get("/x/recent-orders", response_class=HTMLResponse)
async def x_recent_orders():
    return HTMLResponse(
        pages.render_recent_orders(recent_orders[-50:]))


# ── API routes ──────────────────────────────────────────

def _status_span(color: str, text: str) -> HTMLResponse:
    return HTMLResponse(
        f'<span class="text-{color}-400 text-xs">'
        f'{html.escape(text)}</span>')


@app.get("/api/processes")
async def api_processes():
    return await scan_processes()


@app.get("/api/scenarios")
async def api_scenarios():
    return [scenario.name for scenario in await get_scenarios()]


@app.post("/api/processes/all/start")
async def api_start_all(
    request: Request,
    scenario: str = Query(None),
):
    """Seed the accounts, then build and start the scenario's processes.

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
        return _status_span("red", result["error"])
    text = f'started {result["count"]} processes'
    if result["skipped"]:
        text += f' ({result["skipped"]} already running)'
    return _status_span("emerald", text)


@app.post("/api/processes/all/stop")
async def api_stop_all(request: Request):
    """Stop the cluster's processes and the stress run."""
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
    return _status_span(
        "amber", f'stopped {len(result["stopped"])} processes')


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
    try:
        async with bulk_operation():
            report = await cluster_cli.reset()
    except rsx_cli.CliError as err:
        forget_procs()
        return _status_span("red", f"Failed: {err}")
    stopped = list(report.stopped)
    if stress_run.is_running():
        await stress_run.stop()
        stopped.append(STRESS_NAME)
    cleared = list(report.cleared)
    await wal_records.reset()
    if PID_DIR.exists():
        for pid_file in PID_DIR.glob("*.pid"):
            if pid_file.stem != SELF_PID_NAME:
                pid_file.unlink(missing_ok=True)
        cleared.append("pids")
    forget_procs()
    return HTMLResponse(
        f'<span class="text-emerald-400 text-xs">'
        f'reset done &mdash; stopped {len(stopped)} '
        f'processes, cleared {", ".join(cleared)}. Start the '
        f'cluster again from Control.</span>')


async def _action_stop(name: str) -> HTMLResponse:
    _restart_entry(name)["intentional"] = True
    try:
        report = await cluster_cli.down((name,))
    except rsx_cli.CliError as err:
        return _status_span("red", f"Failed: {err}")
    forget_procs()
    if name in report.stopped:
        return _status_span("amber", f"stopped {name}")
    return _status_span("amber", f"{name} not running")


async def _action_kill(name: str) -> HTMLResponse:
    pid = await _running_pid(name)
    if pid is None:
        return _status_span("red", f"{name} not running")
    _restart_entry(name)["intentional"] = True
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        return _status_span("red", f"{name} not running")
    except OSError as err:
        return _status_span("red", f"kill failed: {err}")
    await _wait_for_pid_exit(pid, timeout=2.0)
    try:
        await cluster_cli.down((name,))
    except rsx_cli.CliError as err:
        return _status_span("red", f"Failed: {err}")
    forget_procs()
    return _status_span("red", f"killed {name}")


async def _action_restart(name: str) -> HTMLResponse:
    entry = _restart_entry(name)
    entry.update(
        restarts=0, blocked=False, next_restart_at=0.0, restarting=True)
    try:
        await cluster_cli.restart(name, current_scenario)
    except rsx_cli.CliError as err:
        if err.code == cluster_cli.EXIT_MISUSE:
            return _status_span("blue", f"unknown process: {name}")
        return _status_span("red", f"Failed: {err}")
    finally:
        entry["restarting"] = False
        forget_procs()
    return _status_span(
        "blue", f"restarted {name} (pid {await _running_pid(name)})")


async def _action_start(name: str) -> HTMLResponse:
    if await _running_pid(name) is not None:
        return _status_span("amber", f"{name} already running")
    try:
        await cluster_cli.up(current_scenario, only=(name,), build=True)
    except rsx_cli.CliError as err:
        if err.code == cluster_cli.EXIT_MISUSE:
            return _status_span("red", f"unknown process: {name}")
        return _status_span("red", f"Failed: {err}")
    finally:
        forget_procs()
    return _status_span(
        "emerald", f"started {name} (pid {await _running_pid(name)})")


@app.post("/api/processes/{name}/{action}")
async def api_process_action(
    request: Request,
    name: str,
    action: str,
):
    denied = _require_admin_request(request)
    if denied:
        return denied
    if action == "stop":
        return await _action_stop(name)
    if action == "kill":
        return await _action_kill(name)
    if action == "restart":
        return await _action_restart(name)
    if action == "start":
        return await _action_start(name)
    return JSONResponse(
        {"error": f"unknown action: {action}"},
        status_code=400)


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
    pid = await _running_pid(name)
    if pid is None:
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'{name} not running</span>')
    try:
        killed = await fault_cli.kill(name)
    except rsx_cli.CliError as err:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'Kill failed: {html.escape(str(err))}</span>')
    return HTMLResponse(
        f'<span class="text-red-400 text-xs">'
        f'crashed {name} (pid {killed.pid}) &mdash; watch it '
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
    try:
        fault = await fault_cli.net(action == "clear")
    except rsx_cli.CliError as err:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'Failed: {html.escape(strip_ansi(str(err)).strip()[:200])}'
            f'</span>')
    if not fault.is_active:
        return HTMLResponse(
            f'<span class="text-emerald-400 text-xs">'
            f'{fault.iface} netem cleared &mdash; links healing, watch '
            f'catch-up</span>')
    return HTMLResponse(
        f'<span class="text-amber-400 text-xs">'
        f'{fault.iface}: +{fault.delay_ms}ms delay, '
        f'{fault.loss_pct:g}% loss &mdash; watch casting '
        f'FAULTED &rarr; NAK/replay</span>')


@app.post("/api/fault/wal-corrupt")
async def api_fault_wal_corrupt(request: Request):
    """Flip a payload byte in the newest segment of the live stream
    written last, so its CRC32C fails on replay (mutates WAL on disk).
    The recorder archive is never a target. Guarded by the UI's
    hx-confirm click-through, not an env var."""
    denied = _require_admin_request(request)
    if denied:
        return denied
    audit_log("/api/fault/wal-corrupt", "flip a WAL payload byte")
    try:
        listings = await wal_feed.list_streams(WAL_DIR)
    except rsx_cli.CliError as err:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'Failed: {html.escape(str(err))}</span>')
    streams = [
        stream for listing in listings for stream in listing.streams
        if stream.segments]
    if not streams:
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">'
            'no WAL files found &mdash; start the cluster '
            'first</span>')
    target = max(streams, key=lambda stream: max(
        segment.modified_ms for segment in stream.segments))
    try:
        flip = await fault_cli.wal_corrupt(target.stream_id, target.wal_dir)
    except rsx_cli.CliError as err:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'Failed: {html.escape(str(err))}</span>')
    rel = flip.file.relative_to(WAL_DIR)
    return HTMLResponse(
        f'<span class="text-red-400 text-xs">'
        f'flipped payload byte @ offset {flip.offset} in {rel} '
        f'&mdash; CRC32C fails on replay</span>')


@app.post("/api/scenario/switch")
async def api_scenario_switch(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
    denied = check_confirm(request, "/api/scenario/switch")
    if denied:
        return denied
    form = await request.form()
    scenario = str(form.get("scenario-select", "minimal"))
    audit_log("/api/scenario/switch", f"scenario={scenario}")

    if scenario not in [s.name for s in await get_scenarios()]:
        return _status_span("red", f"unknown scenario: {scenario}")

    async with bulk_operation():
        await stop_all()
        result = await start_all(scenario)
    if "error" in result:
        return _status_span(
            "amber",
            f'switched to {scenario} (restart failed: {result["error"]})')
    return _status_span(
        "emerald",
        f'switched to {scenario} and restarted {result["count"]} processes')


@app.get("/x/current-scenario", response_class=HTMLResponse)
async def x_current_scenario():
    await scan_processes()
    return HTMLResponse(current_scenario)


@app.get("/api/wal/{stream}/status")
async def api_wal_status(stream: str):
    row = next(
        (row for row in await list_wal_streams()
         if row["name"] == stream), None)
    if row is None:
        return {"error": "stream not found"}
    return {
        "stream": stream,
        "files": row["files"],
        "total_bytes": row["total_bytes"],
        "total_size": row["total_size"],
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
    procs = await scan_processes()
    running = sum(1 for p in procs if p["state"] == "running")
    maker_running = find_running(procs, MAKER_NAME) is not None
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
        ("active stress", int(stress_run.is_running())),
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


def _trim_recent_orders():
    # Record the submission timestamp (F26 sliding window) and
    # cap the list. recent_order_ts is independent from
    # recent_orders so the window survives recent_orders pruning.
    recent_order_ts.append(time.time())
    if len(recent_order_ts) > 2000:
        del recent_order_ts[:1000]
    if len(recent_orders) > 200:
        del recent_orders[:100]


# FailureReason discriminants (rsx-types/src/lib.rs, specs/2/18-messages.md).
# The gateway U-frame and the ME's ORDER_FAILED record carry the raw u8;
# map it to a human string so the UI never surfaces a bare "reason=4".
# Notional overflow at the risk boundary is reported as InsufficientMargin
# (4).
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
    13: "fill-or-kill not fillable",
    14: "order below the minimum size",
}

wal_records = wal_feed.WalFeed(REJECT_REASONS)
WAL_FEED_POLL_S = 2.0


async def _execution_venue_error(
    symbol_id: int,
) -> tuple[str, str] | None:
    """None if gateway + the matching engine for `symbol_id` are both
    running; else (order.error field text, HTML detail text). A
    stopped gateway/ME is a normal dev state (the safety suite kills
    them on purpose), not a server error -- callers return 200 with a
    descriptive body so the page keeps working.
    """
    await refresh_symbols()
    symbol = symbols.by_id(symbol_id)
    running_names = {
        p["name"] for p in await scan_processes()
        if p.get("state") == "running"
    }
    if "gw-0" not in running_names:
        return "gateway not running", "gateway (gw-0) not available"
    if symbol and f"me-{symbol.name.lower()}" not in running_names:
        detail = f"matching engine for {symbol.name} not available"
        return detail, detail
    return None


_MARKET_SWEEP_PCT = 50.0
_SIDES = ("buy", "sell")
_TIFS = ("GTC", "IOC", "FOK")


def _symbol_tick(symbol_id: int) -> int:
    symbol = symbols.by_id(symbol_id)
    return symbol.tick if symbol and symbol.tick else 1


def _book_mid(symbol_id: int) -> int | None:
    snap = _book_snap.get(symbol_id)
    if snap:
        bids = snap.get("bids", [])
        asks = snap.get("asks", [])
        if bids and asks:
            return (bids[0]["px"] + asks[0]["px"]) // 2
        if bids:
            return bids[0]["px"]
        if asks:
            return asks[0]["px"]
    bbo = wal_records.bbo(symbol_id)
    if bbo and bbo.get("bid_px") and bbo.get("ask_px"):
        return (bbo["bid_px"] + bbo["ask_px"]) // 2
    if bbo and bbo.get("bid_px"):
        return bbo["bid_px"]
    if bbo and bbo.get("ask_px"):
        return bbo["ask_px"]
    return None


def _px_from_mid(symbol_id: int, mid: int, offset_pct: float) -> int:
    tick = _symbol_tick(symbol_id)
    return max(tick, round(mid * (1.0 + offset_pct / 100.0) / tick) * tick)


def _sweep_px(symbol_id: int, side: str) -> int:
    mid = _book_mid(symbol_id)
    if mid is None:
        return _symbol_tick(symbol_id)
    sweep = _MARKET_SWEEP_PCT if side == "buy" else -_MARKET_SWEEP_PCT
    return _px_from_mid(symbol_id, mid, sweep)


def _decimal_field(value, default: str) -> Decimal | None:
    try:
        parsed = Decimal(str(value or default).strip())
    except InvalidOperation:
        return None
    return parsed if parsed.is_finite() else None


def _order_from_form(
    user_id: int, symbol_id: int, form
) -> tuple[order_cli.NewOrder, dict] | str:
    side = str(form.get("side", "buy")).lower()
    if side not in _SIDES:
        return "invalid side"
    order_type = form.get("order_type", "limit")
    tif = str(form.get("tif", "GTC")).upper()
    if tif not in _TIFS:
        tif = "GTC"
    price = _decimal_field(form.get("price", "0"), "0")
    if price is None or price < 0:
        return "invalid price"
    qty = _decimal_field(form.get("qty", "0"), "0")
    if qty is None:
        return "invalid qty"
    if qty <= 0:
        return "qty must be positive"
    is_market = order_type == "market" or price == 0
    if is_market and tif == "GTC":
        tif = "IOC"
    is_reduce_only = form.get("reduce_only") == "on"
    is_post_only = form.get("post_only") == "on" or order_type == "post_only"
    new_order = order_cli.NewOrder(
        user_id=user_id,
        symbol_id=symbol_id,
        side=side,
        tif=tif,
        px=_sweep_px(symbol_id, side) if is_market else None,
        price=None if is_market else format(price, "f"),
        qty=format(qty, "f"),
        reduce_only=is_reduce_only,
        post_only=is_post_only,
    )
    meta = {
        "user_id": user_id,
        "symbol": str(symbol_id),
        "side": side,
        "price": "mkt" if is_market else format(price, "f"),
        "qty": format(qty, "f"),
        "tif": tif,
        "reduce_only": is_reduce_only,
        "post_only": is_post_only,
    }
    return new_order, meta


def _record_order(order: dict) -> None:
    recent_orders.append(order)
    _trim_recent_orders()


def _timestamp() -> str:
    return datetime.now().strftime("%H:%M:%S")


def _store_outcome(order: dict, outcome: order_cli.Outcome) -> None:
    order["cid"] = outcome.cid or "-"
    order["status"] = outcome.status
    order["order_id"] = outcome.order_id
    if outcome.reason:
        order["reason"] = outcome.reason
    if outcome.rtt_us:
        order["latency_us"] = outcome.rtt_us
        order_latencies.append(outcome.rtt_us)
        if len(order_latencies) > 1000:
            del order_latencies[:500]
    _record_order(order)


def _store_failure(order: dict, err: rsx_cli.CliError) -> None:
    order["cid"] = "-"
    order["status"] = "error"
    order["error"] = str(err)
    _record_order(order)


def _record_outcome(order: dict, outcome: order_cli.Outcome) -> HTMLResponse:
    _store_outcome(order, outcome)
    cid = order["cid"]
    reason = f": {html.escape(outcome.reason)}" if outcome.reason else ""
    if outcome.status in ("filled", "partial", "resting"):
        latency = f" ({outcome.rtt_us}us)" if outcome.rtt_us is not None else ""
        return HTMLResponse(
            f'<span class="text-emerald-400 text-xs">'
            f'order {cid} {outcome.status}{latency}</span>')
    if outcome.status == "timeout":
        seconds = order_cli.ORDER_TIMEOUT_MS // 1000
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'order {cid} timeout: no response in {seconds}s '
            f'(the order may still rest)</span>',
            status_code=504)
    color = "text-red-400" if outcome.status == "rejected" else "text-amber-400"
    return HTMLResponse(
        f'<span class="{color} text-xs">'
        f'order {cid} {outcome.status}{reason}</span>')


def _record_cli_failure(order: dict, err: rsx_cli.CliError) -> HTMLResponse:
    if err.code == order_cli.EXIT_MISUSE:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'{html.escape(str(err))}</span>')
    _store_failure(order, err)
    return HTMLResponse(
        f'<span class="text-amber-400 text-xs">'
        f'order error: {html.escape(str(err))}</span>')


@app.post("/api/orders/test")
async def api_orders_test(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
    idem_key = request.headers.get("x-idempotency-key")
    if not _check_idempotency(idem_key):
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">'
            'duplicate submission (idempotency key '
            'already used)</span>')
    form = await request.form()
    audit_log("/api/orders/test", "submit order")

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

    venue_err = await _execution_venue_error(symbol_id)
    if venue_err:
        order_error, html_detail = venue_err
        _record_order({
            "cid": "-",
            "user_id": user_id,
            "symbol": str(symbol_id),
            "side": form.get("side", "buy"),
            "price": str(form.get("price", "0") or "0"),
            "qty": str(form.get("qty", "0") or "0"),
            "tif": form.get("tif", "GTC"),
            "reduce_only": form.get("reduce_only") == "on",
            "post_only": form.get("post_only") == "on",
            "status": "error",
            "error": order_error,
            "ts": _timestamp(),
        })
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'order error: {html_detail}</span>')

    built = _order_from_form(user_id, symbol_id, form)
    if isinstance(built, str):
        return HTMLResponse(f'<span class="text-red-400 text-xs">{built}</span>')
    new_order, meta = built
    order = {**meta, "status": "pending", "ts": _timestamp()}
    try:
        outcome = await ORDER_GATEWAY.place(new_order)
    except rsx_cli.CliError as err:
        return _record_cli_failure(order, err)
    return _record_outcome(order, outcome)


async def run_verify_now() -> invariants.Report:
    """`rsx-cli verify` over the WAL directories and the playground's Postgres."""
    listings = await wal_listings()
    wal_dirs = sorted({s.wal_dir for lst in listings for s in lst.streams})
    return await verifier.run(wal_dirs or [WAL_DIR], PG_URL)


@app.post("/api/verify/run")
async def api_verify_run():
    report = await run_verify_now()
    return HTMLResponse(pages.render_verify(report))


@app.post("/api/verify/run-json")
async def api_verify_run_json():
    report = await run_verify_now()
    if report.error:
        return JSONResponse({"error": report.error}, status_code=502)
    failed = report.failed_checks()
    return JSONResponse({
        "status": report.status,
        "checks": [asdict(check) for check in report.checks],
        "ready": not failed,
        "failed": len(failed),
        "run_at_ms": report.run_at_ms,
    })


async def _cancel_resting_order(order: dict):
    cid = order["cid"]
    audit_log("/api/orders/{cid}/cancel",
              f"cancel {cid} (user {order['user_id']})")
    try:
        outcome = await ORDER_GATEWAY.cancel(order["user_id"], cid)
    except rsx_cli.CliError as err:
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'{cid} cancel error: {html.escape(str(err))}</span>')
    if outcome.status == "cancelled":
        order["status"] = "cancelled"
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'{cid} cancelled</span>')
    if outcome.status == "timeout":
        seconds = order_cli.ORDER_TIMEOUT_MS // 1000
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'{cid} cancel timeout: no response in {seconds}s (the order '
            f'may already be filled)</span>')
    detail = f": {html.escape(outcome.reason)}" if outcome.reason else ""
    return HTMLResponse(
        f'<span class="text-red-400 text-xs">'
        f'{cid} not cancelled ({outcome.status}{detail})</span>')


@app.post("/api/orders/{cid}/cancel")
async def api_orders_cancel(request: Request, cid: str):
    denied = _require_admin_request(request)
    if denied:
        return denied
    for order in recent_orders:
        if order["cid"] == cid and order["status"] == "resting":
            return await _cancel_resting_order(order)
    return HTMLResponse(
        f'<span class="text-red-400 text-xs">'
        f'{html.escape(cid)} not found or not cancellable</span>')


_BATCH_MAX = 100
BATCH_SYMBOL_ID = 10
BATCH_USER_ID = 1
BATCH_STEP_PCT = 0.5
RANDOM_ORDERS = 5
RANDOM_SPREAD_PCT = 1.0


def _batch_row(new_order: order_cli.NewOrder, outcome: order_cli.Outcome) -> dict:
    symbol_id = new_order.symbol_id
    qty = "min"
    if new_order.qty_raw is not None:
        qty = pages.format_qty(new_order.qty_raw, symbol_id)
    return {
        "user_id": new_order.user_id,
        "symbol": str(symbol_id),
        "side": new_order.side,
        "price": pages.format_price(new_order.px, symbol_id),
        "qty": qty,
        "tif": new_order.tif.upper(),
        "reduce_only": new_order.reduce_only,
        "post_only": new_order.post_only,
        "ts": _timestamp(),
    }


async def _send_batch(batch: list[order_cli.NewOrder]) -> list[order_cli.Outcome]:
    outcomes = await ORDER_GATEWAY.batch(batch)
    for new_order, outcome in zip(batch, outcomes, strict=True):
        _store_outcome(_batch_row(new_order, outcome), outcome)
    return outcomes


def _batch_response(
    label: str, outcomes: list[order_cli.Outcome], note: str = ""
) -> HTMLResponse:
    counts = Counter(outcome.status for outcome in outcomes)
    summary = ", ".join(f"{count} {status}" for status, count in counts.items())
    failed = [
        outcome for outcome in outcomes
        if outcome.status in ("rejected", "timeout")]
    if not failed:
        return HTMLResponse(
            f'<span class="text-emerald-400 text-xs">'
            f'{len(outcomes)} {label} submitted{note}: {summary}</span>')
    reasons = ", ".join(sorted({
        outcome.reason or outcome.status for outcome in failed}))
    return HTMLResponse(
        f'<span class="text-amber-400 text-xs">'
        f'{len(outcomes)} {label} submitted{note}: {summary} '
        f'({html.escape(reasons)})</span>')


async def _submit_batch(
    batch: list[order_cli.NewOrder], label: str, note: str = ""
) -> HTMLResponse:
    try:
        outcomes = await _send_batch(batch)
    except rsx_cli.CliError as err:
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'{label} error: {html.escape(str(err))}</span>')
    return _batch_response(label, outcomes, note)


@app.post("/api/orders/batch")
async def api_orders_batch(request: Request, count: int = Query(10)):
    denied = _require_admin_request(request)
    if denied:
        return denied
    audit_log("/api/orders/batch", f"count={count}")
    n = max(1, min(count, _BATCH_MAX))
    venue_err = await _execution_venue_error(BATCH_SYMBOL_ID)
    if venue_err:
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">batch orders not sent: '
            f'{html.escape(venue_err[1])}</span>')
    mid = _book_mid(BATCH_SYMBOL_ID)
    if mid is None:
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">batch orders not sent: '
            'no book to price against</span>')
    batch = []
    for index in range(n):
        side = "buy" if index % 2 == 0 else "sell"
        offset_pct = (1 + index // 2) * BATCH_STEP_PCT
        batch.append(order_cli.NewOrder(
            user_id=BATCH_USER_ID,
            symbol_id=BATCH_SYMBOL_ID,
            side=side,
            px=_px_from_mid(
                BATCH_SYMBOL_ID, mid,
                -offset_pct if side == "buy" else offset_pct),
            is_min_size=True,
        ))
    capped = " (capped)" if count > _BATCH_MAX else ""
    return await _submit_batch(batch, "batch orders", capped)


async def _live_symbol_mids() -> list[tuple[int, int]]:
    await refresh_symbols()
    live = []
    for symbol in list(symbols.table.values()):
        mid = _book_mid(symbol.id)
        if mid is not None and await _execution_venue_error(symbol.id) is None:
            live.append((symbol.id, mid))
    return live


@app.post("/api/orders/random")
async def api_orders_random(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
    audit_log("/api/orders/random", f"count={RANDOM_ORDERS}")
    live = await _live_symbol_mids()
    if not live:
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">random orders not sent: '
            'no running symbol with a book to price against</span>')
    batch = []
    for index in range(RANDOM_ORDERS):
        symbol_id, mid = live[index % len(live)]
        batch.append(order_cli.NewOrder(
            user_id=BATCH_USER_ID,
            symbol_id=symbol_id,
            side=random.choice(_SIDES),
            px=_px_from_mid(
                symbol_id, mid,
                random.uniform(-RANDOM_SPREAD_PCT, RANDOM_SPREAD_PCT)),
            is_min_size=True,
        ))
    return await _submit_batch(batch, "random orders")


QUICK_QTYS = (10, 20, 50, 100)


@app.post("/api/orders/quick")
async def api_orders_quick(request: Request):
    """Quick order endpoint for the matrix buttons.

    Form params:
      user_id          int    default 1
      side             buy|sell (ignored if randomize=true)
      qty              int (ignored if randomize=true)
      price_offset_pct float  0 = market, positive = above mid,
                              negative = below mid
      symbol_id        int    default 10
      randomize        true|false
      rand_side        true|false  random side only, use qty param
    """
    denied = _require_admin_request(request)
    if denied:
        return denied
    form = await request.form()
    audit_log("/api/orders/quick", "submit order")

    try:
        user_id = int(form.get("user_id", "1"))
    except (ValueError, TypeError):
        return HTMLResponse(
            '<span class="text-red-400 text-xs">invalid user_id</span>')

    try:
        symbol_id = int(form.get("symbol_id", "10"))
    except (ValueError, TypeError):
        symbol_id = 10

    randomize = form.get("randomize", "false").lower() == "true"
    rand_side = form.get("rand_side", "false").lower() == "true"
    try:
        offset_pct = float(form.get("price_offset_pct", "0"))
    except (ValueError, TypeError):
        offset_pct = 0.0
    qty = _decimal_field(form.get("qty", "1"), "1") or Decimal(1)
    side = str(form.get("side", "buy")).lower()
    if randomize:
        qty = Decimal(random.choice(QUICK_QTYS))
        offset_pct = random.uniform(-2.0, 2.0)
    if randomize or rand_side:
        side = random.choice(_SIDES)
    if side not in _SIDES:
        return HTMLResponse(
            '<span class="text-red-400 text-xs">invalid side</span>')
    if qty <= 0:
        return HTMLResponse(
            '<span class="text-red-400 text-xs">qty must be positive</span>')

    mid = _book_mid(symbol_id)
    is_market = offset_pct == 0.0 and not randomize
    if mid is None or is_market:
        px = _sweep_px(symbol_id, side)
        tif = "IOC"
    else:
        px = _px_from_mid(symbol_id, mid, offset_pct)
        tif = "GTC"

    new_order = order_cli.NewOrder(
        user_id=user_id,
        symbol_id=symbol_id,
        side=side,
        tif=tif,
        px=px,
        qty=format(qty, "f"),
    )
    order = {
        "user_id": user_id,
        "symbol": str(symbol_id),
        "side": side,
        "price": pages.format_price(px, symbol_id),
        "qty": format(qty, "f"),
        "tif": tif,
        "reduce_only": False,
        "post_only": False,
        "status": "pending",
        "ts": _timestamp(),
    }
    label = f"{side.upper()} {qty:.0f}"
    try:
        outcome = await ORDER_GATEWAY.place(new_order)
    except rsx_cli.CliError as err:
        is_misuse = err.code == order_cli.EXIT_MISUSE
        if not is_misuse:
            _store_failure(order, err)
        color = "text-red-400" if is_misuse else "text-amber-400"
        return HTMLResponse(
            f'<span class="{color} text-xs font-medium">'
            f'error: {html.escape(str(err))}</span>')
    _store_outcome(order, outcome)
    cid = order["cid"]
    if outcome.status == "timeout":
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs font-medium">'
            f'{label} timeout ({cid})</span>',
            status_code=504)
    is_ok = outcome.status in ("filled", "partial", "resting")
    color = "text-emerald-400" if is_ok else "text-red-400"
    detail = f": {html.escape(outcome.reason)}" if outcome.reason else ""
    return HTMLResponse(
        f'<span class="{color} text-xs font-medium">'
        f'{label} {outcome.status}{detail}</span>')


# ── stress run ───────────────────────────────────────────

STRESS_NAME = "stress"
STRESS_USERS = 10


def _stress_target(gateway_url: str = "") -> stress_cli.Target:
    return stress_cli.Target(
        gateway_url or GATEWAY_URL, MARKETDATA_WS, PG_URL)


async def do_stress_start(
    rate: int, duration: int, gateway_url: str = "",
) -> None:
    """Start the stress child; rsx_cli.CliError when it does not stay up."""
    load = stress_cli.Load(rate, duration, STRESS_USERS)
    await stress_run.start(load, _stress_target(gateway_url))


async def _stress_params(request: Request) -> dict:
    try:
        body = await request.json()
    except ValueError:
        body = dict(await request.form())
    return body if isinstance(body, dict) else {}


@app.post("/api/stress/run")
async def api_stress_run(
    request: Request,
    rate: int = Form(default=10),
    duration: int = Form(default=1),
    gateway_url: str = Form(default=""),
):
    """Start a stress run and return a structured JSON envelope.

    On start: 200 + the run's settings.
    On gateway unreachable: 502 + {"code":"GATEWAY_UNREACHABLE",
    "message":..., "context":{"gateway_url":...}}.
    On bad form: 400 + {"code":"BAD_REQUEST", "message":...}.
    On a child that does not stay up: 500 + {"code":"STRESS_FAILED", ...}
    with the CLI's own message.
    HTMX path (hx-request: true) returns 200 with an error
    span instead of 502 so the UI can render inline.
    """
    is_htmx = request.headers.get("hx-request") == "true"
    for name, value in ("rate", rate), ("duration", duration):
        if value <= 0:
            return JSONResponse(
                status_code=400,
                content={
                    "code": "BAD_REQUEST",
                    "message": f"{name} must be > 0",
                    "context": {name: value},
                })
    target_url = gateway_url or GATEWAY_URL
    # Probe gateway reachability (TCP connect to host:port).
    from urllib.parse import urlparse
    parsed = urlparse(target_url)
    host = parsed.hostname or "localhost"
    port = parsed.port or 8088
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
        if is_htmx:
            return HTMLResponse(
                f'<span class="text-red-400 text-xs">'
                f'gateway unreachable: {html.escape(target_url)}</span>',
                status_code=200)
        return JSONResponse(
            status_code=502,
            content={
                "code": "GATEWAY_UNREACHABLE",
                "message": f"could not reach gateway at {target_url}",
                "context": {"gateway_url": target_url},
            })
    is_busy = stress_run.is_running()
    failure = None
    if not is_busy:
        try:
            await do_stress_start(rate, duration, target_url)
        except rsx_cli.CliError as err:
            failure = str(err)
    if is_htmx:
        if is_busy:
            return HTMLResponse(
                '<span class="text-amber-400 text-xs">'
                'stress already running</span>')
        if failure is not None:
            return HTMLResponse(
                '<span class="text-red-400 text-xs">'
                f'Failed: {html.escape(failure)}</span>')
        return HTMLResponse(
            f'<div class="text-xs text-emerald-400">stress running '
            f'&mdash; {rate}/s for {duration}s against {html.escape(target_url)}'
            f'</div>'
            f'<div class="text-slate-500 text-[11px] mt-1">a report '
            f'appears under <a href="./stress" class="text-blue-400 '
            f'hover:underline">Stress &rarr; historical reports</a> '
            f'when it finishes.</div>')
    if failure is not None:
        return JSONResponse(
            status_code=500,
            content={
                "code": "STRESS_FAILED",
                "message": failure,
                "context": {"gateway_url": target_url},
            })
    return JSONResponse({
        "code": "OK",
        "message": "stress busy" if is_busy else "stress started",
        "context": {
            "rate": rate,
            "duration": duration,
            "gateway_url": target_url,
            "started": not is_busy,
        },
    })


@app.post("/api/stress/start")
async def api_stress_start(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
    if stress_run.is_running():
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">'
            'stress already running</span>')
    params = await _stress_params(request)
    try:
        rate = int(params.get("rate", 1000))
        duration = int(params.get("duration", 60))
    except (TypeError, ValueError):
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            'Failed: rate and duration must be integers</span>',
            status_code=400)
    if rate <= 0 or duration <= 0:
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            'Failed: rate and duration must be > 0</span>',
            status_code=400)
    try:
        await do_stress_start(
            rate, duration, str(params.get("gw_url", "")))
    except rsx_cli.CliError as err:
        return HTMLResponse(
            '<span class="text-red-400 text-xs">'
            f'Failed: {html.escape(str(err))}</span>')
    audit_log("/api/stress/start", "start stress")
    return HTMLResponse(
        '<span class="text-emerald-400 text-xs">'
        f'stress started (pid {stress_run.pid})</span>')


@app.post("/api/stress/stop")
async def api_stress_stop(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
    if not stress_run.is_running():
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">'
            'stress not running</span>')
    await stress_run.stop()
    audit_log("/api/stress/stop", "stop stress")
    return HTMLResponse(
        '<span class="text-amber-400 text-xs">'
        'stress stopped</span>')


@app.get("/api/stress/status")
async def api_stress_status():
    running = stress_run.is_running()
    return {
        "running": running,
        "pid": stress_run.pid if running else None,
        "name": STRESS_NAME,
        "report": stress_run.report.stem.removeprefix("stress-")
        if stress_run.report else None,
        "error": stress_run.error,
    }


@app.get("/api/stress/reports")
async def api_stress_reports():
    """List all stress test reports.

    A corrupt file, or one that is not a report of `rsx-cli stress`, is
    surfaced as {id, corrupt: True, error: ...} instead of being dropped.
    """
    reports = []
    for path in sorted(STRESS_REPORTS_DIR.glob("stress-*.json"), reverse=True):
        report_id = path.stem.removeprefix("stress-")
        try:
            report = stress_cli.read_report(path)
            reports.append(
                asdict(stress_cli.summarize(report_id, report)))
        except (OSError, ValueError, KeyError, TypeError) as exc:
            reports.append({
                "id": report_id,
                "corrupt": True,
                "error": f"{type(exc).__name__}: {exc}",
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


def _stress_scenario_desc(scenario: cluster_cli.Scenario) -> str:
    """Build a human-readable description of a stress scenario."""
    parts = [
        f"{scenario.symbols}sym",
        f"{scenario.gateways}gw",
        f"rep={scenario.replication}",
    ]
    load = scenario.load
    if load:
        parts.append(
            f"{load['rate']}/s {load['duration']}s {load['type']}")
    return ", ".join(parts)


@app.post("/api/stress/scenario/{name}/start")
async def api_stress_scenario_start(name: str, request: Request):
    """Start RSX cluster with the named stress scenario, then
    re-render the scenario panel so HTMX swaps the new state in."""
    denied = _require_admin_request(request)
    if denied:
        return denied
    if name not in [s.name for s in await get_scenarios()]:
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'unknown scenario: {html.escape(name)}</span>',
            status_code=400,
        )
    asyncio.create_task(_start_all_in_background(name))
    return await x_stress_scenarios()


@app.post("/api/stress/scenario/{name}/stop")
async def api_stress_scenario_stop(name: str, request: Request):
    """Stop the stress run (subprocess only); leave RSX up."""
    denied = _require_admin_request(request)
    if denied:
        return denied
    await stress_run.stop()
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

    Read the same scenarios the Control scenario selector uses
    (`rsx-cli cluster scenarios`) so the two surfaces never drift.
    """
    running = stress_run.is_running()
    states: dict[str, dict] = {}
    for scenario in await get_scenarios():
        if scenario.load is None:
            continue
        states[scenario.name] = {
            "running": running and current_scenario == scenario.name,
            "desc": _stress_scenario_desc(scenario),
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
        if r.get("corrupt"):
            report_id = html.escape(str(r["id"]))
            error = html.escape(r.get("error", "corrupt report"))
            rows.append(
                f'<tr><td class="px-2 py-1 text-xs text-red-400" '
                f'colspan="6">{report_id}: {error}</td></tr>'
            )
            continue
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
        p99 = r["p99_latency"]
        latency_color = ("text-emerald-400"
                         if p99 is not None and p99 < 1000
                         else "text-red-400")
        p99_text = f"{p99}us" if p99 is not None else "unavailable"
        failure_title = html.escape("; ".join(map(str, r["failures"])))
        status = html.escape(str(r["status"]))

        rows.append(
            f'<tr class="hover:bg-slate-800/30" title="{failure_title}">'
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
            f' {latency_color}">{p99_text} ({status})</td>'
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
async def api_orders_invalid(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
    audit_log("/api/orders/invalid", "submit order")
    venue_err = await _execution_venue_error(BATCH_SYMBOL_ID)
    if venue_err:
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">invalid order not sent: '
            f'{html.escape(venue_err[1])}</span>')
    misaligned = order_cli.NewOrder(
        user_id=BATCH_USER_ID,
        symbol_id=BATCH_SYMBOL_ID,
        side="buy",
        tif="ioc",
        px=_symbol_tick(BATCH_SYMBOL_ID),
        qty_raw=1,
    )
    try:
        outcomes = await _send_batch([misaligned])
    except rsx_cli.CliError as err:
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">invalid order error: '
            f'{html.escape(str(err))}</span>')
    outcome = outcomes[0]
    detail = f": {html.escape(outcome.reason)}" if outcome.reason else ""
    color = "text-red-400" if outcome.status == "rejected" else "text-amber-400"
    return HTMLResponse(
        f'<span class="{color} text-xs">'
        f'invalid order {outcome.status}{detail}</span>')


@app.post("/api/wal/verify")
async def api_wal_verify():
    listings = await wal_listings()
    streams = [s for listing in listings for s in listing.streams]
    if not streams:
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">'
            'no WAL streams found</span>')
    report = await wal_feed.verify(listings)
    problems = wal_feed.describe_problems(report, listings)
    if problems:
        return HTMLResponse(
            '<div class="text-red-400 text-xs">Failed: WAL corrupt'
            + "".join(
                f"<div>{html.escape(line)}</div>" for line in problems)
            + "</div>")
    total_files = sum(len(s.segments) for s in streams)
    records = sum(entry["records"] for entry in report["streams"])
    return HTMLResponse(
        f'<span class="text-emerald-400 text-xs">'
        f'verified {len(streams)} streams, '
        f'{total_files} files, {records} records</span>')


_WAL_DUMP_HEAD = 512 * 1024      # bytes of the file to feed rsx-cli
_WAL_DUMP_LINE_CAP = 200         # records rendered, whichever first


@app.post("/api/wal/dump")
async def api_wal_dump():
    files = await list_wal_files()
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

    # rsx-cli `wal file` read_to_end()s the WHOLE file — a rotated archive
    # can be gigabytes and OOMs the CLI before it prints a line. But
    # `wal file` stops cleanly at a partial trailing record, so we can
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
        try:
            output = await rsx_cli.run_text("wal", "file", str(tmp_dump))
        except rsx_cli.CliError as exc:
            err = str(exc)
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
    try:
        account = await risk_cli.account_show(PG_URL, user_id, bbo_marks())
    except risk_cli.NoAccount as err:
        return JSONResponse({"error": str(err)}, status_code=404)
    return dataclasses.asdict(account)


# ── Risk dashboard API endpoints ──────────────────────────

@app.get("/api/risk/overview")
async def api_risk_overview():
    """Account overview: collateral, margin and positions of every
    account from `rsx-cli risk overview`, margined at the active
    config's rates and marked at the BBO mid."""
    overview = await read_risk()
    margin_bps = {}
    for config in await read_config():
        row = config.active()
        if row is not None:
            margin_bps[config.symbol_id] = (
                row.initial_margin_rate_bps or 0,
                row.maintenance_margin_rate_bps or 0)

    users_out = []
    all_long_notional = 0
    all_short_notional = 0
    accounts_with_positions = 0
    accounts_near_liq = 0
    for account in overview.users:
        pos_list = []
        total_upnl = 0
        total_im = 0
        total_mm = 0
        for position in account.positions:
            sid = position.symbol_id
            net = position.net_qty
            mark_px = overview.marks.get(sid, position.avg_px)
            notional = abs(net) * mark_px
            upnl = position.upnl or 0
            im_bps, mm_bps = margin_bps.get(sid, (0, 0))
            im = notional * im_bps // 10_000
            mm = notional * mm_bps // 10_000
            total_upnl += upnl
            total_im += im
            total_mm += mm
            if net > 0:
                all_long_notional += notional
            else:
                all_short_notional += notional
            pos_list.append({
                "symbol_id": sid,
                "net": net,
                "entry_px": position.avg_px,
                "mark_px": mark_px,
                "upnl": upnl,
                "notional": notional,
                "im": im,
                "mm": mm,
            })

        if pos_list:
            accounts_with_positions += 1

        frozen = account.frozen_amount()
        equity = account.collateral + total_upnl
        margin_ratio = (
            equity / total_mm if total_mm > 0 else 999.0
        )
        if margin_ratio < 1.5 and total_mm > 0:
            accounts_near_liq += 1

        users_out.append({
            "user_id": account.user_id,
            "collateral": account.collateral,
            "frozen": frozen,
            "available": max(0, account.collateral - frozen),
            "equity": equity,
            "upnl": total_upnl,
            "im_required": total_im,
            "mm_required": total_mm,
            "margin_ratio": round(margin_ratio, 3),
            "positions": pos_list,
        })

    # Users with ANY fill activity, even at a net-zero position: the
    # Maker tab's "Positions N" counts the same set, and the Risk page
    # must not show fewer. Gross OI is the sum of filled qty per
    # symbol, so it stays non-zero when buyers and sellers offset.
    fills = wal_records.fills(2000)
    fill_users = set()
    gross_oi_qty: dict[int, int] = {}
    for fill in fills:
        fill_users.update(
            user_id for user_id in (
                fill.get("taker_user_id", 0), fill.get("maker_user_id", 0))
            if user_id)
        sid = fill.get("symbol_id", 0)
        gross_oi_qty[sid] = gross_oi_qty.get(sid, 0) + fill.get("qty", 0)

    total_oi = all_long_notional + all_short_notional
    # No open position but fills exist (buyers = sellers): fall back
    # to gross filled notional so the dashboard doesn't lie.
    if total_oi == 0 and gross_oi_qty:
        gross_notional = 0
        for sid, qty in gross_oi_qty.items():
            mid = overview.marks.get(sid, 0)
            if mid:
                gross_notional += qty * mid
        total_oi = gross_notional
        # Split evenly between sides for display purposes when
        # we can't infer direction from netting.
        all_long_notional = gross_notional // 2
        all_short_notional = gross_notional - all_long_notional
    accounts_with_positions = max(
        accounts_with_positions, len(fill_users)
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
    book_stats = wal_records.book_stats()
    # Real index source: the mark process aggregates external
    # exchanges (Binance/Coinbase) into RECORD_MARK_PRICE on the
    # `mark` WAL stream. Premium = (book mid - index) / index.
    mark_prices = wal_records.mark_prices()
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
    """Liquidation queue: risk's table, else the WAL's records."""
    rows = (await read_risk()).liquidations
    if rows:
        return {"liquidations": rows, "source": "postgres"}
    liqns = wal_records.liquidations()
    return {
        "liquidations": liqns[:50],
        "source": "wal" if liqns else "none",
    }


@app.get("/api/risk/insurance")
async def api_risk_insurance():
    """Insurance fund balances per symbol."""
    overview = await read_risk()
    if not overview.funds:
        return {"funds": [], "total": 0, "source": "none"}
    return {
        "funds": [dataclasses.asdict(fund) for fund in overview.funds],
        "total": overview.insurance_total,
        "source": "postgres",
    }


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


# ── Latency probes (rsx-cli stress) ──────────────────────
# One order measured by `rsx-cli stress`: a crossing IOC that the maker
# fills (e2e, GW → ME → GW) or an order for a symbol the gateway does not
# serve (gateway only: no risk, no ME). The sample feeds the `e2e` and
# `gw_only` blocks of /api/latency.


def _record_latency(samples: list[int], elapsed_us: int) -> None:
    samples.append(elapsed_us)
    if len(samples) > 1000:
        del samples[:500]


@app.post("/api/latency-probe")
async def api_latency_probe(
    request: Request,
    symbol_id: int = 10,
):
    """Submit a probe order, wait for the fill, record the E2E us.

    Requires the maker to be running (so the order fills against
    existing liquidity). Loopback only (no admin token required)."""
    probe = await stress_cli.probe(
        _stress_target(), "probe", symbol_id)
    result: dict = {"ok": probe.error is None, "symbol_id": symbol_id}
    if probe.elapsed_us is None:
        result["error"] = probe.error
    else:
        result["elapsed_us"] = probe.elapsed_us
        _record_latency(e2e_latencies, probe.elapsed_us)
    if request.headers.get("hx-request") == "true":
        return HTMLResponse(pages.render_probe_result(result))
    return JSONResponse(result)


@app.post("/api/latency-probe-gw")
async def api_latency_probe_gw(request: Request):
    """Submit an order the gateway rejects before risk and the engine.

    Result feeds the `gw_only` block of /api/latency."""
    probe = await stress_cli.probe(
        _stress_target(), "reject")
    if probe.elapsed_us is None:
        return JSONResponse({"ok": False, "error": probe.error})
    _record_latency(gw_only_latencies, probe.elapsed_us)
    return JSONResponse({
        "ok": True,
        "probe_ok": True,
        "elapsed_us": probe.elapsed_us,
        "reasons": probe.reasons,
    })


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
    bbo_map = wal_records.book_stats()
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
    procs = await scan_processes()
    return {
        "processes": len(procs),
        "running": len(
            [p for p in procs if p["state"] == "running"]),
        "postgres": await _probe_postgres_tcp(),
    }


@app.get("/api/status")
async def api_status():
    procs = await scan_processes()
    running = [p for p in procs if p["state"] == "running"]
    maker = find_running(procs, MAKER_NAME)
    maker_running = maker is not None
    maker_stats = _read_maker_stats() if maker_running else {}
    maker_pid = maker["pid"] if maker else None
    gateway_up, marketdata_up, postgres_up = await asyncio.gather(
        _probe_gateway_tcp(),
        _probe_marketdata_tcp(),
        _probe_postgres_tcp(),
    )
    return {
        "processes": len(procs),
        "running": len(running),
        "postgres": postgres_up,
        "gateway": gateway_up,
        "marketdata": marketdata_up,
        "maker": {
            "running": maker_running,
            "pid": maker_pid,
            "levels": maker_stats.get("levels", 0),
        },
    }


# ── rsx-auth (OAuth identity service) ───────────────────

AUTH_NAME = "auth"


def _auth_configured() -> bool:
    """True if GitHub client id + secret are present."""
    return bool(
        os.environ.get("RSX_AUTH_GITHUB_CLIENT_ID")
        and os.environ.get("RSX_AUTH_GITHUB_CLIENT_SECRET")
        and os.environ.get("RSX_GW_JWT_SECRET")
    )


@app.get("/api/auth/status")
async def api_auth_status():
    pid = await _running_pid(AUTH_NAME)
    return {
        "running": pid is not None,
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
    try:
        await cluster_cli.up(current_scenario, only=(AUTH_NAME,))
    except rsx_cli.CliError as err:
        return JSONResponse(
            {"error": f"rsx-auth failed to start: {err}"},
            status_code=500)
    finally:
        forget_procs()
    return {"ok": True}


@app.post("/api/auth/stop")
async def api_auth_stop(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
    await cluster_cli.down((AUTH_NAME,))
    forget_procs()
    return {"ok": True}


# ── market maker ────────────────────────────────────────

MAKER_NAME = "maker"
MAKER_STATUS_FILE = TMP / "maker-status.json"


def _read_maker_stats() -> dict:
    """Read maker status file written by maker subprocess."""
    try:
        return json.loads(MAKER_STATUS_FILE.read_text())
    except Exception:
        return {}


async def _maker_running() -> bool:
    return await _running_pid(MAKER_NAME) is not None


@app.post("/api/maker/start")
async def api_maker_start(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
    if await _maker_running():
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">'
            'maker already running</span>')
    try:
        await cluster_cli.up(current_scenario, only=(MAKER_NAME,))
    except rsx_cli.CliError as err:
        return _status_span("red", f"maker failed to start: {err}")
    finally:
        forget_procs()
    audit_log("/api/maker/start", "start maker")
    return _status_span(
        "emerald", f"maker started (pid {await _running_pid(MAKER_NAME)})")


@app.post("/api/maker/stop")
async def api_maker_stop(request: Request):
    denied = _require_admin_request(request)
    if denied:
        return denied
    if not await _maker_running():
        return HTMLResponse(
            '<span class="text-amber-400 text-xs">'
            'maker not running</span>')
    _restart_entry(MAKER_NAME)["intentional"] = True
    try:
        await cluster_cli.down((MAKER_NAME,))
    except rsx_cli.CliError as err:
        return _status_span("red", f"Failed: {err}")
    finally:
        forget_procs()
    audit_log("/api/maker/stop", "stop maker")
    return HTMLResponse(
        '<span class="text-amber-400 text-xs">'
        'maker stopped</span>')


@app.get("/api/maker/status")
async def api_maker_status():
    pid = await _running_pid(MAKER_NAME)
    running = pid is not None
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
    if await _maker_running():
        try:
            await cluster_cli.restart(MAKER_NAME, current_scenario)
        except rsx_cli.CliError:
            return HTMLResponse(
                '<span class="text-red-400 text-xs">'
                'config saved; maker failed to restart</span>')
        finally:
            forget_procs()
        pid = await _running_pid(MAKER_NAME)
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
    try:
        await cluster_cli.restart(MAKER_NAME, current_scenario)
    except rsx_cli.CliError as err:
        return _status_span("red", f"maker failed to start: {err}")
    finally:
        forget_procs()
    pid = await _running_pid(MAKER_NAME)
    audit_log("/api/maker/restart", f"restarted maker (pid {pid})")
    return HTMLResponse(
        '<span class="text-emerald-400 text-xs">'
        f'maker restarted (pid {pid})</span>')


async def _maker_book(symbol_id: int) -> dict | None:
    """Synthesize a book snapshot from maker config/status.

    Returns None when maker is stopped or has no mid price.
    Sources checked in order:
      1. maker-status.json mid_prices (set by maker after preflight)
      2. maker-config.json mid_override (set by admin PATCH)
    Requires the maker to be running so the book clears when it stops.
    """
    if not await _maker_running():
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
    bbo = wal_records.bbo(symbol_id)
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
    maker_snap = await _maker_book(symbol_id)
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
    bbo = wal_records.bbo(symbol_id)
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
    maker_snap = await _maker_book(symbol_id)
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


def _maker_liquidity_live(procs: list[dict]) -> bool:
    """True when the maker's quotes can actually be resting on a
    live book: the gateway and at least one ME are running. When
    false, the maker's local active_cids/levels are stale bookkeeping
    (the gateway dropped the orders), not real liquidity.
    """
    running = {
        p["name"] for p in procs
        if p.get("state") == "running"
    }
    gw_ok = any(n.startswith("gw-") for n in running)
    me_ok = any(n.startswith("me-") for n in running)
    return gw_ok and me_ok


@app.get("/x/maker-status", response_class=HTMLResponse)
async def x_maker_status():
    procs = await scan_processes()
    maker = find_running(procs, MAKER_NAME)
    if maker is None:
        return HTMLResponse(
            '<span class="text-slate-500 text-xs">'
            'maker stopped</span>')
    stats = _read_maker_stats()
    return HTMLResponse(
        pages.maker_status_html(
            stats, maker["pid"],
            liquidity_live=_maker_liquidity_live(procs)))


@app.get("/x/maker-live", response_class=HTMLResponse)
async def x_maker_live():
    """HTMX partial: live maker status for maker page."""
    procs = await scan_processes()
    maker = find_running(procs, MAKER_NAME)
    running = maker is not None
    pid = maker["pid"] if maker else None
    rs = _restart_state.get(MAKER_NAME, {})
    restarts = rs.get("restarts", 0)
    stats = _read_maker_stats() if running else {}
    return HTMLResponse(
        pages.maker_live_html(
            running=running,
            pid=pid,
            restarts=restarts,
            stats=stats,
            liquidity_live=(
                _maker_liquidity_live(procs) if running else True),
        )
    )


# ── gateway/marketdata probes, symbol catalog, auth proxy ──


async def _probe_tcp(url: str, default_port: int) -> bool:
    """Return True if the host and port of `url` accept a connection."""
    import urllib.parse
    parsed = urllib.parse.urlparse(url)
    host = parsed.hostname or "localhost"
    port = parsed.port or default_port
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


async def _probe_gateway_tcp() -> bool:
    return await _probe_tcp(GATEWAY_HTTP, 8088)


async def _probe_marketdata_tcp() -> bool:
    return await _probe_tcp(MARKETDATA_WS, 8081)


async def _probe_postgres_tcp() -> bool:
    return await _probe_tcp(PG_URL, 5432)


@app.get("/v1/symbols")
async def v1_symbols():
    """Configured symbol catalog (tick, lot, decimals) for the maker,
    the demo scripts and the symbol-config page. Each symbol with a
    row in force carries `min_notional` (raw, risk's price x qty unit)
    and `min_notional_usd` when the row has one."""
    rows = []
    for config in await read_config():
        row = config.active()
        if row is None or config.name is None:
            continue
        entry = {
            "id": config.symbol_id,
            "symbol": config.name,
            "tick_size": row.tick_size,
            "lot_size": row.lot_size,
            "price_decimals": row.price_decimals,
            "qty_decimals": row.qty_decimals,
        }
        if row.min_notional is not None:
            entry["min_notional"] = row.min_notional
            entry["min_notional_usd"] = notional.usd_from_raw(
                row.min_notional, row.price_decimals, row.qty_decimals)
        rows.append(entry)
    rows.sort(key=lambda r: r["id"])
    m_tuples = [
        [r["id"], r["tick_size"], r["lot_size"], r["symbol"]]
        for r in rows
    ]
    return JSONResponse({"symbols": rows, "M": m_tuples})


@app.api_route(
    "/auth/{path:path}",
    methods=["GET", "POST"],
)
@app.api_route(
    "/oauth/{path:path}",
    methods=["GET", "POST"],
)
@app.api_route(
    "/roles/{path:path}",
    methods=["GET", "POST"],
)
async def auth_proxy(path: str, request: Request):
    """Proxy /auth/*, /oauth/*, and /roles/* to rsx-auth.

    In dev, browser clients talk to playground; playground
    forwards auth flows to rsx-auth on :8082. Production
    should use nginx or similar to route directly. /roles/* carries
    the caller's own forwarded Bearer/cookie token straight through --
    rsx-auth's own RBAC (rbac.py `require(action)`) is the sole
    authority on whether the caller may read/write it, exactly like
    /auth/* and /oauth/*.
    """
    prefix = request.url.path.split("/", 2)[1]  # 'auth', 'oauth', or 'roles'
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
                      "content-type", "idempotency-key",
                      "x-request-id"):
                if h in request.headers:
                    fwd_headers[h] = request.headers[h]
            # rsx-auth's own endpoints (verify_bearer) check ONLY the
            # Authorization header, never a cookie -- a browser session
            # (rsx_token cookie, no custom header, e.g. this tab's own
            # client-side fetch() calls) would otherwise 401 on every
            # /auth/me or /roles/* call. Translate a cookie-only token
            # into Bearer; the raw cookie still forwards above
            # unchanged (needed for the OAuth flow's own
            # rsx_oauth_state/rsx_oauth_redirect cookies).
            if "authorization" not in fwd_headers:
                token = _extract_token_from_headers(request.headers)
                if token:
                    fwd_headers["authorization"] = f"Bearer {token}"
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
