"""Pytest configuration for rsx-playground tests."""

import asyncio
import base64
import json
import os
import signal
import socket
import struct
import subprocess
import sys
import time
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock, patch

import psutil
import pytest
from fastapi.testclient import TestClient

sys.path.insert(0, str(Path(__file__).parent.parent))

import server

class RawWsClient:
    """Minimal WebSocket client for testing.

    Bypasses Sec-WebSocket-Accept validation since the
    gateway's custom monoio handshake may compute it
    differently than aiohttp expects.
    """

    def __init__(self, reader, writer):
        self.reader = reader
        self.writer = writer

    @classmethod
    async def connect(cls, host, port, headers=None):
        """Open WS connection. Returns (client, status)."""
        reader, writer = await asyncio.open_connection(
            host, port,
        )
        key = base64.b64encode(os.urandom(16)).decode()
        req = (
            f"GET / HTTP/1.1\r\n"
            f"Host: {host}:{port}\r\n"
            f"Upgrade: websocket\r\n"
            f"Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            f"Sec-WebSocket-Version: 13\r\n"
        )
        if headers:
            for k, v in headers.items():
                req += f"{k}: {v}\r\n"
        req += "\r\n"

        writer.write(req.encode())
        await writer.drain()

        # Read HTTP response headers
        response = await asyncio.wait_for(
            reader.readuntil(b"\r\n\r\n"), timeout=5.0,
        )
        status_line = response.split(b"\r\n")[0].decode()
        status_code = int(status_line.split()[1])

        if status_code != 101:
            writer.close()
            raise ConnectionRefusedError(
                f"handshake failed: {status_code}"
            )

        return cls(reader, writer), status_code

    @classmethod
    async def try_connect(cls, host, port, headers=None):
        """Try to connect, return status code (no raise)."""
        try:
            reader, writer = await asyncio.open_connection(
                host, port,
            )
        except ConnectionRefusedError:
            return None

        key = base64.b64encode(os.urandom(16)).decode()
        req = (
            f"GET / HTTP/1.1\r\n"
            f"Host: {host}:{port}\r\n"
            f"Upgrade: websocket\r\n"
            f"Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            f"Sec-WebSocket-Version: 13\r\n"
        )
        if headers:
            for k, v in headers.items():
                req += f"{k}: {v}\r\n"
        req += "\r\n"

        writer.write(req.encode())
        await writer.drain()

        response = await asyncio.wait_for(
            reader.readuntil(b"\r\n\r\n"), timeout=5.0,
        )
        status_line = response.split(b"\r\n")[0].decode()
        status_code = int(status_line.split()[1])
        writer.close()
        return status_code

    async def send(self, text):
        """Send a masked text frame."""
        data = text.encode()
        mask = os.urandom(4)
        frame = bytearray()
        frame.append(0x81)  # FIN + text opcode
        length = len(data)
        if length <= 125:
            frame.append(0x80 | length)
        elif length <= 65535:
            frame.append(0x80 | 126)
            frame.extend(struct.pack(">H", length))
        else:
            frame.append(0x80 | 127)
            frame.extend(struct.pack(">Q", length))
        frame.extend(mask)
        masked = bytearray(
            b ^ mask[i % 4] for i, b in enumerate(data)
        )
        frame.extend(masked)
        self.writer.write(frame)
        await self.writer.drain()

    async def recv(self, timeout=2.0):
        """Receive a text frame. Returns decoded string."""
        header = await asyncio.wait_for(
            self.reader.readexactly(2), timeout,
        )
        opcode = header[0] & 0x0F
        masked = (header[1] & 0x80) != 0
        length = header[1] & 0x7F
        if length == 126:
            ext = await self.reader.readexactly(2)
            length = struct.unpack(">H", ext)[0]
        elif length == 127:
            ext = await self.reader.readexactly(8)
            length = struct.unpack(">Q", ext)[0]
        if masked:
            mask_key = await self.reader.readexactly(4)
        payload = b""
        if length > 0:
            payload = await self.reader.readexactly(length)
        if masked:
            payload = bytes(
                b ^ mask_key[i % 4]
                for i, b in enumerate(payload)
            )
        if opcode == 8:
            raise ConnectionError("connection closed")
        return payload.decode()

    async def recv_json(self, timeout=2.0):
        """Receive and parse JSON frame."""
        text = await self.recv(timeout)
        return json.loads(text)

    async def close(self):
        """Close the connection."""
        try:
            self.writer.close()
        except Exception:
            pass


# Project root (rsx/)
RSX_ROOT = Path(__file__).parent.parent.parent

# Gateway binary path
GW_BINARY = RSX_ROOT / "target" / "debug" / "rsx-gateway"

# Test ports (offset to avoid collisions)
GW_WS_PORT = 18080
GW_RISK_CMP_PORT = 19300
GW_CMP_PORT = 19200


def _port_open(port: int) -> bool:
    """Check if a TCP port is accepting connections."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.settimeout(0.1)
        return s.connect_ex(("127.0.0.1", port)) == 0


@pytest.fixture(scope="module")
def gateway():
    """Start rsx-gateway for integration tests.

    Starts the gateway binary with test-specific env vars
    and unique ports. Kills process on teardown.
    """
    if not GW_BINARY.exists():
        pytest.skip(f"gateway binary not found: {GW_BINARY}")

    env = {
        **os.environ,
        "RSX_GW_LISTEN": f"0.0.0.0:{GW_WS_PORT}",
        "RSX_GW_JWT_SECRET": "",
        "RSX_GW_IDLE_TIMEOUT_S": "60",
        "RSX_GW_ORDER_TIMEOUT_MS": "2000",
        "RSX_GW_MAX_PENDING": "10000",
        "RSX_GW_RL_USER": "10",
        "RSX_GW_RL_IP": "100",
        "RSX_GW_RL_INSTANCE": "1000",
        "RSX_GW_HEARTBEAT_INTERVAL_S": "5",
        "RSX_RISK_CMP_ADDR": f"127.0.0.1:{GW_RISK_CMP_PORT}",
        "RSX_GW_CMP_ADDR": f"127.0.0.1:{GW_CMP_PORT}",
        "RSX_GW_WAL_DIR": str(RSX_ROOT / "tmp" / "wal_test"),
        "RSX_MAX_SYMBOLS": "16",
        "RSX_DEFAULT_TICK_SIZE": "1",
        "RSX_DEFAULT_LOT_SIZE": "1",
        "RUST_LOG": "info",
    }

    # Ensure WAL dir exists
    wal_dir = RSX_ROOT / "tmp" / "wal_test"
    wal_dir.mkdir(parents=True, exist_ok=True)

    proc = subprocess.Popen(
        [str(GW_BINARY)],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    # Wait for WS port to accept connections
    deadline = time.time() + 5.0
    while time.time() < deadline:
        if proc.poll() is not None:
            _, stderr_bytes = proc.communicate(timeout=5)
            stderr = stderr_bytes.decode()
            pytest.fail(
                f"gateway exited early (rc={proc.returncode})"
                f": {stderr[:500]}"
            )
        if _port_open(GW_WS_PORT):
            break
        time.sleep(0.1)
    else:
        proc.kill()
        proc.wait()
        pytest.fail("gateway did not start within 5s")

    yield {
        "ws_url": f"ws://127.0.0.1:{GW_WS_PORT}",
        "proc": proc,
    }

    # Teardown
    proc.send_signal(signal.SIGTERM)
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()


@pytest.fixture(scope="module")
def gateway_small_pending():
    """Gateway with max_pending=1 for pending queue tests."""
    if not GW_BINARY.exists():
        pytest.skip(f"gateway binary not found: {GW_BINARY}")

    ws_port = GW_WS_PORT + 1
    risk_port = GW_RISK_CMP_PORT + 1
    cmp_port = GW_CMP_PORT + 1

    env = {
        **os.environ,
        "RSX_GW_LISTEN": f"0.0.0.0:{ws_port}",
        "RSX_GW_JWT_SECRET": "",
        "RSX_GW_IDLE_TIMEOUT_S": "60",
        "RSX_GW_ORDER_TIMEOUT_MS": "30000",
        "RSX_GW_MAX_PENDING": "1",
        "RSX_GW_RL_USER": "100",
        "RSX_GW_RL_IP": "1000",
        "RSX_GW_RL_INSTANCE": "10000",
        "RSX_GW_HEARTBEAT_INTERVAL_S": "60",
        "RSX_RISK_CMP_ADDR": f"127.0.0.1:{risk_port}",
        "RSX_GW_CMP_ADDR": f"127.0.0.1:{cmp_port}",
        "RSX_GW_WAL_DIR": str(
            RSX_ROOT / "tmp" / "wal_test_pending"
        ),
        "RSX_MAX_SYMBOLS": "16",
        "RSX_DEFAULT_TICK_SIZE": "1",
        "RSX_DEFAULT_LOT_SIZE": "1",
        "RUST_LOG": "info",
    }

    wal_dir = RSX_ROOT / "tmp" / "wal_test_pending"
    wal_dir.mkdir(parents=True, exist_ok=True)

    proc = subprocess.Popen(
        [str(GW_BINARY)],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    deadline = time.time() + 5.0
    while time.time() < deadline:
        if proc.poll() is not None:
            _, stderr_bytes = proc.communicate(timeout=5)
            stderr = stderr_bytes.decode()
            pytest.fail(
                f"gateway exited early (rc={proc.returncode})"
                f": {stderr[:500]}"
            )
        if _port_open(ws_port):
            break
        time.sleep(0.1)
    else:
        proc.kill()
        proc.wait()
        pytest.fail("gateway did not start within 5s")

    yield {
        "ws_url": f"ws://127.0.0.1:{ws_port}",
        "proc": proc,
    }

    proc.send_signal(signal.SIGTERM)
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()


@pytest.fixture
def client():
    """Create TestClient for server app."""
    return TestClient(server.app)


@pytest.fixture(autouse=True)
def cleanup_state():
    """Clear in-memory state before and after each test."""
    # Before test: clear state
    server.recent_orders.clear()
    server.verify_results.clear()
    server.managed.clear()

    yield

    # After test: cleanup everything
    server.recent_orders.clear()
    server.verify_results.clear()

    # Kill any managed processes
    for name, info in list(server.managed.items()):
        proc = info.get("proc")
        if proc and hasattr(proc, "pid"):
            try:
                os.kill(proc.pid, signal.SIGTERM)
            except (ProcessLookupError, OSError):
                continue
            try:
                os.kill(proc.pid, signal.SIGKILL)
            except (ProcessLookupError, OSError):
                pass
    server.managed.clear()


@pytest.fixture(scope="session", autouse=True)
def cleanup_session():
    """Cleanup at session start and end."""
    # Before session: kill any stray RSX processes
    _kill_stray_processes()

    yield

    # After session: full cleanup
    _kill_stray_processes()
    _cleanup_test_files()


def _kill_stray_processes():
    """Kill any stray RSX processes from previous runs."""
    rsx_binaries = [
        "rsx-matching", "rsx-gateway", "rsx-risk",
        "rsx-marketdata", "rsx-mark", "rsx-recorder"
    ]

    for proc in psutil.process_iter(['pid', 'name', 'cmdline']):
        try:
            cmdline = proc.info.get('cmdline') or []
            cmdline_str = ' '.join(cmdline)

            if any(binary in cmdline_str for binary in rsx_binaries):
                print(f"Killing stray process: {proc.pid} {cmdline_str}")
                proc.kill()
                proc.wait(timeout=5)
        except (psutil.NoSuchProcess, psutil.AccessDenied, psutil.TimeoutExpired):
            pass


def _cleanup_test_files():
    """Clean up test-generated files."""
    import shutil

    root = Path(__file__).parent.parent.parent

    # Clean WAL files in tmp/wal/
    wal_dir = root / "tmp" / "wal"
    if wal_dir.exists():
        for item in wal_dir.iterdir():
            if item.is_dir():
                shutil.rmtree(item, ignore_errors=True)

    # Clean PID files in tmp/pids/
    pid_dir = root / "tmp" / "pids"
    if pid_dir.exists():
        for pid_file in pid_dir.glob("*.pid"):
            pid_file.unlink(missing_ok=True)

    # Clean test log files (keep production logs)
    log_dir = root / "log"
    if log_dir.exists():
        for log_file in log_dir.glob("*.log"):
            # Only clean if file is recent (test-generated)
            if log_file.stat().st_mtime > (os.path.getmtime(log_dir) - 3600):
                log_file.unlink(missing_ok=True)


@pytest.fixture
def mock_postgres_down():
    """Mock postgres as unavailable."""
    original_pool = server.pg_pool
    server.pg_pool = None
    yield
    server.pg_pool = original_pool


@pytest.fixture
def mock_postgres_connected():
    """Mock postgres as connected with a pool."""
    original_pool = server.pg_pool
    server.pg_pool = MagicMock()
    yield
    server.pg_pool = original_pool


@pytest.fixture
async def mock_pg_query_success():
    """Mock successful postgres query."""
    async def _query(*args):
        return [{"user_id": 1, "balance": 10000}]

    with patch.object(server, 'pg_query', side_effect=_query):
        yield


@pytest.fixture
async def mock_pg_query_error():
    """Mock postgres query error."""
    async def _query(*args):
        return {"error": "connection failed"}

    with patch.object(server, 'pg_query', side_effect=_query):
        yield


@pytest.fixture
def running_process():
    """Mock a running process in managed dict."""
    proc = AsyncMock()
    proc.pid = 12345
    proc.returncode = None

    server.managed["test-process"] = {
        "proc": proc,
        "binary": "./target/debug/test-binary",
        "env": {"TEST_ENV": "1"},
    }

    yield proc

    if "test-process" in server.managed:
        del server.managed["test-process"]


@pytest.fixture
def wal_dir_with_files(tmp_path):
    """Create temporary WAL directory with test files."""
    wal_dir = tmp_path / "wal"
    wal_dir.mkdir()

    stream_dir = wal_dir / "test-stream"
    stream_dir.mkdir()

    (stream_dir / "000001.dxs").write_bytes(b"test data" * 100)
    (stream_dir / "000002.dxs").write_bytes(b"test data" * 50)

    original_wal = server.WAL_DIR
    server.WAL_DIR = wal_dir

    yield wal_dir

    server.WAL_DIR = original_wal


@pytest.fixture
def log_dir_with_files(tmp_path):
    """Create temporary log directory with test files."""
    log_dir = tmp_path / "log"
    log_dir.mkdir()

    (log_dir / "gateway.log").write_text(
        "INFO: gateway started\n"
        "ERROR: connection failed\n"
        "WARN: retry attempt 1\n"
    )

    (log_dir / "risk.log").write_text(
        "INFO: risk engine ready\n"
        "DEBUG: position updated\n"
    )

    original_log = server.LOG_DIR
    server.LOG_DIR = log_dir

    yield log_dir

    server.LOG_DIR = original_log
