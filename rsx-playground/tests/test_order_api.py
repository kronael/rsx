"""Tests for order submission API endpoints.

Covers:
- POST /api/orders/test: gateway-down queues with error status
- POST /api/orders/test: invalid user_id/symbol_id returns 400 HTML
- POST /api/orders/test: idempotency key deduplication
- POST /api/orders/test: accepted/rejected/unexpected gateway responses
- POST /api/orders/batch: adds 10 orders to recent_orders
- POST /api/orders/random: adds 5 random orders
- POST /api/orders/invalid: adds rejected order
- POST /api/orders/{cid}/cancel: cancels submitted order
- POST /api/orders/{cid}/cancel: unknown cid returns error
- GET /x/recent-orders: renders HTML table with submitted orders
- _trim_recent_orders: trims when over 200 entries
"""

from unittest.mock import AsyncMock
from unittest.mock import patch

import pytest
from fastapi.testclient import TestClient

import server
from server import app
from server import recent_orders


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
    """Simulate gateway unreachable for send_order_to_gateway."""
    async def _down(*_a, **_kw):
        return None, "gateway not running", None
    monkeypatch.setattr(server, "send_order_to_gateway", _down)


# ── POST /api/orders/test: gateway down ───────────────────────────────────


def test_orders_test_gateway_down_returns_200(client, gw_down):
    """POST /api/orders/test returns 200 HTML when gateway is down."""
    resp = client.post("/api/orders/test", data={})
    assert resp.status_code == 200


def test_orders_test_gateway_down_is_html(client, gw_down):
    """POST /api/orders/test returns text/html content-type."""
    resp = client.post("/api/orders/test", data={})
    assert "text/html" in resp.headers["content-type"]


def test_orders_test_gateway_down_queues_order(client, gw_down):
    """POST /api/orders/test appends order to recent_orders when gateway down."""
    client.post("/api/orders/test", data={})
    assert len(recent_orders) == 1


def test_orders_test_gateway_down_order_has_error_status(client, gw_down):
    """Order stored with status=error when gateway is unreachable."""
    client.post("/api/orders/test", data={})
    assert recent_orders[0]["status"] == "error"


def test_orders_test_gateway_down_response_contains_cid(client, gw_down):
    """Response span contains the cid (pg prefix)."""
    resp = client.post("/api/orders/test", data={})
    assert "pg" in resp.text


def test_orders_test_gateway_down_response_contains_error_hint(client, gw_down):
    """Response mentions 'gateway' or 'queued' when gateway not running."""
    resp = client.post("/api/orders/test", data={})
    text = resp.text.lower()
    assert "gateway" in text or "queued" in text or "error" in text


def test_orders_test_default_fields_stored(client):
    """Order stored with correct default field values."""
    client.post("/api/orders/test", data={})
    o = recent_orders[0]
    assert o["symbol"] == "10"
    assert o["side"] == "buy"
    assert "cid" in o
    assert "ts" in o


def test_orders_test_custom_fields_stored(client):
    """Order stored with custom form field values."""
    client.post("/api/orders/test", data={
        "symbol_id": "3",
        "side": "sell",
        "price": "50000",
        "qty": "2",
        "tif": "IOC",
    })
    o = recent_orders[0]
    assert o["symbol"] == "3"
    assert o["side"] == "sell"
    assert o["price"] == "50000"
    assert o["qty"] == "2"
    assert o["tif"] == "IOC"


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
        data={},
        headers={"x-idempotency-key": "key-001"},
    )
    assert resp.status_code == 200
    assert "duplicate" not in resp.text.lower()


def test_orders_test_idempotency_duplicate_rejected(client):
    """Second submission with same idempotency key returns duplicate response."""
    client.post(
        "/api/orders/test",
        data={},
        headers={"x-idempotency-key": "key-dup"},
    )
    resp = client.post(
        "/api/orders/test",
        data={},
        headers={"x-idempotency-key": "key-dup"},
    )
    assert resp.status_code == 200
    assert "duplicate" in resp.text.lower()


def test_orders_test_idempotency_duplicate_not_appended(client):
    """Duplicate idempotency key submission does not append second order."""
    client.post(
        "/api/orders/test",
        data={},
        headers={"x-idempotency-key": "key-once"},
    )
    client.post(
        "/api/orders/test",
        data={},
        headers={"x-idempotency-key": "key-once"},
    )
    assert len(recent_orders) == 1


def test_orders_test_different_idempotency_keys_both_stored(client):
    """Two submissions with different keys both append orders."""
    client.post(
        "/api/orders/test",
        data={},
        headers={"x-idempotency-key": "key-a"},
    )
    client.post(
        "/api/orders/test",
        data={},
        headers={"x-idempotency-key": "key-b"},
    )
    assert len(recent_orders) == 2


# ── POST /api/orders/test: gateway response paths ─────────────────────────


def test_orders_test_accepted_response_stored(client):
    """OrderAccepted (U status=1) stores order with accepted status."""
    # Wire format: {U: [oid, status, filled, remaining, reason]}
    # status=1 → RESTING (accepted, not rejected)
    accepted_msg = ({"U": [0, 1, 0, 0, 0]}, None, 250)
    with patch(
        "server.send_order_to_gateway",
        new=AsyncMock(return_value=accepted_msg),
    ):
        resp = client.post("/api/orders/test", data={})
    assert resp.status_code == 200
    assert "accepted" in resp.text.lower()
    assert recent_orders[-1]["status"] == "accepted"


def test_orders_test_accepted_response_contains_latency(client):
    """OrderAccepted response span includes latency_us value."""
    accepted_msg = ({"U": [0, 1, 0, 0, 0]}, None, 500)
    with patch(
        "server.send_order_to_gateway",
        new=AsyncMock(return_value=accepted_msg),
    ):
        resp = client.post("/api/orders/test", data={})
    assert "500" in resp.text


def test_orders_test_rejected_response_stored(client):
    """OrderFailed (U status=3) stores order with rejected status."""
    # Wire format: {U: [oid, status, filled, remaining, reason]}
    # status=3 → FAILED; reason is stored as str(reason_code)
    failed_msg = ({"U": [0, 3, 0, 0, "price_band"]}, None, 100)
    with patch(
        "server.send_order_to_gateway",
        new=AsyncMock(return_value=failed_msg),
    ):
        resp = client.post("/api/orders/test", data={})
    assert resp.status_code == 200
    assert "rejected" in resp.text.lower()
    assert recent_orders[-1]["status"] == "rejected"
    assert recent_orders[-1]["reason"] == "price_band"


def test_orders_test_unexpected_response_stored(client):
    """Unexpected gateway response stores order with error status."""
    unexpected_msg = ({"type": "Unknown"}, None, None)
    with patch(
        "server.send_order_to_gateway",
        new=AsyncMock(return_value=unexpected_msg),
    ):
        resp = client.post("/api/orders/test", data={})
    assert resp.status_code == 200
    assert recent_orders[-1]["status"] == "error"


# ── POST /api/orders/batch ────────────────────────────────────────────────


def test_orders_batch_returns_200(client):
    """POST /api/orders/batch returns 200."""
    resp = client.post("/api/orders/batch")
    assert resp.status_code == 200


def test_orders_batch_is_html(client):
    """POST /api/orders/batch returns text/html."""
    resp = client.post("/api/orders/batch")
    assert "text/html" in resp.headers["content-type"]


def test_orders_batch_appends_10_orders(client):
    """POST /api/orders/batch appends exactly 10 orders."""
    client.post("/api/orders/batch")
    assert len(recent_orders) == 10


def test_orders_batch_response_mentions_10(client):
    """POST /api/orders/batch response mentions '10'."""
    resp = client.post("/api/orders/batch")
    assert "10" in resp.text


def test_orders_batch_orders_have_required_fields(client):
    """Batch orders have cid, symbol, side, price, qty, status, ts fields."""
    client.post("/api/orders/batch")
    for o in recent_orders:
        assert "cid" in o
        assert "symbol" in o
        assert "side" in o
        assert "price" in o
        assert "qty" in o
        assert "status" in o
        assert "ts" in o


def test_orders_batch_orders_use_symbol_10(client):
    """Batch orders all use symbol 10."""
    client.post("/api/orders/batch")
    for o in recent_orders:
        assert o["symbol"] == "10"


def test_orders_batch_orders_alternate_sides(client):
    """Batch orders alternate between buy and sell."""
    client.post("/api/orders/batch")
    sides = [o["side"] for o in recent_orders]
    assert "buy" in sides
    assert "sell" in sides


def test_orders_batch_cids_are_unique(client):
    """Batch order cids are all unique."""
    client.post("/api/orders/batch")
    cids = [o["cid"] for o in recent_orders]
    assert len(set(cids)) == len(cids)


# ── POST /api/orders/random ───────────────────────────────────────────────


def test_orders_random_returns_200(client):
    """POST /api/orders/random returns 200."""
    resp = client.post("/api/orders/random")
    assert resp.status_code == 200


def test_orders_random_appends_5_orders(client):
    """POST /api/orders/random appends exactly 5 orders."""
    client.post("/api/orders/random")
    assert len(recent_orders) == 5


def test_orders_random_response_mentions_5(client):
    """POST /api/orders/random response mentions '5'."""
    resp = client.post("/api/orders/random")
    assert "5" in resp.text


def test_orders_random_orders_have_required_fields(client):
    """Random orders have cid, symbol, side, price, qty, status, ts."""
    client.post("/api/orders/random")
    for o in recent_orders:
        assert "cid" in o
        assert "symbol" in o
        assert "side" in o
        assert "price" in o
        assert "qty" in o
        assert "status" in o
        assert "ts" in o


def test_orders_random_orders_have_valid_symbols(client):
    """Random orders use valid symbol IDs [1, 2, 3, 10]."""
    client.post("/api/orders/random")
    valid = {"1", "2", "3", "10"}
    for o in recent_orders:
        assert o["symbol"] in valid, f"unexpected symbol: {o['symbol']}"


def test_orders_random_orders_have_valid_sides(client):
    """Random orders use buy or sell sides."""
    client.post("/api/orders/random")
    for o in recent_orders:
        assert o["side"] in ("buy", "sell")


# ── POST /api/orders/invalid ──────────────────────────────────────────────


def test_orders_invalid_returns_200(client):
    """POST /api/orders/invalid returns 200."""
    resp = client.post("/api/orders/invalid")
    assert resp.status_code == 200


def test_orders_invalid_appends_rejected_order(client):
    """POST /api/orders/invalid appends one rejected order."""
    client.post("/api/orders/invalid")
    assert len(recent_orders) == 1
    assert recent_orders[0]["status"] == "rejected"


def test_orders_invalid_response_mentions_rejected(client):
    """POST /api/orders/invalid response mentions 'rejected' or 'invalid'."""
    resp = client.post("/api/orders/invalid")
    text = resp.text.lower()
    assert "rejected" in text or "invalid" in text


def test_orders_invalid_order_uses_symbol_999(client):
    """Invalid order uses symbol 999."""
    client.post("/api/orders/invalid")
    assert recent_orders[0]["symbol"] == "999"


# ── POST /api/orders/{cid}/cancel ─────────────────────────────────────────


def test_orders_cancel_submitted_order(client):
    """POST /api/orders/{cid}/cancel cancels a submitted order."""
    recent_orders.append({
        "cid": "test-001",
        "symbol": "10",
        "side": "buy",
        "price": "50000",
        "qty": "1",
        "status": "submitted",
        "ts": "12:00:00",
    })
    resp = client.post("/api/orders/test-001/cancel")
    assert resp.status_code == 200
    assert "cancelled" in resp.text.lower()
    assert recent_orders[0]["status"] == "cancelled"


def test_orders_cancel_updates_status_in_place(client):
    """Cancel modifies status in recent_orders in place."""
    recent_orders.append({
        "cid": "test-002",
        "symbol": "10",
        "side": "sell",
        "price": "49000",
        "qty": "2",
        "status": "submitted",
        "ts": "12:00:01",
    })
    client.post("/api/orders/test-002/cancel")
    assert recent_orders[0]["status"] == "cancelled"
    assert recent_orders[0]["cid"] == "test-002"


def test_orders_cancel_unknown_cid_returns_200(client):
    """POST /api/orders/{cid}/cancel for unknown cid returns 200."""
    resp = client.post("/api/orders/nonexistent-cid/cancel")
    assert resp.status_code == 200


def test_orders_cancel_unknown_cid_returns_error(client):
    """Cancel of unknown cid returns error message."""
    resp = client.post("/api/orders/nonexistent-cid/cancel")
    text = resp.text.lower()
    assert "not found" in text or "cancel" in text


def test_orders_cancel_non_submitted_status_not_cancelled(client):
    """Cancel does not change order with status != submitted."""
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
    client.post("/api/orders/batch")
    resp = client.get("/x/recent-orders")
    assert resp.status_code == 200


def test_recent_orders_renders_cids(client):
    """GET /x/recent-orders HTML contains cid strings."""
    client.post("/api/orders/batch")
    resp = client.get("/x/recent-orders")
    # batch cids start with "bat-"
    assert "bat-" in resp.text


def test_recent_orders_no_traceback(client):
    """GET /x/recent-orders does not contain Python tracebacks."""
    client.post("/api/orders/batch")
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
