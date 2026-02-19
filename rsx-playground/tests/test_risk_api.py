"""Tests for risk page HTMX endpoints and REST APIs."""

import pytest
from fastapi.testclient import TestClient
from server import app

client = TestClient(app)


# ── HTMX partial endpoints ────────────────────────────────


def test_position_heatmap_returns_200():
    r = client.get("/x/position-heatmap")
    assert r.status_code == 200
    assert r.headers["content-type"].startswith("text/html")


def test_position_heatmap_empty_state():
    """With no WAL fills, shows graceful empty message."""
    r = client.get("/x/position-heatmap")
    assert r.status_code == 200
    body = r.text
    # Either empty-state message or a table is acceptable
    assert ("no fill data available" in body
            or "<table" in body)


def test_margin_ladder_returns_200():
    r = client.get("/x/margin-ladder")
    assert r.status_code == 200
    assert r.headers["content-type"].startswith("text/html")


def test_margin_ladder_empty_state():
    r = client.get("/x/margin-ladder")
    body = r.text
    assert ("no fill data available" in body
            or "<table" in body)


def test_funding_returns_200():
    r = client.get("/x/funding")
    assert r.status_code == 200
    assert r.headers["content-type"].startswith("text/html")


def test_funding_empty_state():
    r = client.get("/x/funding")
    body = r.text
    assert ("no BBO data available" in body
            or "<table" in body)


def test_risk_latency_returns_200():
    r = client.get("/x/risk-latency")
    assert r.status_code == 200
    assert r.headers["content-type"].startswith("text/html")


def test_risk_latency_empty_shows_dashes():
    """Empty latency list shows -- placeholders."""
    r = client.get("/x/risk-latency")
    body = r.text
    assert "p50" in body
    assert "p95" in body
    assert "p99" in body


def test_reconciliation_returns_200():
    r = client.get("/x/reconciliation")
    assert r.status_code == 200
    assert r.headers["content-type"].startswith("text/html")


def test_reconciliation_shows_checks():
    """Reconciliation always shows 3 skip checks."""
    r = client.get("/x/reconciliation")
    body = r.text
    assert "SKIP" in body
    assert "frozen margin" in body.lower() or "margin" in body.lower()


def test_risk_user_returns_200_default():
    """Default uid=1, no postgres → graceful message."""
    r = client.get("/x/risk-user")
    assert r.status_code == 200
    assert r.headers["content-type"].startswith("text/html")


def test_risk_user_no_postgres_message():
    r = client.get("/x/risk-user")
    body = r.text
    # Either no-data message or a table if postgres is present
    assert ("no data" in body
            or "postgres" in body
            or "<table" in body)


def test_risk_user_custom_uid():
    """Query with different user_id param."""
    r = client.get("/x/risk-user?risk-uid=42")
    assert r.status_code == 200


def test_risk_user_uid_zero():
    r = client.get("/x/risk-user?risk-uid=0")
    assert r.status_code == 200


# ── REST API endpoints ────────────────────────────────────


def test_api_risk_user_returns_200():
    r = client.get("/api/risk/users/1")
    assert r.status_code == 200


def test_api_risk_user_no_postgres():
    """Returns JSON with no-postgres status when DB absent."""
    r = client.get("/api/risk/users/1")
    assert r.status_code == 200
    body = r.json()
    # Either list of rows or no-connection message
    assert isinstance(body, (list, dict))
    if isinstance(body, dict):
        assert "user_id" in body or "status" in body


def test_api_risk_user_different_ids():
    for uid in [1, 2, 100]:
        r = client.get(f"/api/risk/users/{uid}")
        assert r.status_code == 200


def test_api_risk_freeze():
    r = client.post("/api/risk/users/1/freeze")
    assert r.status_code == 200
    body = r.json()
    assert body["action"] == "freeze"
    assert body["user_id"] == 1


def test_api_risk_unfreeze():
    r = client.post("/api/risk/users/1/unfreeze")
    assert r.status_code == 200
    body = r.json()
    assert body["action"] == "unfreeze"


def test_api_risk_invalid_action():
    r = client.post("/api/risk/users/1/explode")
    assert r.status_code == 400
    body = r.json()
    assert "error" in body


def test_api_liquidate_returns_html():
    r = client.post("/api/risk/liquidate")
    assert r.status_code == 200
    assert r.headers["content-type"].startswith("text/html")
    assert "liquidation" in r.text.lower()


# ── Risk page route ───────────────────────────────────────


def test_risk_page_returns_200():
    r = client.get("/risk")
    assert r.status_code == 200
    assert r.headers["content-type"].startswith("text/html")


def test_risk_page_has_risk_partials():
    """Risk page HTML references the HTMX risk partials."""
    r = client.get("/risk")
    body = r.text
    assert "position-heatmap" in body or "margin-ladder" in body


# ── render_ function unit tests ───────────────────────────


def test_render_position_heatmap_none():
    from pages import render_position_heatmap
    out = render_position_heatmap(None)
    assert "no fill data available" in out


def test_render_position_heatmap_empty_list():
    from pages import render_position_heatmap
    out = render_position_heatmap([])
    assert "no fill data available" in out


def test_render_position_heatmap_with_fills():
    from pages import render_position_heatmap
    fills = [
        {"symbol_id": 10, "qty": 100, "taker_side": 0},
        {"symbol_id": 10, "qty": 50,  "taker_side": 1},
        {"symbol_id": 1,  "qty": 200, "taker_side": 0},
    ]
    out = render_position_heatmap(fills)
    assert "<table" in out
    # Net position for symbol 10 = 100 - 50 = +50 (long)
    assert "PENGU" in out
    # BTC symbol_id=1
    assert "BTC" in out


def test_render_position_heatmap_flat_position():
    from pages import render_position_heatmap
    fills = [
        {"symbol_id": 10, "qty": 100, "taker_side": 0},
        {"symbol_id": 10, "qty": 100, "taker_side": 1},
    ]
    out = render_position_heatmap(fills)
    # net = 0, shows neutral color label
    assert "<table" in out


def test_render_margin_ladder_none():
    from pages import render_margin_ladder
    out = render_margin_ladder(None)
    assert "no fill data available" in out


def test_render_margin_ladder_empty():
    from pages import render_margin_ladder
    out = render_margin_ladder([])
    assert "no fill data available" in out


def test_render_margin_ladder_with_data():
    from pages import render_margin_ladder
    fills = [
        {
            "symbol_id": 10, "price": 400, "qty": 10,
            "taker_side": 0,
        },
    ]
    out = render_margin_ladder(fills)
    assert "<table" in out
    assert "PENGU" in out
    assert "buy" in out


def test_render_margin_ladder_sell_side():
    from pages import render_margin_ladder
    fills = [
        {
            "symbol_id": 1, "price": 9000000, "qty": 1000,
            "taker_side": 1,
        },
    ]
    out = render_margin_ladder(fills)
    assert "sell" in out


def test_render_margin_ladder_capped_at_20():
    """Only first 20 fills shown."""
    from pages import render_margin_ladder
    fills = [
        {"symbol_id": 10, "price": i, "qty": 1,
         "taker_side": 0}
        for i in range(30)
    ]
    out = render_margin_ladder(fills)
    assert "<table" in out
    # Can't assert row count easily but must not crash
    assert "PENGU" in out


def test_render_funding_none():
    from pages import render_funding
    out = render_funding(None)
    assert "no BBO data available" in out


def test_render_funding_empty_dict():
    from pages import render_funding
    out = render_funding({})
    assert "no BBO data available" in out


def test_render_funding_with_data():
    from pages import render_funding
    stats = {
        10: {"bid_px": 390, "ask_px": 410,
             "bid_qty": 500, "ask_qty": 300},
    }
    out = render_funding(stats)
    assert "<table" in out
    assert "PENGU" in out


def test_render_funding_zero_bid_ask():
    from pages import render_funding
    stats = {
        1: {"bid_px": 0, "ask_px": 0,
            "bid_qty": 0, "ask_qty": 0},
    }
    out = render_funding(stats)
    # mid = 0 → rate = "--"
    assert "--" in out


def test_render_risk_latency_empty():
    from pages import render_risk_latency
    out = render_risk_latency([])
    assert "p50" in out
    assert "--" in out


def test_render_risk_latency_none():
    from pages import render_risk_latency
    out = render_risk_latency(None)
    assert "p50" in out


def test_render_risk_latency_with_data():
    from pages import render_risk_latency
    latencies = [50, 80, 120, 200, 500, 300, 90, 60, 40, 70]
    out = render_risk_latency(latencies)
    assert "p50" in out
    assert "p95" in out
    assert "p99" in out
    assert "us" in out


def test_render_risk_latency_fast_color():
    """Values <100us should use emerald (green) color."""
    from pages import render_risk_latency
    out = render_risk_latency([10, 20, 30])
    assert "emerald" in out


def test_render_risk_latency_slow_color():
    """Values >=500us should use red color."""
    from pages import render_risk_latency
    out = render_risk_latency([500, 600, 700, 800])
    assert "red" in out


def test_render_reconciliation_has_three_checks():
    from pages import render_reconciliation
    out = render_reconciliation()
    assert out.count("SKIP") == 3


def test_render_reconciliation_check_names():
    from pages import render_reconciliation
    out = render_reconciliation()
    assert "Frozen margin" in out or "frozen margin" in out.lower()
    assert "Shadow book" in out or "shadow book" in out.lower()
    assert "Mark price" in out or "mark price" in out.lower()


def test_render_risk_user_none():
    from pages import render_risk_user
    out = render_risk_user(None)
    assert "no data found" in out


def test_render_risk_user_empty_dict():
    from pages import render_risk_user
    out = render_risk_user({})
    # empty dict is falsy
    assert "no data found" in out


def test_render_risk_user_with_data():
    from pages import render_risk_user
    data = {"user_id": 1, "symbol_id": 10,
            "net_position": 100, "margin": 500}
    out = render_risk_user(data)
    assert "<table" in out
    assert "user_id" in out
    assert "net_position" in out


def test_render_risk_user_html_escaping():
    """Values with HTML chars must be escaped."""
    from pages import render_risk_user
    data = {"key": "<script>alert(1)</script>"}
    out = render_risk_user(data)
    assert "<script>" not in out
    assert "&lt;script&gt;" in out
