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
from stress_client import run_stress_test
from stress_client import StressConfig

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
_loader = importlib.machinery.SourceFileLoader(
    "start_mod", str(ROOT / "start"))
_spec = importlib.util.spec_from_loader(
    "start_mod", _loader)
start_mod = importlib.util.module_from_spec(_spec)
_loader.exec_module(start_mod)

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


async def _md_ws_subscriber():
    """Subscribe to marketdata WS; maintain _book_snap from L2/BBO."""
    # CHANNEL_BBO=1, CHANNEL_DEPTH=2
    CHANNELS = 3
    DEFAULT_SYMBOLS = [10]

    while True:
        try:
            async with aiohttp.ClientSession() as session:
                async with session.ws_connect(
                    f"{MARKETDATA_WS}/ws",
                    heartbeat=10,
                ) as ws:
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
        except Exception:
            pass
        await asyncio.sleep(2)


# ── process manager ─────────────────────────────────────

# name -> {"proc": asyncio.Process, "binary": str,
#          "env": dict}
managed: dict[str, dict] = {}
build_log: list[str] = []
current_scenario = "minimal"


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
        return {"error": f"process exited immediately (code "
                         f"{proc.returncode})"}


async def stop_process(name):
    """Stop a managed process by SIGTERM."""
    info = managed.get(name)
    if not info:
        return {"error": f"{name} not managed"}
    proc = info["proc"]
    if proc.returncode is not None:
        # already stopped, clean PID file
        pid_file = PID_DIR / f"{name}.pid"
        if pid_file.exists():
            pid_file.unlink()
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
    # Clean up managed dict
    if name in managed:
        del managed[name]
    return {"status": f"{name} stopped"}


async def kill_process(name):
    """Kill a managed process by SIGKILL."""
    info = managed.get(name)
    if not info:
        return {"error": f"{name} not managed"}
    proc = info["proc"]
    if proc.returncode is not None:
        del managed[name]
        return {"status": f"{name} already stopped"}
    proc.kill()
    await proc.wait()
    pid_file = PID_DIR / f"{name}.pid"
    if pid_file.exists():
        pid_file.unlink()
    del managed[name]
    return {"status": f"{name} killed"}


async def restart_process(name):
    """Restart a managed process."""
    info = managed.get(name)
    if not info:
        return {"error": f"{name} not managed"}
    await stop_process(name)
    await asyncio.sleep(0.3)
    return await spawn_process(
        name, info["binary"], info["env"])


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


# collateral for playground test users: 1 trillion raw units
# (tick=1 for all symbols, so this = 1T ticks of buying power)
_SEED_USERS = [1, 2, 3, 4, 5, 99]
_SEED_COLLATERAL = 1_000_000_000_000


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
    env = {
        "GATEWAY_URL": GATEWAY_URL,
        "MARKETDATA_WS": MARKETDATA_WS,
        "RSX_SYMBOLS_URL": f"http://localhost:{49171}/v1/symbols",
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
        for port in [8080, 8180, 9110, 9200, 9400, 9510]:
            try:
                subprocess.run(
                    ["fuser", "-k", f"{port}/tcp"],
                    capture_output=True, timeout=2,
                )
            except subprocess.TimeoutExpired:
                pass
        for port in [9110, 9200, 9510]:
            try:
                subprocess.run(
                    ["fuser", "-k", f"{port}/udp"],
                    capture_output=True, timeout=2,
                )
            except subprocess.TimeoutExpired:
                pass
    else:
        # fallback: check with lsof and kill by PID
        for port in [8080, 8180, 9110, 9200, 9400, 9510]:
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

    return {"started": started, "count": len(started)}


async def stop_all():
    """Stop all managed processes."""
    stopped = []
    for name in list(managed.keys()):
        await stop_process(name)
        stopped.append(name)
    return {"stopped": stopped}


@asynccontextmanager
async def lifespan(app):
    global _md_ws_task
    await pg_connect()
    _md_ws_task = asyncio.create_task(_md_ws_subscriber())
    yield
    if _md_ws_task:
        _md_ws_task.cancel()
        try:
            await _md_ws_task
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
    gateway_up = await _probe_gateway_tcp()
    return {
        "status": "ok",
        "port": 49171,
        "processes_running": len(running),
        "processes_total": len(procs),
        "postgres": pg_pool is not None,
        "gateway": gateway_up,
    }

# ── in-memory state ─────────────────────────────────────

recent_orders: list[dict] = []
verify_results: list[dict] = []
order_latencies: list[int] = []
gateway_ws = None
_idempotency_keys: dict[str, float] = {}
_IDEMPOTENCY_TTL = 300

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
                ps = psutil.Process(proc.pid)
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
                result.append({
                    "name": name, "pid": proc.pid,
                    "state": "running", "cpu": "-",
                    "mem": "-", "uptime": "-",
                })
        else:
            result.append({
                "name": name, "pid": "-",
                "state": "stopped", "cpu": "-",
                "mem": "-", "uptime": "-",
            })

    # 2. PID files (from ./start or previous session)
    if PID_DIR.exists():
        for pid_file in sorted(PID_DIR.glob("*.pid")):
            name = pid_file.stem
            if name in seen:
                continue
            try:
                pid = int(pid_file.read_text().strip())
                ps = psutil.Process(pid)
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


def parse_wal_bbo(symbol_id):
    """Get latest BBO for a symbol from WAL."""
    latest = None
    for stream_dir in _wal_stream_dirs():
        for rec in parse_wal_records(
            stream_dir, {RECORD_BBO}
        ):
            if rec["symbol_id"] == symbol_id:
                if latest is None or rec["seq"] > latest["seq"]:
                    latest = rec
    return latest


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
    """Get book stats from WAL BBO records."""
    symbols = {}
    for stream_dir in _wal_stream_dirs():
        for rec in parse_wal_records(
            stream_dir, {RECORD_BBO}
        ):
            sid = rec["symbol_id"]
            if sid not in symbols or rec["seq"] > symbols[sid]["seq"]:
                symbols[sid] = rec
    return symbols


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
    return HTMLResponse(pages.overview_page())


@app.get("/overview", response_class=HTMLResponse)
async def overview():
    return HTMLResponse(pages.overview_page())


@app.get("/topology", response_class=HTMLResponse)
async def topology():
    return HTMLResponse(pages.topology_page())


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
    return HTMLResponse(
        pages.render_key_metrics(scan_processes(),
                                 scan_wal_streams()))


@app.get("/x/ring-pressure", response_class=HTMLResponse)
async def x_ring_pressure():
    return HTMLResponse(pages.render_ring_pressure())


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
    return HTMLResponse(pages.render_cmp_flows())


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
async def x_wal_timeline():
    all_records = []
    for stream_dir in _wal_stream_dirs():
        all_records.extend(parse_wal_records(stream_dir))
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
    procs = scan_processes()
    running = [p for p in procs
               if p["state"] == "running"]
    if not running:
        return HTMLResponse(
            '<span class="text-slate-500 text-xs">'
            'no processes running</span>')
    stats = parse_wal_book_stats()
    return HTMLResponse(
        pages.render_book_stats(stats))


@app.get("/x/live-fills", response_class=HTMLResponse)
@app.get("/x/fills", response_class=HTMLResponse)
async def x_fills():
    procs = scan_processes()
    running = [p for p in procs
               if p["state"] == "running"]
    if not running:
        return HTMLResponse(
            '<span class="text-slate-500 text-xs">'
            'no processes running</span>')
    fills = parse_wal_fills()
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
    return HTMLResponse(pages.render_reconciliation())


@app.get("/x/latency-regression",
         response_class=HTMLResponse)
async def x_latency_regression():
    return HTMLResponse(
        pages.render_latency_regression(order_latencies))


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
    return HTMLResponse(
        '<span class="text-emerald-400 text-xs">'
        '0 stale orders</span>')


@app.get("/x/book", response_class=HTMLResponse)
async def x_book(symbol_id: int = Query(10)):
    procs = scan_processes()
    running = [p for p in procs if p["state"] == "running"]
    bbo = parse_wal_bbo(symbol_id) if running else None
    return HTMLResponse(
        pages.render_book_ladder(symbol_id, bbo))


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
    return HTMLResponse(
        '<span class="text-slate-600">'
        f'user {risk_uid} — no data '
        '(postgres not connected or no rows)</span>')


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
                except ProcessLookupError:
                    result = {"status": f"{name} not running"}
            else:
                result = {"status": f"{name} not running"}
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'{result.get("status", "ok")}</span>')

    if action == "restart":
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
                        os.kill(
                            int(proc["pid"]), signal.SIGTERM)
                        await asyncio.sleep(0.5)
                    except ProcessLookupError:
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
                response = await asyncio.wait_for(
                    ws.receive(timeout=2.0), timeout=2.0,
                )
                latency_us = (time.perf_counter_ns() - start_ns) // 1000
                if response.type == aiohttp.WSMsgType.TEXT:
                    msg = json.loads(response.data)
                    return msg, None, latency_us
                return None, "unexpected ws message type", None
    except (ConnectionRefusedError, OSError):
        return None, "gateway not running", None
    except asyncio.TimeoutError:
        return None, "timeout waiting for response", None
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

    order_msg = {
        "type": "NewOrder",
        "symbol_id": symbol_id,
        "side": form.get("side", "buy"),
        "order_type": form.get("order_type", "limit"),
        "price": form.get("price", "0"),
        "qty": form.get("qty", "0"),
        "client_order_id": cid,
        "tif": form.get("tif", "GTC"),
        "reduce_only": form.get("reduce_only") == "on",
        "post_only": form.get("post_only") == "on",
    }

    order = {
        "cid": cid,
        "user_id": user_id,
        "symbol": str(order_msg["symbol_id"]),
        "side": order_msg["side"],
        "price": order_msg["price"],
        "qty": order_msg["qty"],
        "tif": order_msg["tif"],
        "reduce_only": order_msg["reduce_only"],
        "post_only": order_msg["post_only"],
        "status": "pending",
        "ts": datetime.now().strftime("%H:%M:%S"),
    }

    result = await send_order_to_gateway(order_msg, user_id)
    if result[1]:
        order["status"] = "error"
        order["error"] = result[1]
        recent_orders.append(order)
        _trim_recent_orders()
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'order {cid} queued ({result[1]})</span>')

    msg, _, latency_us = result
    if latency_us:
        order_latencies.append(latency_us)
        if len(order_latencies) > 1000:
            del order_latencies[:500]

    if msg and msg.get("type") == "OrderAccepted":
        order["status"] = "accepted"
        order["latency_us"] = latency_us
        recent_orders.append(order)
        _trim_recent_orders()
        return HTMLResponse(
            f'<span class="text-emerald-400 text-xs">'
            f'order {cid} accepted ({latency_us}us)</span>')
    elif msg and msg.get("type") == "OrderFailed":
        order["status"] = "rejected"
        order["reason"] = msg.get("reason", "unknown")
        recent_orders.append(order)
        _trim_recent_orders()
        return HTMLResponse(
            f'<span class="text-red-400 text-xs">'
            f'order {cid} rejected: {order["reason"]}</span>')
    else:
        order["status"] = "error"
        recent_orders.append(order)
        _trim_recent_orders()
        return HTMLResponse(
            f'<span class="text-amber-400 text-xs">'
            f'order {cid} unexpected response</span>')


@app.post("/api/verify/run")
async def api_verify_run():
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

    for inv in [
        "Fills precede ORDER_DONE (per order)",
        "Exactly-one completion per order",
        "FIFO within price level (time priority)",
        "Position = sum of fills (risk engine)",
        "Tips monotonic, never decrease",
        "No crossed book (bid < ask)",
        "SPSC preserves event FIFO order",
        "Slab no-leak: allocated = free + active",
        "Funding zero-sum across users per symbol",
        "Advisory lock exclusive: one main per shard",
    ]:
        if running:
            checks.append({
                "name": inv,
                "status": "skip",
                "time": now,
                "detail": "requires instrumentation",
            })
        else:
            checks.append({
                "name": inv,
                "status": "fail",
                "time": now,
                "detail": "system not running",
            })

    verify_results.clear()
    verify_results.extend(checks)
    return HTMLResponse(pages.render_verify(checks))


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


@app.post("/api/stress/run")
async def api_stress_run(
    request: Request,
    rate: int = Form(100),
    duration: int = Form(60),
):
    """Launch stress test and save results"""
    is_htmx = request.headers.get("hx-request") == "true"
    config = StressConfig(rate=rate, duration=duration, gateway_url=GATEWAY_URL)

    # Run stress test and wait for results
    try:
        results = await run_stress_test(config)
    except Exception as e:
        err = html.escape(str(e))
        if is_htmx:
            return HTMLResponse(
                f'<span class="text-red-400 text-xs">'
                f'error: {err}</span>')
        return JSONResponse(
            {"status": "error", "error": str(e)},
            status_code=502)

    # Check if gateway was unreachable
    if "error" in results:
        err = html.escape(results["error"])
        if is_htmx:
            return HTMLResponse(
                f'<span class="text-red-400 text-xs">'
                f'gateway unreachable: {err}</span>')
        return JSONResponse(
            {"status": "error", "error": results["error"]},
            status_code=502)

    # Save results with timestamp
    timestamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    report_file = STRESS_REPORTS_DIR / f"stress-{timestamp}.json"

    with open(report_file, "w") as f:
        json.dump({
            "timestamp": timestamp,
            "config": results["config"],
            "metrics": results["metrics"],
            "latency_us": results["latency_us"],
        }, f, indent=2)

    m = results["metrics"]
    lat = results["latency_us"]
    if is_htmx:
        return HTMLResponse(
            f'<div class="space-y-1 text-xs">'
            f'<span class="text-emerald-400">completed</span>'
            f' in {m["elapsed_sec"]}s'
            f'<div class="text-slate-400">'
            f'submitted={m["submitted"]} '
            f'accepted={m["accepted"]} '
            f'rate={m["actual_rate"]}/s</div>'
            f'<div class="text-slate-400">'
            f'p50={lat["p50"]}us p95={lat["p95"]}us '
            f'p99={lat["p99"]}us</div>'
            f'<a href="./stress/{timestamp}" '
            f'class="text-blue-400 hover:underline">'
            f'view full report</a></div>')

    return JSONResponse({
        "status": "completed",
        "report_id": timestamp,
        "results": results
    })



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
        '<table class="w-full text-left">'
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
async def api_deposit(user_id: int):
    return HTMLResponse(
        f'<span class="text-slate-500 text-xs">'
        f'deposit for user {user_id} requires '
        f'risk engine</span>')


@app.post("/api/risk/liquidate")
async def api_liquidate():
    return HTMLResponse(
        '<span class="text-slate-500 text-xs">'
        'liquidation requires risk engine</span>')


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
    return {"user_id": user_id, "action": action,
            "status": "requires risk engine"}


@app.get("/api/mark/prices")
async def api_mark_prices():
    return {"status": "requires mark process"}


@app.get("/api/metrics")
async def api_metrics():
    procs = scan_processes()
    return {
        "processes": len(procs),
        "running": len(
            [p for p in procs if p["state"] == "running"]),
        "postgres": pg_pool is not None,
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
    return {
        "running": running,
        "pid": pid,
        "name": MAKER_NAME,
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
    tmp = MAKER_CONFIG.with_suffix(".tmp")
    tmp.write_text(json.dumps({"mid_override": mid_override}))
    tmp.replace(MAKER_CONFIG)
    return {"ok": True}


@app.get("/api/book/{symbol_id}")
async def api_book(symbol_id: int):
    # Prefer live snapshot from marketdata WS
    snap = _book_snap.get(symbol_id)
    if snap:
        return snap
    # Fallback: WAL BBO (at most 1 bid + 1 ask)
    bbo = parse_wal_bbo(symbol_id)
    if bbo is None:
        return {"bids": [], "asks": []}
    bids = []
    asks = []
    if bbo["bid_px"] != 0:
        bids.append({"px": bbo["bid_px"], "qty": bbo["bid_qty"]})
    if bbo["ask_px"] != 0:
        asks.append({"px": bbo["ask_px"], "qty": bbo["ask_qty"]})
    return {"bids": bids, "asks": asks}


@app.get("/api/bbo/{symbol_id}")
async def api_bbo(symbol_id: int):
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


# ── trading UI: WS proxy + REST proxy + static ─────────


@app.websocket("/ws/private")
async def ws_private_proxy(ws: WebSocket):
    """Proxy private WS to Gateway."""
    await ws.accept()
    headers = {"x-user-id": ws.headers.get(
        "x-user-id", "1")}
    try:
        async with aiohttp.ClientSession() as session:
            async with session.ws_connect(
                GATEWAY_URL, headers=headers,
            ) as upstream:
                async def fwd_up():
                    try:
                        async for msg in upstream:
                            if msg.type == aiohttp.WSMsgType.TEXT:
                                await ws.send_text(msg.data)
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
                async def fwd_up():
                    try:
                        async for msg in upstream:
                            if msg.type == aiohttp.WSMsgType.TEXT:
                                await ws.send_text(msg.data)
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
    except (ConnectionRefusedError, OSError):
        await ws.close(code=1013,
                       reason="marketdata not running")


async def _probe_gateway_tcp() -> bool:
    """Return True if gateway TCP port is reachable."""
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


@app.get("/v1/symbols")
async def v1_symbols():
    """Return configured symbol catalog (local fallback)."""
    rows = []
    for name, cfg in start_mod.SYMBOLS.items():
        rows.append([cfg["id"], cfg["tick"], cfg["lot"], name])
    rows.sort(key=lambda r: r[0])
    return JSONResponse({"M": rows})


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
    # Base price: BTC ~95000, ETH ~3000, else 100
    name = sym.upper()
    if "BTC" in name:
        base = 95_000_000  # tick units (tick=0.1 → 9500000)
        base_raw = int(95_000 / tick)
    elif "ETH" in name:
        base_raw = int(3_000 / tick)
    else:
        base_raw = int(100 / tick)
    bars = []
    rng = random.Random(42)
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
            async with session.request(
                method, url,
                data=body if body else None,
                headers={
                    "content-type": request.headers.get(
                        "content-type", "application/json"),
                },
            ) as resp:
                data = await resp.read()
                return JSONResponse(
                    content=json.loads(data),
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
        reload=True,
    )
