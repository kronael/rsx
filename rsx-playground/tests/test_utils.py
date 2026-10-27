"""Test utilities for RSX Playground API tests."""

import json
import os
import struct
import subprocess
import time
from datetime import datetime
from pathlib import Path

import rsx_cli

WAL_HEADER = struct.Struct("<BBHHHI4s")
CRC32C_POLY = 0x82F63B78
FIXTURE_RECORD_TYPE = 0xFE
FIXTURE_FRAME_BYTES = 32


def mint_token(user_id: int, secret: str | None = None, ttl_s: int = 3600) -> str:
    """A gateway JWT from `rsx-cli token mint`, signed with `secret` or else
    the RSX_GW_JWT_SECRET of the environment."""
    env = os.environ.copy()
    if secret is not None:
        env["RSX_GW_JWT_SECRET"] = secret
    done = subprocess.run(
        [str(rsx_cli.BINARY), "token", "mint", "-u", str(user_id),
         "-t", str(ttl_s), "--json"],
        env=env, capture_output=True, text=True, check=True, timeout=10,
    )
    return json.loads(done.stdout)["token"]


def create_test_order(
    cid=None,
    symbol="10",
    side="buy",
    price="50000",
    qty="1",
    tif="GTC",
    reduce_only=False,
    post_only=False,
    status="submitted",
):
    """Generate a test order dict."""
    if cid is None:
        cid = f"test{int(time.time()*1e6):018d}"[:20]

    return {
        "cid": cid,
        "symbol": symbol,
        "side": side,
        "price": price,
        "qty": qty,
        "tif": tif,
        "reduce_only": reduce_only,
        "post_only": post_only,
        "status": status,
        "ts": datetime.now().strftime("%H:%M:%S"),
    }


def assert_order_structure(order):
    """Validate order response structure."""
    required_fields = [
        "cid", "symbol", "side", "price", "qty", "status"
    ]
    for field in required_fields:
        assert field in order, f"missing field: {field}"


def assert_process_structure(proc):
    """Validate process response structure."""
    required_fields = [
        "name", "pid", "state", "cpu", "mem", "uptime"
    ]
    for field in required_fields:
        assert field in proc, f"missing field: {field}"

    assert proc["state"] in ["running", "stopped"]


def assert_wal_stream_structure(stream):
    """Validate WAL stream response structure."""
    required_fields = ["name", "files", "total_size", "newest"]
    for field in required_fields:
        assert field in stream, f"missing field: {field}"

    assert isinstance(stream["files"], int)
    assert stream["files"] >= 0


def create_mock_process_info(
    name="test-proc",
    pid=12345,
    state="running",
    cpu="2.5%",
    mem="128.0MB",
    uptime="5m30s",
):
    """Create a mock process info dict."""
    return {
        "name": name,
        "pid": pid,
        "state": state,
        "cpu": cpu,
        "mem": mem,
        "uptime": uptime,
    }


def create_mock_wal_stream(
    name="test-stream",
    files=5,
    total_size="1.2MB",
    newest="10:30:15",
):
    """Create a mock WAL stream info dict."""
    return {
        "name": name,
        "files": files,
        "total_size": total_size,
        "newest": newest,
    }


def build_crc32c_table():
    table = []
    for byte in range(256):
        crc = byte
        for _ in range(8):
            crc = (crc >> 1) ^ CRC32C_POLY if crc & 1 else crc >> 1
        table.append(crc)
    return table


CRC32C_TABLE = build_crc32c_table()


def crc32c(data):
    crc = 0xFFFFFFFF
    for byte in data:
        crc = CRC32C_TABLE[(crc ^ byte) & 0xFF] ^ (crc >> 8)
    return crc ^ 0xFFFFFFFF


def wal_frame(record_type, payload, length=None):
    """One WAL record: a version-1 header carrying the payload's CRC32C."""
    if length is not None:
        payload = payload.ljust(length, b"\0")
    header = WAL_HEADER.pack(
        1, 0, record_type, len(payload), 0, crc32c(payload), b"\0" * 4)
    return header + payload


def seq_frame(seq):
    """A 32-byte record of an unknown type: seq, ts_ns."""
    payload = struct.pack("<QQ", seq, seq * 1000)
    return wal_frame(FIXTURE_RECORD_TYPE, payload)


def write_wal_stream(wal_dir, name, frames, stream_id=1,
                     segment="active"):
    """Write `frames` as one segment of stream `stream_id` under
    `wal_dir/name`, the layout rsx-cli reads: <name>/<id>/<id>_<segment>.wal.
    `segment` is "active" or "<first>_<last>"."""
    stream = Path(wal_dir) / name / str(stream_id)
    stream.mkdir(parents=True, exist_ok=True)
    path = stream / f"{stream_id}_{segment}.wal"
    path.write_bytes(b"".join(frames))
    return path
