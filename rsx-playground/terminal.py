"""PTY bridge for the Playground embedded rsx-term.

This is intentionally not a general shell. The only command launched is the
local Go trading terminal against the local Playground-managed gateway.
"""

from __future__ import annotations

import asyncio
import json
import os
import pty
import signal
import shutil
import struct
import subprocess
import termios
from pathlib import Path

from fastapi import WebSocket


DEFAULT_COLS = 120
DEFAULT_ROWS = 36


def _is_loopback(host: str | None) -> bool:
    return host in {"127.0.0.1", "::1", "localhost"}


async def authorized(ws: WebSocket, admin_token: str) -> bool:
    host = ws.client.host if ws.client else None
    if _is_loopback(host):
        return True
    if admin_token and ws.headers.get("x-admin-token", "") == admin_token:
        return True
    await ws.close(code=1008, reason="terminal requires loopback/admin")
    return False


async def serve_rsx_term(ws: WebSocket, root: Path) -> None:
    await ws.accept()

    term_dir = root / "rsx-term"
    if not term_dir.exists():
        await ws.send_text("\r\nrsx-term directory not found\r\n")
        await ws.close(code=1011)
        return

    master_fd, slave_fd = pty.openpty()
    _resize(master_fd, DEFAULT_COLS, DEFAULT_ROWS)

    env = os.environ.copy()
    env.update({
        "TERM": "xterm-256color",
        "COLORTERM": "truecolor",
        "RSX_GW_URL": "ws://127.0.0.1:8088",
        "RSX_MD_URL": "ws://127.0.0.1:8180",
    })

    try:
        proc = subprocess.Popen(
            [_go_bin(), "run", "."],
            cwd=str(term_dir),
            env=env,
            stdin=slave_fd,
            stdout=slave_fd,
            stderr=slave_fd,
            start_new_session=True,
            close_fds=True,
        )
    except FileNotFoundError:
        os.close(master_fd)
        os.close(slave_fd)
        await ws.send_text("\r\ngo not found; install Go to run rsx-term\r\n")
        await ws.close(code=1011)
        return

    os.close(slave_fd)

    async def pump_pty() -> None:
        while True:
            try:
                data = await asyncio.to_thread(os.read, master_fd, 4096)
            except OSError:
                break
            if not data:
                break
            await ws.send_text(data.decode("utf-8", errors="replace"))

    reader = asyncio.create_task(pump_pty())
    try:
        while True:
            raw = await ws.receive_text()
            try:
                msg = json.loads(raw)
            except json.JSONDecodeError:
                continue
            kind = msg.get("type")
            if kind == "input":
                data = str(msg.get("data", ""))
                if data:
                    await asyncio.to_thread(
                        os.write, master_fd, data.encode("utf-8")
                    )
            elif kind == "resize":
                cols = int(msg.get("cols", DEFAULT_COLS))
                rows = int(msg.get("rows", DEFAULT_ROWS))
                _resize(master_fd, cols, rows)
    except Exception:
        pass
    finally:
        reader.cancel()
        _terminate(proc)
        try:
            os.close(master_fd)
        except OSError:
            pass


def _resize(fd: int, cols: int, rows: int) -> None:
    cols = max(20, min(cols, 240))
    rows = max(8, min(rows, 120))
    size = struct.pack("HHHH", rows, cols, 0, 0)
    try:
        termios.tcsetwinsize(fd, (rows, cols))
    except AttributeError:
        import fcntl

        fcntl.ioctl(fd, termios.TIOCSWINSZ, size)
    except OSError:
        pass


def _go_bin() -> str:
    configured = os.environ.get("RSX_GO_BIN", "").strip()
    if configured:
        return configured
    found = shutil.which("go")
    if found:
        return found
    fallback = Path("/usr/local/go/bin/go")
    if fallback.exists():
        return str(fallback)
    return "go"


def _terminate(proc: subprocess.Popen) -> None:
    if proc.poll() is not None:
        return
    try:
        os.killpg(proc.pid, signal.SIGTERM)
        proc.wait(timeout=2)
    except Exception:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except Exception:
            pass
