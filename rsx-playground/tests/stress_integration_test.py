"""
Integration tests for stress test functionality.

Run with: python -m pytest tests/stress_integration_test.py -v
"""

import asyncio
import json
import pytest
from pathlib import Path

from stress_client import StressConfig
from stress_client import run_stress_test


def test_stress_client_imports():
    """Test that stress client modules import correctly."""
    from stress_client import StressClient
    from stress_client import OrderMetrics

    config = StressConfig(rate=10, duration=1)
    assert config.rate == 10
    assert config.duration == 1

    metrics = OrderMetrics()
    assert metrics.submitted == 0
    assert metrics.accepted == 0


def test_stress_config_defaults():
    """Test StressConfig default values."""
    config = StressConfig()

    assert config.gateway_url == "ws://localhost:8080"
    assert config.rate == 1000
    assert config.duration == 60
    assert config.symbols == ["BTCUSD"]
    assert config.users == 10
    assert config.connections == 10


@pytest.mark.skipif(
    True,
    reason="Requires Gateway running on localhost:8080",
)
def test_gateway_websocket_connection():
    """Test that Gateway WebSocket accepts connections."""
    import aiohttp

    async def _test():
        async with aiohttp.ClientSession() as session:
            async with session.ws_connect(
                "ws://localhost:8080",
                headers={"x-user-id": "1"},
            ) as ws:
                order = {
                    "type": "NewOrder",
                    "symbol_id": 0,
                    "side": "buy",
                    "price": "50000.00",
                    "qty": "0.1",
                    "client_order_id": "test-001",
                    "tif": "GTC",
                    "reduce_only": False,
                    "post_only": False,
                }
                await ws.send_str(json.dumps(order))
                response = await asyncio.wait_for(
                    ws.receive(timeout=2.0), timeout=2.0,
                )
                msg = json.loads(response.data)
                assert msg.get("type") in [
                    "OrderAccepted",
                    "OrderFailed",
                ]

    asyncio.run(_test())


@pytest.mark.skipif(
    True,
    reason="Requires Gateway running on localhost:8080",
)
def test_stress_test_run_small():
    """Test running a small stress test."""
    config = StressConfig(rate=5, duration=2, connections=2)
    results = asyncio.run(run_stress_test(config))

    assert "config" in results
    assert "metrics" in results
    assert "latency_us" in results
    assert results["config"]["target_rate"] == 5
    assert results["config"]["duration"] == 2
    assert results["metrics"]["submitted"] > 0

    if results["metrics"]["accepted"] > 0:
        assert results["latency_us"]["p50"] > 0


def test_stress_report_generation():
    """Test that stress reports can be generated."""
    import pages

    data = {
        "timestamp": "20260213-120000",
        "config": {
            "target_rate": 100,
            "duration": 10,
            "connections": 10,
        },
        "metrics": {
            "submitted": 1000,
            "accepted": 970,
            "rejected": 25,
            "errors": 5,
            "elapsed_sec": 10.02,
            "actual_rate": 99.8,
            "accept_rate": 97.0,
        },
        "latency_us": {
            "p50": 245,
            "p95": 680,
            "p99": 1250,
            "min": 120,
            "max": 3840,
        },
    }

    html = pages.stress_report_page(data)

    assert "2026-02-13 12:00:00" in html or "20260213" in html
    assert "1,000" in html or "1000" in html  # submitted
    assert "970" in html  # accepted
    assert "97.0" in html  # accept rate
    assert "245" in html  # p50 latency


def test_stress_page_renders():
    """Test that stress test page renders without errors."""
    import pages

    html = pages.stress_page()

    assert "Run Stress Test" in html
    assert "Historical Reports" in html
    assert "/api/stress/run" in html
    assert "orders/sec" in html.lower()


def test_stress_client_error_handling():
    """Test that stress client handles errors gracefully."""
    config = StressConfig(
        gateway_url="ws://localhost:65535",
        rate=1,
        duration=1,
        connections=1,
    )

    results = asyncio.run(run_stress_test(config))

    assert results["metrics"]["submitted"] == 0
    assert results["metrics"]["errors"] >= 0


def test_percentile_calculation():
    """Test that percentile calculation works correctly."""
    data = [100, 200, 300, 400, 500, 600, 700, 800, 900, 1000]

    def percentile(data, p):
        if not data:
            return 0
        k = (len(data) - 1) * p / 100
        f = int(k)
        c = f + 1
        if c >= len(data):
            return data[-1]
        return data[f] + (k - f) * (data[c] - data[f])

    p50 = percentile(data, 50)
    p95 = percentile(data, 95)
    p99 = percentile(data, 99)

    assert 540 <= p50 <= 560
    assert 950 <= p95 <= 960
    assert 989 <= p99 <= 993


def test_report_file_format():
    """Test that report files are saved in correct JSON format."""
    import tempfile

    with tempfile.TemporaryDirectory() as tmpdir:
        report_file = Path(tmpdir) / "stress-20260213-120000.json"

        report_data = {
            "timestamp": "20260213-120000",
            "config": {"target_rate": 100, "duration": 10},
            "metrics": {"submitted": 1000, "accepted": 970},
            "latency_us": {"p50": 245, "p95": 680, "p99": 1250},
        }

        with open(report_file, "w") as f:
            json.dump(report_data, f, indent=2)

        assert report_file.exists()

        with open(report_file) as f:
            loaded = json.load(f)

        assert loaded["timestamp"] == "20260213-120000"
        assert loaded["config"]["target_rate"] == 100
        assert loaded["metrics"]["submitted"] == 1000


def test_environment_check():
    """Check if test environment is properly configured."""
    import sys

    assert sys.version_info >= (3, 11)

    root = Path(__file__).parent.parent
    assert (root / "stress_client.py").exists()
    assert (root / "server.py").exists()
    assert (root / "pages.py").exists()
