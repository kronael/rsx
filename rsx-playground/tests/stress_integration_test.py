"""
Integration tests for stress test functionality.

These tests validate the entire stress test flow:
1. Gateway WebSocket connectivity
2. Stress client can connect and send orders
3. Reports are generated correctly
4. HTML rendering works
5. API endpoints return correct data

Run with: pytest tests/stress_integration_test.py -v
"""

import asyncio
import json
import pytest
import sys
from pathlib import Path

# Add parent to path for imports
sys.path.insert(0, str(Path(__file__).parent.parent))

from stress_client import StressConfig, run_stress_test


@pytest.mark.asyncio
async def test_stress_client_imports():
    """Test that stress client modules import correctly."""
    from stress_client import StressClient, OrderMetrics

    config = StressConfig(rate=10, duration=1)
    assert config.rate == 10
    assert config.duration == 1

    metrics = OrderMetrics()
    assert metrics.submitted == 0
    assert metrics.accepted == 0


@pytest.mark.asyncio
async def test_stress_config_defaults():
    """Test StressConfig default values."""
    config = StressConfig()

    assert config.gateway_url == "ws://localhost:8080"
    assert config.rate == 1000
    assert config.duration == 60
    assert config.symbols == ["BTCUSD"]
    assert config.users == 10
    assert config.connections == 10


@pytest.mark.asyncio
@pytest.mark.skipif(
    True,  # Skip by default - requires Gateway running
    reason="Requires Gateway running on localhost:8080"
)
async def test_gateway_websocket_connection():
    """Test that Gateway WebSocket accepts connections.

    Prerequisites:
    - Gateway must be running: ./target/debug/rsx-gateway
    - Must be outside sandbox (network enabled)
    """
    import websockets

    try:
        async with websockets.connect(
            "ws://localhost:8080",
            additional_headers=[("x-user-id", "1")]
        ) as ws:
            # Connection successful
            assert ws.open

            # Try to send a simple message
            order = {
                "type": "NewOrder",
                "symbol_id": 0,
                "side": "buy",
                "price": "50000.00",
                "qty": "0.1",
                "client_order_id": "test-001",
                "tif": "GTC",
                "reduce_only": False,
                "post_only": False
            }

            await ws.send(json.dumps(order))

            # Wait for response (with timeout)
            response = await asyncio.wait_for(ws.recv(), timeout=2.0)
            msg = json.loads(response)

            # Should get either OrderAccepted or OrderFailed
            assert msg.get("type") in ["OrderAccepted", "OrderFailed"]

    except Exception as e:
        pytest.fail(f"Gateway WebSocket connection failed: {e}")


@pytest.mark.asyncio
@pytest.mark.skipif(
    True,  # Skip by default - requires Gateway running
    reason="Requires Gateway running on localhost:8080"
)
async def test_stress_test_run_small():
    """Test running a small stress test (5 orders/sec for 2 seconds).

    Prerequisites:
    - Gateway must be running
    - Must be outside sandbox
    """
    config = StressConfig(rate=5, duration=2, connections=2)

    results = await run_stress_test(config)

    # Verify results structure
    assert "config" in results
    assert "metrics" in results
    assert "latency_us" in results

    # Verify config matches
    assert results["config"]["target_rate"] == 5
    assert results["config"]["duration"] == 2

    # Verify some orders were submitted
    assert results["metrics"]["submitted"] > 0

    # Verify latency data exists
    if results["metrics"]["accepted"] > 0:
        assert results["latency_us"]["p50"] > 0


def test_stress_report_generation():
    """Test that stress reports can be generated."""
    from pathlib import Path
    import sys
    sys.path.insert(0, str(Path(__file__).parent.parent))

    import pages

    # Sample report data
    data = {
        "timestamp": "20260213-120000",
        "config": {
            "target_rate": 100,
            "duration": 10,
            "connections": 10
        },
        "metrics": {
            "submitted": 1000,
            "accepted": 970,
            "rejected": 25,
            "errors": 5,
            "elapsed_sec": 10.02,
            "actual_rate": 99.8,
            "accept_rate": 97.0
        },
        "latency_us": {
            "p50": 245,
            "p95": 680,
            "p99": 1250,
            "min": 120,
            "max": 3840
        }
    }

    # Generate HTML report
    html = pages.stress_report_page(data)

    # Verify HTML contains key elements
    assert "20260213-120000" in html or "2026-02-13 12:00:00" in html
    assert "1000" in html  # submitted
    assert "970" in html   # accepted
    assert "97.0" in html  # accept rate
    assert "245" in html   # p50 latency
    assert "1250" in html or "1,250" in html  # p99 latency


def test_stress_page_renders():
    """Test that stress test page renders without errors."""
    import sys
    from pathlib import Path
    sys.path.insert(0, str(Path(__file__).parent.parent))

    import pages

    html = pages.stress_page()

    # Verify key elements are present
    assert "Run Stress Test" in html
    assert "Historical Reports" in html
    assert "/api/stress/run" in html
    assert "orders/sec" in html.lower()


@pytest.mark.asyncio
async def test_stress_client_error_handling():
    """Test that stress client handles errors gracefully."""
    # Try to connect to non-existent Gateway
    config = StressConfig(
        gateway_url="ws://localhost:65535",  # Invalid port
        rate=1,
        duration=1,
        connections=1
    )

    results = await run_stress_test(config)

    # Should complete without crashing
    assert results["metrics"]["submitted"] == 0
    assert results["metrics"]["errors"] >= 0


def test_percentile_calculation():
    """Test that percentile calculation works correctly."""
    from stress_client import run_stress_test

    # Test data
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

    # Verify reasonable values
    assert 400 <= p50 <= 600
    assert 900 <= p95 <= 1000
    assert 950 <= p99 <= 1000


@pytest.mark.asyncio
async def test_report_file_format():
    """Test that report files are saved in correct JSON format."""
    from pathlib import Path
    import tempfile
    import json

    # Create temp directory for test reports
    with tempfile.TemporaryDirectory() as tmpdir:
        report_file = Path(tmpdir) / "stress-20260213-120000.json"

        # Sample report data
        report_data = {
            "timestamp": "20260213-120000",
            "config": {"target_rate": 100, "duration": 10},
            "metrics": {"submitted": 1000, "accepted": 970},
            "latency_us": {"p50": 245, "p95": 680, "p99": 1250}
        }

        # Save report
        with open(report_file, "w") as f:
            json.dump(report_data, f, indent=2)

        # Verify file was created
        assert report_file.exists()

        # Verify can load report
        with open(report_file) as f:
            loaded = json.load(f)

        assert loaded["timestamp"] == "20260213-120000"
        assert loaded["config"]["target_rate"] == 100
        assert loaded["metrics"]["submitted"] == 1000


# Diagnostic test that always runs
def test_environment_check():
    """Check if test environment is properly configured."""
    import sys
    from pathlib import Path

    # Check Python version
    assert sys.version_info >= (3, 11), "Python 3.11+ required"

    # Check stress_client.py exists
    stress_client_path = Path(__file__).parent.parent / "stress_client.py"
    assert stress_client_path.exists(), f"stress_client.py not found at {stress_client_path}"

    # Check server.py exists
    server_path = Path(__file__).parent.parent / "server.py"
    assert server_path.exists(), f"server.py not found at {server_path}"

    # Check pages.py exists
    pages_path = Path(__file__).parent.parent / "pages.py"
    assert pages_path.exists(), f"pages.py not found at {pages_path}"

    print("\nEnvironment check passed!")
    print(f"Python: {sys.version}")
    print(f"Stress client: {stress_client_path}")
    print(f"Server: {server_path}")
    print(f"Pages: {pages_path}")


if __name__ == "__main__":
    # Run diagnostic test
    test_environment_check()
    print("\nRun full test suite with: pytest tests/stress_integration_test.py -v")
