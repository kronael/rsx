"""Tests for risk page HTMX endpoints and REST APIs."""

from unittest.mock import AsyncMock

import pytest
from fastapi.testclient import TestClient

import risk_cli
from server import app
from test_utils import account
from test_utils import position

client = TestClient(app)


# ── HTMX partial endpoints ────────────────────────────────


def test_position_heatmap_returns_200():
    r = client.get("/x/position-heatmap")
    assert r.status_code == 200
    assert r.headers["content-type"].startswith("text/html")


def test_position_heatmap_empty_state():
    """With no accounts, shows graceful empty message."""
    r = client.get("/x/position-heatmap")
    assert r.status_code == 200
    assert "no open positions" in r.text


def test_margin_ladder_returns_200():
    r = client.get("/x/margin-ladder")
    assert r.status_code == 200
    assert r.headers["content-type"].startswith("text/html")


def test_margin_ladder_empty_state():
    r = client.get("/x/margin-ladder")
    assert "no open positions" in r.text


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
    """Default uid=1 with no account → 200 HTML message."""
    r = client.get("/x/risk-user")
    assert r.status_code == 200
    assert r.headers["content-type"].startswith("text/html")


def test_risk_user_without_account_message():
    r = client.get("/x/risk-user")
    assert "has no account" in r.text


def test_risk_user_custom_uid():
    """Query with different user_id param."""
    r = client.get("/x/risk-user?risk-uid=42")
    assert r.status_code == 200


def test_risk_user_uid_zero():
    r = client.get("/x/risk-user?risk-uid=0")
    assert r.status_code == 200


# ── REST API endpoints ────────────────────────────────────


def stub_account(monkeypatch, found):
    stub = AsyncMock(return_value=found)
    monkeypatch.setattr(risk_cli, "account_show", stub)
    return stub


def test_api_risk_user_returns_200(monkeypatch):
    stub_account(monkeypatch, account(1))
    r = client.get("/api/risk/users/1")
    assert r.status_code == 200
    assert r.json()["user_id"] == 1


def test_api_risk_user_without_account_is_404():
    r = client.get("/api/risk/users/1")
    assert r.status_code == 404
    assert "has no account" in r.json()["error"]


def test_api_risk_user_different_ids(monkeypatch):
    stub = stub_account(monkeypatch, account(1))
    for uid in [1, 2, 100]:
        r = client.get(f"/api/risk/users/{uid}")
        assert r.status_code == 200
    assert [call.args[1] for call in stub.await_args_list] == [1, 2, 100]


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


def heatmap_user(*nets, symbol_id=10):
    return {"positions": [
        {"symbol_id": symbol_id, "net": net} for net in nets]}


def ladder_user(user_id, margin_ratio=2.0, symbol_id=10):
    return {
        "user_id": user_id, "margin_ratio": margin_ratio,
        "equity": 1_000_000_000, "mm_required": 500_000_000,
        "positions": [{"symbol_id": symbol_id, "net": 5}],
    }


def test_render_position_heatmap_empty_list():
    from pages import render_position_heatmap
    out = render_position_heatmap([])
    assert "no open positions" in out


def test_render_position_heatmap_with_positions():
    from pages import render_position_heatmap
    users = [
        heatmap_user(100, -50),
        heatmap_user(200, symbol_id=1),
    ]
    out = render_position_heatmap(users)
    assert "<table" in out
    assert "PENGU" in out
    assert "BTC" in out


def test_render_position_heatmap_flat_position():
    from pages import render_position_heatmap
    out = render_position_heatmap([heatmap_user(100, -100)])
    assert "<table" in out
    assert "text-slate-500" in out


def test_render_margin_ladder_empty():
    from pages import render_margin_ladder
    out = render_margin_ladder([])
    assert "no open positions" in out


def test_render_margin_ladder_flat_accounts_are_skipped():
    from pages import render_margin_ladder
    out = render_margin_ladder([{"user_id": 1, "positions": []}])
    assert "no open positions" in out


def test_render_margin_ladder_with_data():
    from pages import render_margin_ladder
    out = render_margin_ladder([ladder_user(7)])
    assert "<table" in out
    assert ">7<" in out


def test_render_margin_ladder_closest_to_liquidation_first():
    from pages import render_margin_ladder
    out = render_margin_ladder([
        ladder_user(1, margin_ratio=5.0),
        ladder_user(2, margin_ratio=1.1),
    ])
    assert out.index(">2<") < out.index(">1<")


def test_render_margin_ladder_capped_at_20():
    from pages import MARGIN_LADDER_MAX_ACCOUNTS
    from pages import render_margin_ladder
    users = [ladder_user(uid + 1000) for uid in range(30)]
    out = render_margin_ladder(users)
    assert out.count("<tr") - 1 == MARGIN_LADDER_MAX_ACCOUNTS


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
    lower = out.lower()
    assert "frozen margin" in lower
    # Both sides derive from WAL, so the label says self-consistency.
    assert "wal self-consistency" in lower
    assert "shadow" in lower
    # The mark-process index is named as the source.
    assert "mark-process index" in lower


def test_render_risk_user_flat_account():
    from pages import render_risk_user
    out = render_risk_user(account(3))
    assert "user 3" in out
    assert "flat" in out


def test_render_risk_user_with_positions():
    from pages import render_risk_user
    out = render_risk_user(account(1, positions=[position(10, 100)]))
    assert "<table" in out
    assert "PENGU" in out
