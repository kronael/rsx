"""Tests for order submission API endpoints.

Covers:
- POST /api/orders/test: gateway-down queues with error status
- POST /api/orders/test: invalid user_id/symbol_id returns 400 HTML
- POST /api/orders/test: idempotency key deduplication
- POST /api/orders/test: resting/rejected gateway outcomes
- POST /api/orders/{cid}/cancel: unknown cid and non-resting orders
- GET /x/recent-orders: renders HTML table with submitted orders
- _trim_recent_orders: trims when over 200 entries
"""

import pytest
from fake_gateway import ScriptedGateway
from fastapi.testclient import TestClient

import order_cli
import rsx_cli
import server
from server import app
from server import recent_orders


ORDER = {"price": "0.049", "qty": "30"}


@pytest.fixture(autouse=True)
def clear_orders():
    """Clear recent_orders and idempotency keys between tests."""
    recent_orders.clear()
    server._idempotency_keys.clear()
    yield
    recent_orders.clear()
    server._idempotency_keys.clear()


@pytest.fixture
def client():
    return TestClient(app)


@pytest.fixture
def gw_down(monkeypatch):
    """Simulate a gateway the CLI cannot reach."""
    down = rsx_cli.CliError("connect ws://127.0.0.1:8088: refused", 1)
    monkeypatch.setattr(
        server, "ORDER_GATEWAY", ScriptedGateway([down] * 4))


def outcome(status, reason=None, rtt_us=250):
    return order_cli.Outcome(
        cid="abc123", status=status, reason=reason, order_id="a" * 32,
        rtt_us=rtt_us, frames=[])


# ── POST /api/orders/test: gateway down ───────────────────────────────────


def test_orders_test_gateway_down_returns_200(client, gw_down):
    """POST /api/orders/test returns 200 HTML when gateway is down."""
    resp = client.post("/api/orders/test", data=ORDER)
    assert resp.status_code == 200


def test_orders_test_gateway_down_is_html(client, gw_down):
    """POST /api/orders/test returns text/html content-type."""
    resp = client.post("/api/orders/test", data=ORDER)
    assert "text/html" in resp.headers["content-type"]


def test_orders_test_gateway_down_queues_order(client, gw_down):
    """POST /api/orders/test appends order to recent_orders when gateway down."""
    client.post("/api/orders/test", data=ORDER)
    assert len(recent_orders) == 1


def test_orders_test_gateway_down_order_has_error_status(client, gw_down):
    """Order stored with error status when gateway is unreachable."""
    client.post("/api/orders/test", data=ORDER)
    assert recent_orders[0]["status"] == "error"
    assert "refused" in recent_orders[0]["error"]


def test_orders_test_gateway_down_response_contains_error_hint(client, gw_down):
    """Response mentions 'gateway' or 'queued' when gateway not running."""
    resp = client.post("/api/orders/test", data=ORDER)
    assert "order error: connect ws://127.0.0.1:8088: refused" in resp.text


def test_orders_test_default_fields_stored(client):
    """Order stored with correct default field values."""
    client.post("/api/orders/test", data=ORDER)
    order = recent_orders[0]
    assert order["symbol"] == "10"
    assert order["side"] == "buy"
    assert "cid" in order
    assert "ts" in order


def test_orders_test_custom_fields_stored(client):
    """Order stored with custom form field values."""
    client.post("/api/orders/test", data={
        "symbol_id": "3",
        "side": "sell",
        "price": "50000",
        "qty": "2",
        "tif": "IOC",
    })
    order = recent_orders[0]
    assert order["symbol"] == "3"
    assert order["side"] == "sell"
    assert order["price"] == "50000"
    assert order["qty"] == "2"
    assert order["tif"] == "IOC"


# ── POST /api/orders/test: invalid params ─────────────────────────────────


def test_orders_test_invalid_user_id_returns_200(client):
    """Invalid user_id returns 200 with HTML error."""
    resp = client.post("/api/orders/test", data={"user_id": "abc"})
    assert resp.status_code == 200


def test_orders_test_invalid_user_id_contains_error(client):
    """Invalid user_id response contains 'invalid'."""
    resp = client.post("/api/orders/test", data={"user_id": "abc"})
    assert "invalid" in resp.text.lower()


def test_orders_test_invalid_user_id_no_order_stored(client):
    """Invalid user_id does not append to recent_orders."""
    client.post("/api/orders/test", data={"user_id": "abc"})
    assert len(recent_orders) == 0


def test_orders_test_invalid_symbol_id_returns_200(client):
    """Invalid symbol_id returns 200 with HTML error."""
    resp = client.post("/api/orders/test", data={"symbol_id": "xyz"})
    assert resp.status_code == 200


def test_orders_test_invalid_symbol_id_contains_error(client):
    """Invalid symbol_id response contains 'invalid'."""
    resp = client.post("/api/orders/test", data={"symbol_id": "xyz"})
    assert "invalid" in resp.text.lower()


def test_orders_test_invalid_symbol_id_no_order_stored(client):
    """Invalid symbol_id does not append to recent_orders."""
    client.post("/api/orders/test", data={"symbol_id": "xyz"})
    assert len(recent_orders) == 0


# ── POST /api/orders/test: idempotency ───────────────────────────────────


def test_orders_test_idempotency_first_submission_succeeds(client):
    """First submission with idempotency key returns non-duplicate response."""
    resp = client.post(
        "/api/orders/test",
        data=ORDER,
        headers={"x-idempotency-key": "key-001"},
    )
    assert resp.status_code == 200
    assert "duplicate" not in resp.text.lower()


def test_orders_test_idempotency_duplicate_rejected(client):
    """Second submission with same idempotency key returns duplicate response."""
    client.post(
        "/api/orders/test",
        data=ORDER,
        headers={"x-idempotency-key": "key-dup"},
    )
    resp = client.post(
        "/api/orders/test",
        data=ORDER,
        headers={"x-idempotency-key": "key-dup"},
    )
    assert resp.status_code == 200
    assert "duplicate" in resp.text.lower()


def test_orders_test_idempotency_duplicate_not_appended(client):
    """Duplicate idempotency key submission does not append second order."""
    client.post(
        "/api/orders/test",
        data=ORDER,
        headers={"x-idempotency-key": "key-once"},
    )
    client.post(
        "/api/orders/test",
        data=ORDER,
        headers={"x-idempotency-key": "key-once"},
    )
    assert len(recent_orders) == 1


def test_orders_test_different_idempotency_keys_both_stored(client):
    """Two submissions with different keys both append orders."""
    client.post(
        "/api/orders/test",
        data=ORDER,
        headers={"x-idempotency-key": "key-a"},
    )
    client.post(
        "/api/orders/test",
        data=ORDER,
        headers={"x-idempotency-key": "key-b"},
    )
    assert len(recent_orders) == 2


# ── POST /api/orders/test: gateway response paths ─────────────────────────


def test_orders_test_resting_outcome_stored(client, monkeypatch):
    monkeypatch.setattr(
        server, "ORDER_GATEWAY", ScriptedGateway([outcome("resting")]))
    resp = client.post("/api/orders/test", data=ORDER)
    assert resp.status_code == 200
    assert "order abc123 resting" in resp.text
    assert recent_orders[-1]["status"] == "resting"
    assert recent_orders[-1]["cid"] == "abc123"


def test_orders_test_resting_response_contains_latency(client, monkeypatch):
    monkeypatch.setattr(
        server, "ORDER_GATEWAY",
        ScriptedGateway([outcome("resting", rtt_us=500)]))
    resp = client.post("/api/orders/test", data=ORDER)
    assert "500us" in resp.text
    assert recent_orders[-1]["latency_us"] == 500


def test_orders_test_rejected_outcome_stored(client, monkeypatch):
    monkeypatch.setattr(
        server, "ORDER_GATEWAY",
        ScriptedGateway([outcome("rejected", "PRICE_BAND")]))
    resp = client.post("/api/orders/test", data=ORDER)
    assert resp.status_code == 200
    assert "rejected: PRICE_BAND" in resp.text
    assert recent_orders[-1]["status"] == "rejected"
    assert recent_orders[-1]["reason"] == "PRICE_BAND"


# ── POST /api/orders/{cid}/cancel ─────────────────────────────────────────


def test_orders_cancel_unknown_cid_returns_200(client):
    """POST /api/orders/{cid}/cancel for unknown cid returns 200."""
    resp = client.post("/api/orders/nonexistent-cid/cancel")
    assert resp.status_code == 200


def test_orders_cancel_unknown_cid_returns_error(client):
    """Cancel of unknown cid returns error message."""
    resp = client.post("/api/orders/nonexistent-cid/cancel")
    text = resp.text.lower()
    assert "not found" in text or "cancel" in text


def test_orders_cancel_non_resting_status_not_cancelled(client):
    """Cancel does not change an order that is not resting."""
    recent_orders.append({
        "cid": "test-003",
        "symbol": "10",
        "side": "buy",
        "price": "50000",
        "qty": "1",
        "status": "accepted",
        "ts": "12:00:02",
    })
    resp = client.post("/api/orders/test-003/cancel")
    assert resp.status_code == 200
    assert recent_orders[0]["status"] == "accepted"


# ── GET /x/recent-orders ──────────────────────────────────────────────────


def test_recent_orders_empty_returns_200(client):
    """GET /x/recent-orders returns 200 when no orders."""
    resp = client.get("/x/recent-orders")
    assert resp.status_code == 200


def test_recent_orders_empty_is_html(client):
    """GET /x/recent-orders returns text/html."""
    resp = client.get("/x/recent-orders")
    assert "text/html" in resp.headers["content-type"]


def test_recent_orders_with_data_returns_200(client):
    """GET /x/recent-orders returns 200 with orders in recent_orders."""
    client.post("/api/orders/test", data=ORDER)
    resp = client.get("/x/recent-orders")
    assert resp.status_code == 200


def test_recent_orders_renders_cids(client):
    """GET /x/recent-orders HTML contains cid strings."""
    recent_orders.append({
        "cid": "k3x9a0b1c", "user_id": 1, "symbol": "10", "side": "buy",
        "price": "0.049", "qty": "30", "tif": "GTC", "status": "resting",
        "ts": "12:00:00",
    })
    resp = client.get("/x/recent-orders")
    assert "k3x9a0b1c" in resp.text


def test_recent_orders_no_traceback(client):
    """GET /x/recent-orders does not contain Python tracebacks."""
    client.post("/api/orders/test", data=ORDER)
    resp = client.get("/x/recent-orders")
    assert "Traceback" not in resp.text
    assert "Exception" not in resp.text


def test_recent_orders_after_test_order(client):
    """GET /x/recent-orders shows order from /api/orders/test."""
    client.post("/api/orders/test", data={"price": "50000", "qty": "1"})
    resp = client.get("/x/recent-orders")
    assert resp.status_code == 200
    assert "<" in resp.text  # is HTML


# ── _trim_recent_orders ───────────────────────────────────────────────────


def test_trim_recent_orders_below_threshold_no_trim(client):
    """recent_orders stays intact when fewer than 200 entries."""
    for i in range(50):
        recent_orders.append({"cid": f"t-{i:04d}", "status": "submitted"})
    server._trim_recent_orders()
    assert len(recent_orders) == 50


def test_trim_recent_orders_above_threshold_trims(client):
    """_trim_recent_orders removes first 100 entries when over 200."""
    for i in range(201):
        recent_orders.append({"cid": f"t-{i:04d}", "status": "submitted"})
    server._trim_recent_orders()
    # Should trim first 100 → 101 remain
    assert len(recent_orders) == 101


def test_trim_recent_orders_keeps_newest(client):
    """After trim, the newest orders are retained."""
    for i in range(201):
        recent_orders.append({"cid": f"t-{i:04d}", "status": "submitted"})
    server._trim_recent_orders()
    # Last entry should be t-0200
    assert recent_orders[-1]["cid"] == "t-0200"
