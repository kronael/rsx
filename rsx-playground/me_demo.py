"""Duplicate order id demo against a scratch matching engine.

The gateway mints a fresh order id for every order, so no client can make
the live engine see an id twice. This module starts the real rsx-matching
binary on its own ports, symbol and WAL directory, casts the same
OrderMessage to it twice the way risk does, and leaves the engine's WAL
for the caller to decode. The live engines are never touched.
"""

import json
import socket
import struct
import subprocess
import time
import urllib.request
from pathlib import Path

RECORD_ORDER_REQUEST = 9
SCRATCH_SYMBOL_ID = 999
READY_TIMEOUT_S = 15.0
DUPLICATE_TIMEOUT_S = 10.0
ORDER_PAYLOAD_BYTES = 128

WAL_HEADER = struct.Struct("<BBHHHI4s")
ORDER_MESSAGE = struct.Struct("<QqqBBBB4sIIQQQQ")

CRC32C_POLY = 0x82F63B78


def _build_crc32c_table():
    table = []
    for byte in range(256):
        crc = byte
        for _ in range(8):
            crc = (crc >> 1) ^ CRC32C_POLY if crc & 1 else crc >> 1
        table.append(crc)
    return table


CRC32C_TABLE = _build_crc32c_table()


def crc32c(data: bytes) -> int:
    crc = 0xFFFFFFFF
    for byte in data:
        crc = CRC32C_TABLE[(crc ^ byte) & 0xFF] ^ (crc >> 8)
    return crc ^ 0xFFFFFFFF


def build_order_frame(
    seq: int,
    user_id: int,
    order_id_lo: int,
    side: int,
    price: int,
    qty: int,
) -> bytes:
    """One RECORD_ORDER_REQUEST casting frame: 16-byte WalHeader (version
    1, CRC32C of the payload) + the 128-byte OrderMessage risk sends."""
    payload = ORDER_MESSAGE.pack(
        seq, price, qty, side, 0, 0, 0, b"\0" * 4,
        user_id, 0, time.time_ns(), 0, order_id_lo, time.time_ns(),
    ).ljust(ORDER_PAYLOAD_BYTES, b"\0")
    header = WAL_HEADER.pack(
        1, 0, RECORD_ORDER_REQUEST, len(payload), 0,
        crc32c(payload), b"\0" * 4,
    )
    return header + payload


def _free_port(kind: int) -> int:
    with socket.socket(socket.AF_INET, kind) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _fetch_metrics(health_addr: str) -> dict[str, int]:
    url = f"http://{health_addr}/metrics"
    with urllib.request.urlopen(url, timeout=0.5) as resp:
        data = json.loads(resp.read().decode("utf-8"))
    return {c["name"]: int(c["value"]) for c in data["counters"]}


def _is_ready(health_addr: str) -> bool:
    url = f"http://{health_addr}/ready"
    with urllib.request.urlopen(url, timeout=0.5) as resp:
        return resp.status == 200


def _wait_until(deadline_s: float, is_done, what: str, log: Path) -> None:
    deadline = time.monotonic() + deadline_s
    while time.monotonic() < deadline:
        try:
            if is_done():
                return
        except (OSError, ValueError, KeyError):
            pass
        time.sleep(0.05)
    tail = log.read_text(errors="replace")[-2000:] if log.exists() else ""
    raise RuntimeError(f"scratch matching engine: {what}\n{tail}")


def run_duplicate_demo(binary: Path, workdir: Path, order_id_lo: int) -> Path:
    """Cast the same order twice to a scratch ME and return its WAL dir.

    Raises RuntimeError with the engine's log tail when the engine does
    not become ready or does not reject the second copy in time."""
    if not binary.exists():
        raise FileNotFoundError(f"binary not found: {binary}")
    workdir.mkdir(parents=True, exist_ok=True)
    wal_dir = workdir / "wal"
    log = workdir / "me.log"
    sinks = [
        socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        for _ in range(3)
    ]
    try:
        for sink in sinks:
            sink.bind(("127.0.0.1", 0))
        nak_addr, risk_addr, md_addr = (
            f"127.0.0.1:{sink.getsockname()[1]}" for sink in sinks
        )
        me_port = _free_port(socket.SOCK_DGRAM)
        health_addr = f"127.0.0.1:{_free_port(socket.SOCK_STREAM)}"
        env = {
            "RSX_ME_SYMBOL_ID": str(SCRATCH_SYMBOL_ID),
            "RSX_ME_PRICE_DECIMALS": "0",
            "RSX_ME_QTY_DECIMALS": "0",
            "RSX_ME_TICK_SIZE": "1",
            "RSX_ME_LOT_SIZE": "1",
            "RSX_ME_BOOK_CAPACITY": "4096",
            "RSX_ME_WAL_DIR": str(wal_dir),
            "RSX_ME_CAST_ADDR": f"127.0.0.1:{me_port}",
            "RSX_RISK_ME_SEND_ADDR": nak_addr,
            "RSX_RISK_ME_RECV_ADDR": risk_addr,
            "RSX_MD_CAST_ADDR": md_addr,
            "RSX_ME_HEALTH_ADDR": health_addr,
        }
        with log.open("wb") as log_file:
            engine = subprocess.Popen(
                [str(binary)], env=env,
                stdout=log_file, stderr=subprocess.STDOUT,
            )
        try:
            _wait_until(
                READY_TIMEOUT_S, lambda: _is_ready(health_addr),
                "not ready", log)
            sender = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            with sender:
                for seq in 1, 2:
                    sender.sendto(
                        build_order_frame(
                            seq, user_id=1, order_id_lo=order_id_lo,
                            side=1, price=100, qty=1),
                        ("127.0.0.1", me_port))
            _wait_until(
                DUPLICATE_TIMEOUT_S,
                lambda: _fetch_metrics(health_addr)
                .get("rejects_duplicate", 0) >= 1,
                "did not reject the second copy", log)
        finally:
            engine.terminate()
            try:
                engine.wait(timeout=5)
            except subprocess.TimeoutExpired:
                engine.kill()
                engine.wait()
    finally:
        for sink in sinks:
            sink.close()
    return wal_dir
