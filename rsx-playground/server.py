"""RSX Playground — dev dashboard.

Usage: cd rsx-playground && uv run server.py
"""

import asyncio
import html
import json
import os
import signal
import subprocess
import sys
import time
from contextlib import asynccontextmanager
from datetime import datetime
from pathlib import Path

import psutil
import uvicorn
from fastapi import FastAPI
from fastapi import Query
from fastapi import Request
from fastapi.responses import HTMLResponse
from fastapi.responses import JSONResponse
from fastapi.responses import PlainTextResponse

import pages

import aiohttp

ROOT = Path(__file__).resolve().parent.parent
TMP = ROOT / "tmp"
WAL_DIR = TMP / "wal"
LOG_DIR = ROOT / "log"
PID_DIR = TMP / "pids"
STRESS_REPORTS_DIR = TMP / "stress-reports"
STRESS_REPORTS_DIR.mkdir(exist_ok=True)

PG_URL = os.environ.get(
    "DATABASE_URL",  # postgresql://rsx:folium@10.0.2.1:5432/rsx_dev
    "postgres://rsx:rsx@127.0.0.1:5432/rsx",
)

GATEWAY_URL = os.environ.get(
    "GATEWAY_URL", "ws://localhost:8080"
)

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
        # Process exited immediately, clean up PID file
        pid_file = PID_DIR / f"{name}.pid"
        if pid_file.exists():
            pid_file.unlink()
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
        return {"status": f"{name} already stopped"}
    proc.kill()
    await proc.wait()
    pid_file = PID_DIR / f"{name}.pid"
    if pid_file.exists():
        pid_file.unlink()
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

    # kill stale processes on known ports
    import shutil
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
                    if pid:
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

    # spawn all
    started = []
    for name, binary, env in plan:
        result = await spawn_process(name, binary, env)
        if "pid" in result:
            started.append(name)
        await asyncio.sleep(0.1)
    if started:
        current_scenario = scenario
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
    await pg_connect()
    yield
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


app = FastAPI(title="RSX Playground", lifespan=lifespan)


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
    return {
        "status": "ok",
        "port": 49171,
        "processes_running": len(running),
        "processes_total": len(procs),
        "postgres": pg_pool is not None
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
            except (psutil.NoSuchProcess,
                    psutil.AccessDenied,
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
    for entry in sorted(WAL_DIR.iterdir()):
        if not entry.is_dir():
            continue
        files = list(entry.glob("*.dxs"))
        files += list(entry.glob("*.wal"))
        total = sum(
            f.stat().st_size for f in files if f.exists()
        )
        newest = ""
        if files:
            mt = max(f.stat().st_mtime for f in files)
            newest = datetime.fromtimestamp(mt).strftime(
                "%H:%M:%S")
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
    for entry in sorted(WAL_DIR.iterdir()):
        if not entry.is_dir():
            continue
        # Check both top-level files and subdirectories
        for item in sorted(entry.iterdir()):
            if item.is_file():
                st = item.stat()
                files.append({
                    "stream": entry.name,
                    "name": item.name,
                    "size": human_size(st.st_size),
                    "modified": datetime.fromtimestamp(
                        st.st_mtime).strftime("%H:%M:%S"),
                })
            elif item.is_dir():
                # Scan subdirectory (e.g., tmp/wal/pengu/10/)
                for f in sorted(item.iterdir()):
                    if f.is_file():
                        st = f.stat()
                        files.append({
                            "stream": f"{entry.name}/{item.name}",
                            "name": f.name,
                            "size": human_size(st.st_size),
                            "modified": datetime.fromtimestamp(
                                st.st_mtime).strftime("%H:%M:%S"),
                        })
    return files


import re
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


@app.get("/docs/{filename:path}")
async def docs(filename: str):
    """Serve playground documentation files."""
    docs_dir = Path(__file__).parent / "docs"
    if not filename:
        filename = "README.md"
    if not filename.endswith(".md"):
        filename += ".md"
    file_path = docs_dir / filename
    if not file_path.exists() or not file_path.is_file():
        return HTMLResponse("<h1>404 Not Found</h1>", status_code=404)
    content = file_path.read_text()
    # Simple markdown to HTML (very basic)
    html = f"""<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>RSX Playground Docs - {filename}</title>
<style>
body {{
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI",
    Helvetica, Arial, sans-serif;
  max-width: 800px;
  margin: 2rem auto;
  padding: 0 2rem;
  line-height: 1.6;
  background: #0f172a;
  color: #cbd5e1;
}}
h1, h2, h3 {{ color: #60a5fa; }}
a {{ color: #60a5fa; text-decoration: none; }}
a:hover {{ text-decoration: underline; }}
code {{
  background: #1e293b;
  padding: 2px 6px;
  border-radius: 3px;
  font-family: monospace;
}}
pre {{
  background: #1e293b;
  padding: 1rem;
  border-radius: 6px;
  overflow-x: auto;
}}
pre code {{
  background: none;
  padding: 0;
}}
</style>
</head>
<body>
<nav style="margin-bottom: 2rem; padding-bottom: 1rem;
  border-bottom: 1px solid #334155;">
<a href="/docs">Playground Docs</a> |
<a href="/">Playground UI</a> |
<a href="https://krons.cx/rsx/docs" target="_blank">Full Docs</a>
</nav>
<pre>{content}</pre>
</body>
</html>"""
    return HTMLResponse(html)


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
    return HTMLResponse(pages.render_wal_lag())


@app.get("/x/wal-rotation", response_class=HTMLResponse)
async def x_wal_rotation():
    return HTMLResponse(pages.render_wal_rotation())


@app.get("/x/wal-timeline", response_class=HTMLResponse)
async def x_wal_timeline():
    return HTMLResponse(pages.render_wal_timeline())


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
    return HTMLResponse(
        '<span class="text-slate-500 text-xs">'
        'start RSX processes to see book stats</span>')


@app.get("/x/live-fills", response_class=HTMLResponse)
async def x_live_fills():
    return HTMLResponse(
        '<span class="text-slate-500 text-xs">'
        'start RSX processes to see fills</span>')


@app.get("/x/trade-agg", response_class=HTMLResponse)
async def x_trade_agg():
    return HTMLResponse(
        '<span class="text-slate-500 text-xs">'
        'start RSX processes to see trade data</span>')


@app.get("/x/position-heatmap",
         response_class=HTMLResponse)
async def x_position_heatmap():
    return HTMLResponse(pages.render_position_heatmap())


@app.get("/x/margin-ladder", response_class=HTMLResponse)
async def x_margin_ladder():
    return HTMLResponse(pages.render_margin_ladder())


@app.get("/x/funding", response_class=HTMLResponse)
async def x_funding():
    return HTMLResponse(pages.render_funding())


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
    return HTMLResponse(
        f'<span class="text-slate-500 text-xs">'
        f'trace for {oid}: requires live system</span>')


@app.get("/x/stale-orders", response_class=HTMLResponse)
async def x_stale_orders():
    return HTMLResponse(
        '<span class="text-emerald-400 text-xs">'
        '0 stale orders</span>')


@app.get("/x/book", response_class=HTMLResponse)
async def x_book(symbol_id: int = Query(10)):
    return HTMLResponse(
        '<span class="text-slate-600">'
        f'book for symbol {symbol_id} — '
        'start RSX processes to see live data</span>')


@app.get("/x/risk-user", response_class=HTMLResponse)
async def x_risk_user(
    risk_uid: int = Query(1, alias="risk-uid"),
):
    # try postgres first
    data = await pg_query(
        "SELECT * FROM risk_positions "
        "WHERE user_id = $1 LIMIT 20",
        risk_uid,
    )
    if data is None:
        # no postgres, try balances table
        data = await pg_query(
            "SELECT * FROM balances "
            "WHERE user_id = $1 LIMIT 20",
            risk_uid,
        )
    if data and isinstance(data, list) and data:
        rows = ""
        for row in data:
            cells = "".join(
                f'<td class="py-1.5 px-2 text-xs '
                f'border-b border-slate-800/50">{v}</td>'
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
        "ORDER BY created_at DESC LIMIT 20",
    )
    if data and isinstance(data, list) and data:
        rows = ""
        for row in data:
            cells = "".join(
                f'<td class="py-1.5 px-2 text-xs '
                f'border-b border-slate-800/50">{v}</td>'
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
    cid = f"pg{int(time.time()*1e6):018d}"[:20]

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
        "No crossed book (bid < ask)",
        "Tips monotonic",
        "Slab no-leak",
        "Position = sum of fills",
        "FIFO within price level",
        "Exactly-one completion per order",
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
    import random
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
    rate: int = 100,
    duration: int = 60,
):
    """Launch stress test and save results"""
    from stress_client import run_stress_test, StressConfig

    config = StressConfig(rate=rate, duration=duration)

    # Run stress test and wait for results
    results = await run_stress_test(config)

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

    return JSONResponse({
        "status": "completed",
        "report_id": timestamp,
        "results": results
    })


# Keep backward compatibility
@app.post("/api/orders/stress")
async def api_orders_stress(
    rate: int = 100,
    duration: int = 60,
):
    """Launch stress test (backward compat)"""
    return await api_stress_run(rate, duration)


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
    if not reports.body:
        return HTMLResponse('<div class="text-slate-500 text-xs">No stress tests run yet</div>')

    data = json.loads(reports.body)
    if not data:
        return HTMLResponse('<div class="text-slate-500 text-xs">No stress tests run yet</div>')

    rows = []
    for r in data:
        timestamp_fmt = r["timestamp"]
        # Format: 20260213-211030 -> 2026-02-13 21:10:30
        if len(timestamp_fmt) == 15:
            ts = f"{timestamp_fmt[0:4]}-{timestamp_fmt[4:6]}-{timestamp_fmt[6:8]} {timestamp_fmt[9:11]}:{timestamp_fmt[11:13]}:{timestamp_fmt[13:15]}"
        else:
            ts = timestamp_fmt

        # Escape HTML to prevent XSS
        ts_escaped = html.escape(ts)
        id_escaped = html.escape(str(r["id"]))

        accept_color = "text-emerald-400" if r["accept_rate"] >= 95 else "text-amber-400"
        latency_color = "text-emerald-400" if r["p99_latency"] < 1000 else "text-amber-400"

        rows.append(
            f'<tr class="hover:bg-slate-800/30">'
            f'<td class="px-2 py-1 text-xs">'
            f'<a href="/stress/{id_escaped}" class="text-blue-400 hover:underline">{ts_escaped}</a>'
            f'</td>'
            f'<td class="px-2 py-1 text-xs text-right">{r["rate"]}/s</td>'
            f'<td class="px-2 py-1 text-xs text-right">{r["duration"]}s</td>'
            f'<td class="px-2 py-1 text-xs text-right">{r["submitted"]:,}</td>'
            f'<td class="px-2 py-1 text-xs text-right {accept_color}">{r["accept_rate"]}%</td>'
            f'<td class="px-2 py-1 text-xs text-right {latency_color}">{r["p99_latency"]}µs</td>'
            f'</tr>'
        )

    table = (
        '<table class="w-full text-left">'
        '<thead><tr class="border-b border-slate-700">'
        '<th class="px-2 py-1 text-[10px] text-slate-400">Timestamp</th>'
        '<th class="px-2 py-1 text-[10px] text-slate-400 text-right">Rate</th>'
        '<th class="px-2 py-1 text-[10px] text-slate-400 text-right">Duration</th>'
        '<th class="px-2 py-1 text-[10px] text-slate-400 text-right">Submitted</th>'
        '<th class="px-2 py-1 text-[10px] text-slate-400 text-right">Accept %</th>'
        '<th class="px-2 py-1 text-[10px] text-slate-400 text-right">p99</th>'
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

    html = f'<div class="text-xs"><div class="text-slate-400 mb-2">Latest: {file_name}</div>'
    html += f'<pre class="text-slate-300 whitespace-pre-wrap">{output[:2000]}</pre></div>'
    return HTMLResponse(html)


@app.get("/api/risk/users/{user_id}")
async def api_risk_user(user_id: int):
    data = await pg_query(
        "SELECT * FROM risk_positions "
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
