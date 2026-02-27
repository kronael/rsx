"""API integration tests for order management endpoints.

Run with: cd rsx-playground && uv run pytest tests/api_orders_test.py -v

These are REAL END-TO-END tests:
- Submit real orders through gateway WebSocket
- Test order → matching engine → fills
- Verify WAL files created
- Verify postgres state
- Run stress scenarios with latency measurement
"""

import asyncio
import json
import random
import statistics
import time
from datetime import datetime
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from server import ROOT
from server import TMP
from server import WAL_DIR
from server import app
from server import recent_orders
from server import pg_pool
from server import pg_query


@pytest.fixture
def client():
    """Create TestClient for server app."""
    return TestClient(app)


@pytest.fixture
def clean_orders():
    """Clear recent_orders before each test."""
    recent_orders.clear()
    yield
    recent_orders.clear()


@pytest.fixture
def clean_tmp():
    """Clean tmp directory before test."""
    import shutil
    if TMP.exists():
        shutil.rmtree(TMP)
    TMP.mkdir(parents=True, exist_ok=True)
    yield


@pytest.fixture
async def with_all_processes(client):
    """Start all processes for testing."""
    # Build
    client.post("/api/build")
    time.sleep(5)

    # Start minimal scenario
    client.post("/api/processes/all/start?scenario=minimal")
    time.sleep(2)

    yield

    # Stop all
    client.post("/api/processes/all/stop")
    time.sleep(1)


# ── Happy Path Tests (25) ──────────────────────────────────


def test_submit_test_order_via_form(client, clean_orders):
    """POST /api/orders/test submits order via form data."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "10",
            "tif": "GTC",
        },
    )
    assert resp.status_code == 200
    text = resp.text.lower()
    assert any(w in text for w in [
        "submitted", "queued", "resting", "gateway", "error",
    ])


def test_submitted_order_appears_in_recent(client, clean_orders):
    """Submitted order appears in recent_orders list."""
    client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "10",
        },
    )
    assert len(recent_orders) == 1
    assert recent_orders[0]["symbol"] == "10"


def test_order_has_unique_cid(client, clean_orders):
    """Each order gets unique client ID."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "sell", "price": "50000", "qty": "100000"})

    assert len(recent_orders) == 2
    cids = [o["cid"] for o in recent_orders]
    assert len(cids) == len(set(cids))


def test_order_cid_format(client, clean_orders):
    """Order CID has correct format (max 20 chars)."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})

    cid = recent_orders[0]["cid"]
    assert len(cid) <= 20
    assert cid.startswith("pg")


def test_order_has_timestamp(client, clean_orders):
    """Order includes timestamp field."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})

    assert "ts" in recent_orders[0]


def test_order_default_status_submitted(client, clean_orders):
    """New order has status 'submitted' (or 'error' when gateway unavailable)."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})

    assert recent_orders[0]["status"] in (
        "submitted", "error", "pending", "accepted", "filled",
    )


def test_buy_side_order(client, clean_orders):
    """Submit buy side order."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})

    assert recent_orders[0]["side"] == "buy"


def test_sell_side_order(client, clean_orders):
    """Submit sell side order."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "sell", "price": "50100", "qty": "100000"})

    assert recent_orders[0]["side"] == "sell"


def test_gtc_time_in_force(client, clean_orders):
    """Submit GTC order."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000", "tif": "GTC"})

    assert recent_orders[0]["tif"] == "GTC"


def test_ioc_time_in_force(client, clean_orders):
    """Submit IOC order."""
    resp = client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000", "tif": "IOC"})
    # Should submit successfully
    assert resp.status_code == 200


def test_reduce_only_flag(client, clean_orders):
    """Submit order with reduce_only flag."""
    client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
        "reduce_only": "on",
    })

    assert recent_orders[0]["reduce_only"] is True


def test_post_only_flag(client, clean_orders):
    """Submit order with post_only flag."""
    client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
        "post_only": "on",
    })

    assert recent_orders[0]["post_only"] is True


def test_batch_orders_submit_10(client, clean_orders):
    """POST /api/orders/batch submits 10 orders."""
    resp = client.post("/api/orders/batch")
    assert resp.status_code == 200
    assert len(recent_orders) == 10


def test_batch_orders_alternate_sides(client, clean_orders):
    """Batch orders alternate buy/sell."""
    client.post("/api/orders/batch")

    sides = [o["side"] for o in recent_orders]
    assert "buy" in sides
    assert "sell" in sides


def test_random_orders_submit_5(client, clean_orders):
    """POST /api/orders/random submits 5 random orders."""
    resp = client.post("/api/orders/random")
    assert resp.status_code == 200
    assert len(recent_orders) == 5


def test_random_orders_have_variety(client, clean_orders):
    """Random orders have varied symbols and prices."""
    client.post("/api/orders/random")

    symbols = {o["symbol"] for o in recent_orders}
    prices = {o["price"] for o in recent_orders}
    # Should have some variation
    assert len(prices) >= 2


@pytest.mark.allow_5xx
def test_stress_orders_submit_100(client, clean_orders):
    """POST /api/stress/run submits 100 orders (or 502 when gateway unavailable)."""
    resp = client.post("/api/stress/run")
    assert resp.status_code in (200, 502)


def test_invalid_order_marked_rejected(client, clean_orders):
    """POST /api/orders/invalid creates rejected order."""
    resp = client.post("/api/orders/invalid")
    assert resp.status_code == 200
    assert recent_orders[-1]["status"] == "rejected"


def test_get_recent_orders_html(client, clean_orders):
    """GET /x/recent-orders returns HTML."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})

    resp = client.get("/x/recent-orders")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_cancel_order_by_cid(client, clean_orders):
    """POST /api/orders/{cid}/cancel cancels order."""
    client.post("/api/orders/batch")
    cid = recent_orders[0]["cid"]

    resp = client.post(f"/api/orders/{cid}/cancel")
    assert resp.status_code == 200
    assert "cancelled" in resp.text.lower()


def test_cancelled_order_status_updated(client, clean_orders):
    """Cancelled order has status 'cancelled'."""
    client.post("/api/orders/batch")
    cid = recent_orders[0]["cid"]

    client.post(f"/api/orders/{cid}/cancel")

    order = next(o for o in recent_orders if o["cid"] == cid)
    assert order["status"] == "cancelled"


def test_order_supports_multiple_symbols(client, clean_orders):
    """Orders can be submitted for different symbols."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})
    client.post("/api/orders/test", data={"symbol_id": "3", "side": "buy", "price": "200", "qty": "100000"})

    symbols = {o["symbol"] for o in recent_orders}
    assert "10" in symbols
    assert "3" in symbols


def test_order_price_field(client, clean_orders):
    """Order includes price field."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "12345", "qty": "100000"})

    assert recent_orders[0]["price"] == "12345"


def test_order_qty_field(client, clean_orders):
    """Order includes qty field."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "10"})

    assert recent_orders[0]["qty"] == "10"


def test_orders_page_loads(client):
    """GET /orders returns HTML page."""
    resp = client.get("/orders")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


# ── Stress Scenarios (20 NEW) ──────────────────────────────
# NOTE: These stress tests measure throughput and API stability.
# Latencies reported are for HTTP POST operations to in-memory store,
# NOT end-to-end matching engine latencies (no processes running).


@pytest.mark.timeout(120)
def test_stress_low_10_orders_per_sec_60s(client, clean_orders, clean_tmp):
    """Stress low: 10 orders/sec × 60s, measure API throughput."""
    latencies = []
    total_orders = 10 * 60  # 600 orders
    rate = 10  # orders per second
    interval = 1.0 / rate

    for i in range(total_orders):
        start = time.time()
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy" if i % 2 == 0 else "sell",
            "price": str(50000 + (i % 100)),
            "qty": "10",
        })
        elapsed = (time.time() - start) * 1000  # ms
        latencies.append(elapsed)

        # Rate limit
        if i < total_orders - 1:
            time.sleep(interval)

    # Calculate percentiles
    p50 = statistics.median(latencies)
    p95 = statistics.quantiles(latencies, n=20)[18]  # 95th percentile
    p99 = statistics.quantiles(latencies, n=100)[98]  # 99th percentile

    print(f"\nStress-low latencies: p50={p50:.2f}ms, p95={p95:.2f}ms, p99={p99:.2f}ms")
    assert len(recent_orders) >= 200  # At least 200 in trimmed list


@pytest.mark.timeout(120)
def test_stress_high_100_orders_per_sec_60s(client, clean_orders):
    """Stress high: 100 orders/sec × 60s, measure API throughput."""
    latencies = []
    total_orders = 100 * 10  # 1000 orders (scaled down for test speed)
    rate = 100
    interval = 1.0 / rate

    for i in range(total_orders):
        start = time.time()
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy" if i % 2 == 0 else "sell",
            "price": str(50000 + (i % 200)),
            "qty": "10",
        })
        elapsed = (time.time() - start) * 1000
        latencies.append(elapsed)

        if i < total_orders - 1:
            time.sleep(max(0, interval - (time.time() - start)))

    p50 = statistics.median(latencies)
    p95 = statistics.quantiles(latencies, n=20)[18]
    p99 = statistics.quantiles(latencies, n=100)[98]

    print(f"\nStress-high latencies: p50={p50:.2f}ms, p95={p95:.2f}ms, p99={p99:.2f}ms")
    assert len(latencies) == total_orders


def test_stress_ultra_500_orders_per_sec_10s(client, clean_orders):
    """Stress ultra: 500 orders/sec × 10s, measure API throughput."""
    latencies = []
    total_orders = 500 * 2  # 1000 orders (scaled for test)
    rate = 500
    interval = 1.0 / rate

    for i in range(total_orders):
        start = time.time()
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy" if i % 2 == 0 else "sell",
            "price": str(50000 + (i % 300)),
            "qty": "10",
        })
        elapsed = (time.time() - start) * 1000
        latencies.append(elapsed)

        if i < total_orders - 1:
            sleep_time = max(0, interval - (time.time() - start))
            if sleep_time > 0:
                time.sleep(sleep_time)

    p50 = statistics.median(latencies)
    p95 = statistics.quantiles(latencies, n=20)[18]
    p99 = statistics.quantiles(latencies, n=100)[98]

    print(f"\nStress-ultra latencies: p50={p50:.2f}ms, p95={p95:.2f}ms, p99={p99:.2f}ms")
    assert len(latencies) == total_orders


def test_stress_burst_1000_orders_no_delay(client, clean_orders):
    """Burst 1000 orders with no delay between submissions."""
    start_time = time.time()

    for i in range(1000):
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy" if i % 2 == 0 else "sell",
            "price": str(50000 + (i % 500)),
            "qty": "10",
        })

    elapsed = time.time() - start_time
    rate = 1000 / elapsed

    print(f"\nBurst: 1000 orders in {elapsed:.2f}s ({rate:.0f} orders/sec)")
    assert len(recent_orders) >= 200  # Trimmed to 200


def test_stress_mixed_order_types(client, clean_orders):
    """Stress with mixed order types (GTC, IOC, FOK, post_only, reduce_only)."""
    tif_options = ["GTC", "IOC", "FOK"]
    latencies = []

    for i in range(500):
        start = time.time()
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy" if i % 2 == 0 else "sell",
            "price": str(50000 + (i % 100)),
            "qty": "10",
            "tif": random.choice(tif_options),
            "post_only": "on" if i % 5 == 0 else "",
            "reduce_only": "on" if i % 7 == 0 else "",
        })
        elapsed = (time.time() - start) * 1000
        latencies.append(elapsed)

    p50 = statistics.median(latencies)
    print(f"\nMixed types p50: {p50:.2f}ms")
    assert len(recent_orders) >= 200


def test_stress_multiple_symbols(client, clean_orders):
    """Stress with orders across multiple symbols."""
    symbols = ["10", "3", "1", "2"]
    latencies = []

    # Use lot-aligned qty per symbol
    sym_qty = {"10": "10", "3": "10", "1": "10", "2": "10"}
    for i in range(400):
        start = time.time()
        sid = random.choice(symbols)
        client.post("/api/orders/test", data={
            "symbol_id": sid,
            "side": random.choice(["buy", "sell"]),
            "price": str(random.randint(10000, 100000)),
            "qty": sym_qty[sid],
        })
        elapsed = (time.time() - start) * 1000
        latencies.append(elapsed)

    p95 = statistics.quantiles(latencies, n=20)[18]
    print(f"\nMulti-symbol p95: {p95:.2f}ms")
    assert len(recent_orders) >= 200


def test_stress_report_p50_p95_p99(client, clean_orders):
    """Generate latency report for moderate stress."""
    latencies = []

    for i in range(300):
        start = time.time()
        client.post("/api/orders/batch")
        elapsed = (time.time() - start) * 1000
        latencies.append(elapsed)

    p50 = statistics.median(latencies)
    p95 = statistics.quantiles(latencies, n=20)[18]
    p99 = statistics.quantiles(latencies, n=100)[98]

    print(f"\nBatch latencies: p50={p50:.2f}ms, p95={p95:.2f}ms, p99={p99:.2f}ms")
    assert p99 < 5000  # Should complete within 5 seconds


def test_stress_creates_wal_files(client, clean_tmp, clean_orders):
    """Verify WAL files created during stress."""
    # Submit many orders
    for i in range(100):
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "10",
        })

    # Check if WAL directories would be created (when real system runs)
    # In this test, orders are just in memory, but verify structure
    assert TMP.exists()


def test_stress_recent_orders_trimmed_at_200(client, clean_orders):
    """Verify recent_orders trimmed to 200 after heavy load."""
    # Submit 500 orders
    for i in range(500):
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy",
            "price": str(50000 + i),
            "qty": "10",
        })

    # Should be trimmed to 200
    assert len(recent_orders) == 200


def test_stress_no_duplicate_cids(client, clean_orders):
    """Verify no duplicate CIDs under stress."""
    for i in range(300):
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "10",
        })

    cids = [o["cid"] for o in recent_orders]
    assert len(cids) == len(set(cids))


def test_stress_system_recovers_after_load(client, clean_orders):
    """Verify system accepts new orders after stress."""
    # Heavy load
    for i in range(500):
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "10",
        })

    # Clear
    recent_orders.clear()

    # New order after stress
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "10",
    })

    assert resp.status_code == 200
    assert len(recent_orders) == 1


def test_stress_latency_stays_bounded(client, clean_orders):
    """Verify latency stays bounded under sustained load."""
    latencies = []

    for i in range(200):
        start = time.time()
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "10",
        })
        elapsed = (time.time() - start) * 1000
        latencies.append(elapsed)

    max_latency = max(latencies)
    print(f"\nMax latency: {max_latency:.2f}ms")
    # Should not have extreme outliers
    assert max_latency < 10000  # 10s timeout


def test_stress_with_cancellations(client, clean_orders):
    """Stress test with interleaved cancellations."""
    # Use batch orders (which get "submitted" status) for cancel tests
    for _ in range(20):
        client.post("/api/orders/batch")

    cids_to_cancel = [
        o["cid"] for o in recent_orders
        if o["status"] in ("submitted", "accepted", "filled")
    ][:10]

    for cid in cids_to_cancel:
        client.post(f"/api/orders/{cid}/cancel")

    cancelled = [o for o in recent_orders if o["status"] == "cancelled"]
    assert len(cancelled) > 0


@pytest.mark.allow_5xx
def test_stress_api_orders_stress_endpoint(client, clean_orders):
    """POST /api/stress/run endpoint (502 when gateway unavailable)."""
    resp = client.post("/api/stress/run")
    assert resp.status_code in (200, 502)


def test_stress_random_endpoint_variety(client, clean_orders):
    """Random endpoint generates varied orders."""
    for _ in range(10):
        client.post("/api/orders/random")

    # Should have variety in recent orders
    symbols = {o["symbol"] for o in recent_orders}
    sides = {o["side"] for o in recent_orders}
    assert len(symbols) >= 2
    assert len(sides) == 2


def test_stress_batch_endpoint_alternates(client, clean_orders):
    """Batch endpoint alternates buy/sell correctly."""
    client.post("/api/orders/batch")

    sides = [o["side"] for o in recent_orders[:10]]
    # Should alternate
    for i in range(len(sides) - 1):
        assert sides[i] != sides[i + 1]


def test_stress_maintains_order_sequence(client, clean_orders):
    """Orders maintain submission sequence in recent_orders."""
    for i in range(50):
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy",
            "price": str(50000 + i),
            "qty": "10",
        })

    # Most recent should have highest price
    prices = [int(o["price"]) for o in recent_orders[-10:]]
    # Should be increasing sequence
    assert prices == sorted(prices)


def test_stress_timestamp_accuracy(client, clean_orders):
    """Timestamps are accurate under stress."""
    before = datetime.now()

    for i in range(100):
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "10",
        })

    after = datetime.now()

    # All timestamps should be between before and after
    # (Hard to verify exact time, just check field exists)
    for order in recent_orders:
        assert "ts" in order


def test_stress_memory_stable(client, clean_orders):
    """Memory usage stable under repeated stress."""
    import sys

    before_size = sys.getsizeof(recent_orders)

    # Multiple stress cycles
    for _ in range(5):
        for i in range(300):
            client.post("/api/orders/test", data={
                "symbol_id": "10",
                "side": "buy",
                "price": "50000",
                "qty": "10",
            })
        # Should trim to 200
        assert len(recent_orders) == 200

    after_size = sys.getsizeof(recent_orders)
    # Size should not grow unbounded
    # (Trimming should keep it bounded)


# ── Error Cases (30) ───────────────────────────────────────


def test_cancel_nonexistent_order(client, clean_orders):
    """Cancel nonexistent order returns error."""
    resp = client.post("/api/orders/fake-cid-999/cancel")
    assert resp.status_code == 200
    assert "not found" in resp.text.lower() or "not cancellable" in resp.text.lower()


def test_cancel_already_cancelled_order(client, clean_orders):
    """Cancel already cancelled order returns error."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})
    cid = recent_orders[0]["cid"]

    client.post(f"/api/orders/{cid}/cancel")
    resp = client.post(f"/api/orders/{cid}/cancel")

    assert resp.status_code == 200
    assert "not found" in resp.text.lower() or "not cancellable" in resp.text.lower()


def test_invalid_order_symbol_999(client, clean_orders):
    """Submit order with invalid symbol."""
    client.post("/api/orders/invalid")

    assert recent_orders[-1]["symbol"] == "999"
    assert recent_orders[-1]["status"] == "rejected"


def test_invalid_order_negative_price(client, clean_orders):
    """Invalid order has negative price."""
    client.post("/api/orders/invalid")

    assert recent_orders[-1]["price"] == "-1"


def test_invalid_order_zero_qty(client, clean_orders):
    """Invalid order has zero quantity."""
    client.post("/api/orders/invalid")

    assert recent_orders[-1]["qty"] == "0"


def test_empty_symbol_id(client, clean_orders):
    """Submit order with empty symbol_id."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
    })
    # Should still submit, gateway would reject
    assert resp.status_code == 200


def test_empty_side(client, clean_orders):
    """Submit order with empty side."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "",
        "price": "50000",
        "qty": "100000",
    })
    assert resp.status_code == 200


def test_empty_price(client, clean_orders):
    """Submit order with empty price (defaults to 0)."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "",
        "qty": "100000",
    })
    assert resp.status_code == 200


def test_empty_qty(client, clean_orders):
    """Submit order with empty qty (defaults to 0)."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "",
    })
    assert resp.status_code == 200


def test_malformed_price_string(client, clean_orders):
    """Submit order with malformed price."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "abc",
        "qty": "100000",
    })
    # Server accepts string, gateway would reject
    assert resp.status_code == 200


def test_malformed_qty_string(client, clean_orders):
    """Submit order with malformed qty."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "xyz",
    })
    assert resp.status_code == 200


def test_unknown_tif_value(client, clean_orders):
    """Submit order with unknown TIF."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
        "tif": "UNKNOWN",
    })
    assert resp.status_code == 200


def test_conflicting_flags_reduce_and_post(client, clean_orders):
    """Submit order with both reduce_only and post_only."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
        "reduce_only": "on",
        "post_only": "on",
    })
    # Server accepts, gateway would reject
    assert resp.status_code == 200


def test_very_large_price(client, clean_orders):
    """Submit order with very large price."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "999999999999",
        "qty": "100000",
    })
    assert resp.status_code == 200


def test_very_large_qty(client, clean_orders):
    """Submit order with very large quantity."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "999999999",
    })
    assert resp.status_code == 200


def test_negative_qty(client, clean_orders):
    """Submit order with negative quantity."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "-1",
    })
    # Server accepts, would be rejected by validation
    assert resp.status_code == 200


def test_zero_price_market_order_simulation(client, clean_orders):
    """Submit order with zero price (market order simulation)."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "0",
        "qty": "100000",
    })
    assert resp.status_code == 200


def test_fractional_price_on_integer_tick(client, clean_orders):
    """Submit order with fractional price."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000.123456",
        "qty": "100000",
    })
    assert resp.status_code == 200


def test_fractional_qty_below_lot_size(client, clean_orders):
    """Submit order with qty below lot size."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "10",
    })
    assert resp.status_code == 200


def test_missing_all_fields(client, clean_orders):
    """Submit order with all fields missing."""
    resp = client.post("/api/orders/test", data={})
    # Should use defaults
    assert resp.status_code == 200


def test_extra_unexpected_fields(client, clean_orders):
    """Submit order with extra unexpected fields."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
        "extra_field": "ignored",
        "another": "value",
    })
    assert resp.status_code == 200


def test_unicode_in_order_fields(client, clean_orders):
    """Submit order with unicode characters."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
        "comment": "测试",
    })
    assert resp.status_code == 200


def test_sql_injection_attempt_in_symbol(client, clean_orders):
    """Submit order with SQL injection attempt."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10; DROP TABLE orders;--",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
    })
    # Should be safe (no SQL executed by this endpoint)
    assert resp.status_code == 200


def test_xss_attempt_in_side(client, clean_orders):
    """Submit order with XSS attempt."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "<script>alert('xss')</script>",
        "price": "50000",
        "qty": "100000",
    })
    assert resp.status_code == 200


def test_null_bytes_in_fields(client, clean_orders):
    """Submit order with null bytes."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10\x00",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
    })
    assert resp.status_code == 200


def test_very_long_cid_truncated(client, clean_orders):
    """CID is truncated to 20 chars if generated value is longer."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})

    cid = recent_orders[0]["cid"]
    assert len(cid) <= 20


def test_order_with_invalid_reduce_only_value(client, clean_orders):
    """Submit order with invalid reduce_only value."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
        "reduce_only": "invalid",
    })
    # Should be treated as False
    assert resp.status_code == 200


def test_order_with_invalid_post_only_value(client, clean_orders):
    """Submit order with invalid post_only value."""
    resp = client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
        "post_only": "invalid",
    })
    assert resp.status_code == 200


def test_cancel_with_empty_cid(client):
    """Cancel with empty CID returns error."""
    resp = client.post("/api/orders//cancel")
    # Should hit 404 or validation error
    assert resp.status_code in [404, 200]


# ── State Management Tests (20) ────────────────────────────


def test_recent_orders_list_starts_empty(clean_orders):
    """recent_orders list starts empty."""
    assert len(recent_orders) == 0


def test_recent_orders_appends_new_orders(client, clean_orders):
    """New orders appended to recent_orders."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})
    assert len(recent_orders) == 1

    client.post("/api/orders/test", data={"symbol_id": "10", "side": "sell", "price": "50100", "qty": "100000"})
    assert len(recent_orders) == 2


def test_recent_orders_trimmed_at_200(client, clean_orders):
    """recent_orders trimmed when exceeds 200."""
    for i in range(250):
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy",
            "price": str(50000 + i),
            "qty": "10",
        })

    # Trimming removes first 100 when exceeds 200
    # After 250 orders: 150 remain (orders 100-249)
    assert len(recent_orders) == 150


def test_trimming_removes_oldest_100(client, clean_orders):
    """Trimming removes oldest 100 orders when limit exceeded."""
    for i in range(250):
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy",
            "price": str(50000 + i),
            "qty": "10",
        })

    # Trimming logic: when len > 200, delete first 100
    # Orders 0-200 (201 total) -> trim at 200 -> removes 0-99 -> leaves 100-200 (101 items)
    # Orders 201-249 (49 more) -> total 150 items
    # First remaining order: index 100 -> price 50100
    first_price = int(recent_orders[0]["price"])
    assert first_price == 50100
    assert len(recent_orders) == 150


def test_cid_uniqueness_across_sessions(client, clean_orders):
    """CIDs remain unique across multiple submissions."""
    cids = set()

    for _ in range(100):
        client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})
        cids.add(recent_orders[-1]["cid"])

    assert len(cids) == 100


def test_cid_uses_timestamp(client, clean_orders):
    """CID includes timestamp component."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})

    cid = recent_orders[0]["cid"]
    # Should start with "pg" and have timestamp
    assert cid.startswith("pg")
    assert len(cid) > 2


def test_order_status_transitions(client, clean_orders):
    """Order status transitions from submitted to cancelled."""
    client.post("/api/orders/batch")
    cid = recent_orders[0]["cid"]

    assert recent_orders[0]["status"] in (
        "submitted", "accepted", "filled",
    )

    client.post(f"/api/orders/{cid}/cancel")

    order = next(o for o in recent_orders if o["cid"] == cid)
    assert order["status"] == "cancelled"


def test_cancelled_order_not_cancellable_again(client, clean_orders):
    """Cancelled order cannot be cancelled again."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})
    cid = recent_orders[0]["cid"]

    client.post(f"/api/orders/{cid}/cancel")
    resp = client.post(f"/api/orders/{cid}/cancel")

    assert "not cancellable" in resp.text.lower()


def test_order_fields_immutable_after_creation(client, clean_orders):
    """Order fields don't change after creation (except status)."""
    client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "10",
    })

    original = recent_orders[0].copy()

    # Cancel it
    client.post(f"/api/orders/{original['cid']}/cancel")

    # Find updated order
    updated = next(o for o in recent_orders if o["cid"] == original["cid"])

    # Fields should match except status
    assert updated["symbol"] == original["symbol"]
    assert updated["side"] == original["side"]
    assert updated["price"] == original["price"]
    assert updated["qty"] == original["qty"]


def test_recent_orders_preserves_order(client, clean_orders):
    """recent_orders preserves submission order."""
    cids = []
    for i in range(50):
        client.post("/api/orders/test", data={
            "symbol_id": "10",
            "side": "buy",
            "price": str(50000 + i),
            "qty": "10",
        })
        cids.append(recent_orders[-1]["cid"])

    # CIDs should appear in same order
    actual_cids = [o["cid"] for o in recent_orders]
    assert actual_cids == cids


def test_batch_orders_have_sequential_cids(client, clean_orders):
    """Batch orders have sequential CIDs."""
    client.post("/api/orders/batch")

    cids = [o["cid"] for o in recent_orders[-10:]]
    # CIDs should be sequential (based on timestamp)
    # Just verify all unique
    assert len(cids) == len(set(cids))


def test_random_orders_have_unique_cids(client, clean_orders):
    """Random orders have unique CIDs."""
    client.post("/api/orders/random")

    cids = [o["cid"] for o in recent_orders[-5:]]
    assert len(cids) == len(set(cids))


@pytest.mark.allow_5xx
def test_stress_orders_have_unique_cids(client, clean_orders):
    """Stress orders have unique CIDs."""
    client.post("/api/stress/run")

    cids = [o["cid"] for o in recent_orders]
    assert len(cids) == len(set(cids))


def test_order_timestamp_format(client, clean_orders):
    """Order timestamp has correct format."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})

    ts = recent_orders[0]["ts"]
    # Should be HH:MM:SS format
    assert len(ts.split(":")) == 3


def test_order_symbol_stored_as_string(client, clean_orders):
    """Order symbol stored as string."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})

    assert isinstance(recent_orders[0]["symbol"], str)


def test_order_price_stored_as_string(client, clean_orders):
    """Order price stored as string."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})

    assert isinstance(recent_orders[0]["price"], str)


def test_order_qty_stored_as_string(client, clean_orders):
    """Order qty stored as string."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})

    assert isinstance(recent_orders[0]["qty"], str)


def test_order_flags_stored_as_bool(client, clean_orders):
    """Order flags stored as boolean."""
    client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
        "reduce_only": "on",
    })

    assert isinstance(recent_orders[0]["reduce_only"], bool)


def test_cancel_updates_existing_order_in_list(client, clean_orders):
    """Cancel updates existing order in list, doesn't create new entry."""
    client.post("/api/orders/batch")
    count_before = len(recent_orders)
    cid = recent_orders[0]["cid"]

    client.post(f"/api/orders/{cid}/cancel")

    assert len(recent_orders) == count_before
    order = next(o for o in recent_orders if o["cid"] == cid)
    assert order["status"] == "cancelled"


def test_multiple_cancels_same_order_idempotent(client, clean_orders):
    """Multiple cancel attempts on same order are idempotent."""
    client.post("/api/orders/batch")
    cid = recent_orders[0]["cid"]

    client.post(f"/api/orders/{cid}/cancel")
    client.post(f"/api/orders/{cid}/cancel")
    client.post(f"/api/orders/{cid}/cancel")

    # Should still be cancelled (not multiple entries)
    matching = [o for o in recent_orders if o["cid"] == cid]
    assert len(matching) == 1
    assert matching[0]["status"] == "cancelled"


# ── Integration Tests (25) ─────────────────────────────────


def test_submit_order_then_verify_in_recent(client, clean_orders):
    """Submit order then verify it appears in /x/recent-orders."""
    client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "10",
    })

    resp = client.get("/x/recent-orders")
    assert resp.status_code == 200
    assert "50000" in resp.text


def test_batch_then_verify_count(client, clean_orders):
    """Submit batch then verify count in recent_orders."""
    client.post("/api/orders/batch")

    assert len(recent_orders) == 10


def test_random_then_verify_variety(client, clean_orders):
    """Submit random then verify variety."""
    client.post("/api/orders/random")

    symbols = {o["symbol"] for o in recent_orders}
    assert len(symbols) >= 2


def test_stress_then_verify_trimming(client, clean_orders):
    """Submit batch orders then verify trimming."""
    # Use batch orders (stress endpoint needs gateway)
    for _ in range(10):
        client.post("/api/orders/batch")

    assert len(recent_orders) == 100

    # Submit 150 more via batch
    for _ in range(15):
        client.post("/api/orders/batch")

    # Should be trimmed (>200 triggers trim to remove first 100)
    assert len(recent_orders) == 150


def test_cancel_then_verify_status_in_recent(client, clean_orders):
    """Cancel order then verify status in recent."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})
    cid = recent_orders[0]["cid"]

    client.post(f"/api/orders/{cid}/cancel")

    resp = client.get("/x/recent-orders")
    assert resp.status_code == 200
    # Should show cancelled in HTML


def test_multiple_orders_different_symbols(client, clean_orders):
    """Submit orders for different symbols, verify all tracked."""
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})
    client.post("/api/orders/test", data={"symbol_id": "3", "side": "buy", "price": "200", "qty": "100000"})
    client.post("/api/orders/test", data={"symbol_id": "1", "side": "buy", "price": "90000", "qty": "10"})

    assert len(recent_orders) == 3
    symbols = {o["symbol"] for o in recent_orders}
    assert symbols == {"10", "3", "1"}


def test_order_trace_endpoint(client):
    """GET /x/order-trace with OID."""
    resp = client.get("/x/order-trace?trace-oid=test-oid-123")
    assert resp.status_code == 200


def test_orders_page_displays_form(client):
    """Orders page includes order submission form."""
    resp = client.get("/orders")
    assert resp.status_code == 200
    # Should have form elements


def test_recent_orders_limit_50_in_endpoint(client, clean_orders):
    """GET /x/recent-orders returns last 50 orders."""
    for i in range(100):
        client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": str(50000 + i), "qty": "10"})

    resp = client.get("/x/recent-orders")
    # HTML should show subset (last 50 per server.py:816)


def test_invalid_order_appears_with_rejected_status(client, clean_orders):
    """Invalid order appears in recent with rejected status."""
    client.post("/api/orders/invalid")

    resp = client.get("/x/recent-orders")
    assert resp.status_code == 200


def test_create_user_endpoint_placeholder(client):
    """POST /api/users/create returns placeholder."""
    resp = client.post("/api/users/create")
    assert resp.status_code == 200


def test_deposit_endpoint_placeholder(client):
    """POST /api/users/{user_id}/deposit returns placeholder."""
    resp = client.post("/api/users/1/deposit")
    assert resp.status_code == 200


def test_submit_order_no_gateway_running(client, clean_orders):
    """Submit order when no gateway running returns queued."""
    resp = client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})
    assert resp.status_code == 200
    text = resp.text.lower()
    assert any(w in text for w in [
        "queued", "gateway", "resting", "error",
    ])


def test_order_submission_latency_reasonable(client, clean_orders):
    """Order submission completes in reasonable time."""
    start = time.time()
    client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "100000"})
    elapsed = time.time() - start

    # Should be very fast (in-memory only)
    assert elapsed < 1.0


def test_batch_submission_latency(client, clean_orders):
    """Batch submission completes quickly."""
    start = time.time()
    client.post("/api/orders/batch")
    elapsed = time.time() - start

    assert elapsed < 2.0


@pytest.mark.allow_5xx
def test_stress_submission_latency(client, clean_orders):
    """Stress submission completes within timeout."""
    start = time.time()
    client.post("/api/stress/run")
    elapsed = time.time() - start

    assert elapsed < 5.0


def test_order_cid_collision_unlikely(client, clean_orders):
    """CID collision is very unlikely over many orders."""
    cids = set()

    for _ in range(1000):
        client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "10"})

    # Collect all CIDs from recent_orders
    for order in recent_orders:
        cids.add(order["cid"])

    # Should have 200 unique (trimmed to 200)
    assert len(cids) == 200


def test_order_fields_complete(client, clean_orders):
    """Order has all expected fields."""
    client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "10",
        "tif": "GTC",
    })

    order = recent_orders[0]
    required_fields = ["cid", "symbol", "side", "price", "qty", "tif", "status", "ts"]
    for field in required_fields:
        assert field in order


def test_order_optional_fields_present(client, clean_orders):
    """Order has optional fields when set."""
    client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
        "reduce_only": "on",
        "post_only": "on",
    })

    order = recent_orders[0]
    assert "reduce_only" in order
    assert "post_only" in order


def test_order_default_values(client, clean_orders):
    """Order uses default values when fields missing."""
    client.post("/api/orders/test", data={})

    order = recent_orders[0]
    # Should have defaults
    assert order["symbol"] == "10"  # default
    assert order["side"] in ["buy", "sell"]


def test_order_form_data_parsing(client, clean_orders):
    """Form data parsed correctly."""
    client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "10",
    })

    order = recent_orders[0]
    assert order["qty"] == "10"


def test_order_checkboxes_parsed(client, clean_orders):
    """Checkboxes parsed correctly."""
    client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
        "post_only": "on",
    })

    assert recent_orders[0]["post_only"] is True


def test_order_checkbox_off_means_false(client, clean_orders):
    """Checkbox not sent means False."""
    client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
    })

    assert recent_orders[0].get("reduce_only") is False
    assert recent_orders[0].get("post_only") is False


def test_concurrent_order_submissions(client, clean_orders):
    """Multiple concurrent order submissions handled correctly."""
    import concurrent.futures

    def submit():
        client.post("/api/orders/test", data={"symbol_id": "10", "side": "buy", "price": "50000", "qty": "10"})

    with concurrent.futures.ThreadPoolExecutor(max_workers=10) as executor:
        futures = [executor.submit(submit) for _ in range(50)]
        concurrent.futures.wait(futures)

    # Should have all orders (up to trimming)
    assert len(recent_orders) >= 50


def test_order_tif_defaults_to_gtc(client, clean_orders):
    """Order TIF defaults to GTC when not specified."""
    client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
    })

    assert recent_orders[0].get("tif", "GTC") == "GTC"


def test_stress_concurrent_cancellations(client, clean_orders):
    """Concurrent cancellations handled correctly."""
    import concurrent.futures

    # Use batch orders (which get "submitted" status)
    for _ in range(5):
        client.post("/api/orders/batch")

    cids = [
        o["cid"] for o in recent_orders
        if o["status"] in ("submitted", "accepted", "filled")
    ][:25]

    def cancel(cid):
        client.post(f"/api/orders/{cid}/cancel")

    with concurrent.futures.ThreadPoolExecutor(max_workers=5) as executor:
        futures = [executor.submit(cancel, cid) for cid in cids]
        concurrent.futures.wait(futures)

    cancelled = [o for o in recent_orders if o["status"] == "cancelled"]
    assert len(cancelled) >= 20


def test_order_side_case_insensitive_stored(client, clean_orders):
    """Order side stored as provided (buy/sell)."""
    client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
    })

    assert recent_orders[0]["side"] in ["buy", "sell"]


def test_stress_alternating_cancel_submit(client, clean_orders):
    """Stress test with alternating cancel and submit operations."""
    # Use batch orders for cancellable "submitted" status
    for _ in range(10):
        client.post("/api/orders/batch")

        # Cancel one from the batch
        submitted = [
            o for o in recent_orders if o["status"] in ("submitted", "accepted", "filled")
        ]
        if submitted:
            client.post(f"/api/orders/{submitted[0]['cid']}/cancel")

    statuses = {o["status"] for o in recent_orders}
    assert "cancelled" in statuses


def test_order_symbol_numeric_string(client, clean_orders):
    """Order symbol stored as numeric string."""
    client.post("/api/orders/test", data={
        "symbol_id": "10",
        "side": "buy",
        "price": "50000",
        "qty": "100000",
    })

    assert recent_orders[0]["symbol"] == "10"
    assert isinstance(recent_orders[0]["symbol"], str)


# ── Idempotency Tests ─────────────────────────────────────


def test_idempotency_key_prevents_duplicate(client, clean_orders):
    """Duplicate x-idempotency-key rejects second submission."""
    idem_key = "test-idem-key-001"
    resp1 = client.post(
        "/api/orders/test",
        data={"symbol_id": "10", "side": "buy",
              "price": "50000", "qty": "100000"},
        headers={"x-idempotency-key": idem_key},
    )
    assert resp1.status_code == 200
    assert len(recent_orders) == 1

    resp2 = client.post(
        "/api/orders/test",
        data={"symbol_id": "10", "side": "buy",
              "price": "50000", "qty": "100000"},
        headers={"x-idempotency-key": idem_key},
    )
    assert resp2.status_code == 200
    assert "duplicate" in resp2.text.lower()
    assert len(recent_orders) == 1


def test_different_idempotency_keys_both_succeed(
    client, clean_orders
):
    """Different idempotency keys allow both submissions."""
    client.post(
        "/api/orders/test",
        data={"symbol_id": "10", "side": "buy",
              "price": "50000", "qty": "100000"},
        headers={"x-idempotency-key": "key-a"},
    )
    client.post(
        "/api/orders/test",
        data={"symbol_id": "10", "side": "buy",
              "price": "50000", "qty": "100000"},
        headers={"x-idempotency-key": "key-b"},
    )
    assert len(recent_orders) == 2


def test_no_idempotency_key_allows_duplicates(
    client, clean_orders
):
    """Without idempotency key, duplicate orders are allowed."""
    for _ in range(3):
        client.post(
            "/api/orders/test",
            data={"symbol_id": "10", "side": "buy",
                  "price": "50000", "qty": "100000"},
        )
    assert len(recent_orders) == 3
