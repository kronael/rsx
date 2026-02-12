"""Test utilities for RSX Playground API tests."""

import time
from datetime import datetime


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


def assert_verify_result_structure(result):
    """Validate verify result response structure."""
    required_fields = ["name", "status", "time"]
    for field in required_fields:
        assert field in result, f"missing field: {field}"

    assert result["status"] in ["pass", "fail", "warn", "skip"]


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
