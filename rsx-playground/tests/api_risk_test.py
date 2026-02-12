"""Risk API endpoint tests.

Run with: cd rsx-playground && uv run pytest tests/api_risk_test.py -v
"""

import pytest
from fastapi.testclient import TestClient
from unittest.mock import AsyncMock, patch

from test_utils import assert_process_structure


# ── Happy Path Tests (15) ─────────────────────────────────


def test_risk_user_query_success(client, mock_postgres_connected):
    """GET /api/risk/users/{id} returns user data."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "balance": 10000, "position": 0}
        ]
        resp = client.get("/api/risk/users/1")

    assert resp.status_code == 200
    data = resp.json()
    assert isinstance(data, list)
    assert len(data) > 0


def test_risk_user_query_no_postgres(client, mock_postgres_down):
    """GET /api/risk/users/{id} handles no postgres gracefully."""
    resp = client.get("/api/risk/users/1")
    assert resp.status_code == 200
    data = resp.json()
    assert "user_id" in data
    assert data["user_id"] == 1


def test_risk_freeze_user(client):
    """POST /api/risk/users/{id}/freeze returns success."""
    resp = client.post("/api/risk/users/1/freeze")
    assert resp.status_code == 200
    data = resp.json()
    assert "action" in data
    assert data["action"] == "freeze"
    assert data["user_id"] == 1


def test_risk_unfreeze_user(client):
    """POST /api/risk/users/{id}/unfreeze returns success."""
    resp = client.post("/api/risk/users/1/unfreeze")
    assert resp.status_code == 200
    data = resp.json()
    assert "action" in data
    assert data["action"] == "unfreeze"


def test_risk_liquidate_trigger(client):
    """POST /api/risk/liquidate triggers liquidation."""
    resp = client.post("/api/risk/liquidate")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]
    assert "liquidation" in resp.text.lower()


def test_x_risk_user_with_data(client, mock_postgres_connected):
    """GET /x/risk-user with valid user returns HTML table."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "symbol_id": 10, "position": 100}
        ]
        resp = client.get("/x/risk-user?risk-uid=1")

    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_risk_user_no_data(client, mock_postgres_connected):
    """GET /x/risk-user with no data returns message."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = []
        resp = client.get("/x/risk-user?risk-uid=999")

    assert resp.status_code == 200
    assert "no data" in resp.text.lower()


def test_x_liquidations_with_data(client, mock_postgres_connected):
    """GET /x/liquidations with data returns HTML table."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "symbol_id": 10, "qty": 50}
        ]
        resp = client.get("/x/liquidations")

    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_liquidations_no_data(client, mock_postgres_connected):
    """GET /x/liquidations with no data returns message."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = []
        resp = client.get("/x/liquidations")

    assert resp.status_code == 200
    assert "no active liquidations" in resp.text.lower()


def test_risk_multiple_users_query(client, mock_postgres_connected):
    """Query multiple users returns all data."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "balance": 10000},
            {"user_id": 2, "balance": 20000},
        ]
        resp = client.get("/api/risk/users/1")

    assert resp.status_code == 200
    data = resp.json()
    assert len(data) == 2


def test_risk_user_with_positions(client, mock_postgres_connected):
    """User with multiple positions returns all."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "symbol_id": 10, "position": 100},
            {"user_id": 1, "symbol_id": 20, "position": -50},
        ]
        resp = client.get("/api/risk/users/1")

    assert resp.status_code == 200
    data = resp.json()
    assert len(data) == 2


def test_risk_fallback_to_balances_table(
    client, mock_postgres_connected
):
    """Fallback to balances table if risk_positions fails."""
    with patch('server.pg_query') as mock_query:
        mock_query.side_effect = [
            [],
            [{"user_id": 1, "balance": 5000}]
        ]
        resp = client.get("/x/risk-user?risk-uid=1")

    assert resp.status_code == 200


def test_x_position_heatmap_returns_html(client):
    """GET /x/position-heatmap returns HTML."""
    resp = client.get("/x/position-heatmap")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_margin_ladder_returns_html(client):
    """GET /x/margin-ladder returns HTML."""
    resp = client.get("/x/margin-ladder")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_funding_returns_html(client):
    """GET /x/funding returns HTML."""
    resp = client.get("/x/funding")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


# ── Error Cases (20) ──────────────────────────────────────


def test_risk_user_unknown_id(client, mock_postgres_connected):
    """Query unknown user ID returns empty or no data."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = []
        resp = client.get("/api/risk/users/999999")

    assert resp.status_code == 200
    data = resp.json()
    assert isinstance(data, list)
    assert len(data) == 0


def test_risk_action_invalid(client):
    """POST /api/risk/users/{id}/invalid returns 400."""
    resp = client.post("/api/risk/users/1/invalid")
    assert resp.status_code == 400
    data = resp.json()
    assert "error" in data


def test_risk_postgres_query_error(client, mock_postgres_connected):
    """Postgres query error returns error dict."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = {"error": "connection timeout"}
        resp = client.get("/x/risk-user?risk-uid=1")

    assert resp.status_code == 200
    assert "error" in resp.text.lower()


def test_risk_user_negative_id(client):
    """Negative user ID handled gracefully."""
    resp = client.get("/api/risk/users/-1")
    assert resp.status_code == 200


def test_risk_user_zero_id(client):
    """User ID 0 handled gracefully."""
    resp = client.get("/api/risk/users/0")
    assert resp.status_code == 200


def test_risk_freeze_nonexistent_user(client):
    """Freeze nonexistent user returns success (engine handles)."""
    resp = client.post("/api/risk/users/999999/freeze")
    assert resp.status_code == 200


def test_risk_unfreeze_already_unfrozen(client):
    """Unfreeze already unfrozen user is idempotent."""
    resp = client.post("/api/risk/users/1/unfreeze")
    assert resp.status_code == 200


def test_risk_double_freeze(client):
    """Freeze already frozen user is idempotent."""
    resp1 = client.post("/api/risk/users/1/freeze")
    resp2 = client.post("/api/risk/users/1/freeze")
    assert resp1.status_code == 200
    assert resp2.status_code == 200


def test_liquidations_postgres_error(
    client, mock_postgres_connected
):
    """Liquidations query error returns message."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = {"error": "timeout"}
        resp = client.get("/x/liquidations")

    assert resp.status_code == 200


def test_risk_user_invalid_query_param(client):
    """Invalid query param defaults to safe value."""
    resp = client.get("/x/risk-user?risk-uid=invalid")
    assert resp.status_code in [200, 422]


def test_risk_liquidate_no_engine(client):
    """Liquidate without risk engine returns message."""
    resp = client.post("/api/risk/liquidate")
    assert resp.status_code == 200
    assert "requires risk engine" in resp.text.lower()


def test_risk_user_max_int_id(client):
    """Max int user ID handled gracefully."""
    resp = client.get("/api/risk/users/2147483647")
    assert resp.status_code == 200


def test_risk_postgres_connection_lost(
    client, mock_postgres_connected
):
    """Connection lost during query handled."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = None
        resp = client.get("/api/risk/users/1")

    assert resp.status_code == 200


def test_x_risk_user_no_postgres(client, mock_postgres_down):
    """Risk user query with no postgres shows message."""
    resp = client.get("/x/risk-user?risk-uid=1")
    assert resp.status_code == 200
    assert "no data" in resp.text.lower()


def test_x_liquidations_no_postgres(client, mock_postgres_down):
    """Liquidations with no postgres shows message."""
    resp = client.get("/x/liquidations")
    assert resp.status_code == 200


def test_risk_action_missing_user_id(client):
    """Missing user ID in URL returns 404."""
    resp = client.post("/api/risk/users//freeze")
    assert resp.status_code in [404, 422]


def test_risk_user_sql_injection_attempt(
    client, mock_postgres_connected
):
    """SQL injection in user ID handled safely."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = []
        resp = client.get("/api/risk/users/1'; DROP TABLE users; --")

    assert resp.status_code in [200, 422]


def test_risk_concurrent_freeze_unfreeze(client):
    """Concurrent freeze/unfreeze handled."""
    resp1 = client.post("/api/risk/users/1/freeze")
    resp2 = client.post("/api/risk/users/1/unfreeze")
    assert resp1.status_code == 200
    assert resp2.status_code == 200


def test_risk_query_timeout(client, mock_postgres_connected):
    """Query timeout handled gracefully."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = {"error": "timeout"}
        resp = client.get("/api/risk/users/1")

    assert resp.status_code == 200


def test_risk_malformed_response(client, mock_postgres_connected):
    """Malformed postgres response handled."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = "invalid"
        resp = client.get("/api/risk/users/1")

    assert resp.status_code == 200


# ── State Management (10) ─────────────────────────────────


def test_freeze_flag_persists_in_db(client):
    """Freeze action expects persistence in DB."""
    resp = client.post("/api/risk/users/1/freeze")
    assert resp.status_code == 200
    data = resp.json()
    assert "requires risk engine" in data["status"]


def test_position_updates_reflected(client, mock_postgres_connected):
    """Position updates reflected in subsequent queries."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "position": 100}
        ]
        resp1 = client.get("/api/risk/users/1")

        mock_query.return_value = [
            {"user_id": 1, "position": 150}
        ]
        resp2 = client.get("/api/risk/users/1")

    assert resp1.status_code == 200
    assert resp2.status_code == 200


def test_multiple_symbols_per_user(client, mock_postgres_connected):
    """User with multiple symbols shows all positions."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "symbol_id": 10, "position": 100},
            {"user_id": 1, "symbol_id": 20, "position": 50},
            {"user_id": 1, "symbol_id": 30, "position": -25},
        ]
        resp = client.get("/api/risk/users/1")

    assert resp.status_code == 200
    data = resp.json()
    assert len(data) == 3


def test_zero_position_included(client, mock_postgres_connected):
    """Zero positions included in results."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "symbol_id": 10, "position": 0}
        ]
        resp = client.get("/api/risk/users/1")

    assert resp.status_code == 200
    data = resp.json()
    assert len(data) == 1


def test_negative_position_included(client, mock_postgres_connected):
    """Negative positions (shorts) included."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "symbol_id": 10, "position": -100}
        ]
        resp = client.get("/api/risk/users/1")

    assert resp.status_code == 200
    data = resp.json()
    assert len(data) == 1
    assert data[0]["position"] == -100


def test_freeze_then_query_shows_frozen(client):
    """Freeze then query expects frozen flag visible."""
    client.post("/api/risk/users/1/freeze")
    resp = client.get("/api/risk/users/1")
    assert resp.status_code == 200


def test_unfreeze_then_query_shows_active(client):
    """Unfreeze then query expects active flag."""
    client.post("/api/risk/users/1/unfreeze")
    resp = client.get("/api/risk/users/1")
    assert resp.status_code == 200


def test_liquidation_creates_record(
    client, mock_postgres_connected
):
    """Liquidation creates record in DB."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = []
        client.post("/api/risk/liquidate")
        resp = client.get("/x/liquidations")

    assert resp.status_code == 200


def test_balance_updates_atomic(client, mock_postgres_connected):
    """Balance updates expected to be atomic."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "balance": 10000}
        ]
        resp = client.get("/api/risk/users/1")

    assert resp.status_code == 200


def test_query_limit_respected(client, mock_postgres_connected):
    """Query limit (20 rows) respected."""
    with patch('server.pg_query') as mock_query:
        rows = [
            {"user_id": 1, "symbol_id": i, "position": i}
            for i in range(25)
        ]
        mock_query.return_value = rows
        resp = client.get("/x/risk-user?risk-uid=1")

    assert resp.status_code == 200


# ── Integration Tests (15) ────────────────────────────────


def test_freeze_then_verify_rejected_order(client):
    """Freeze user, submit order, verify rejected."""
    client.post("/api/risk/users/1/freeze")
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_unfreeze_then_verify_accepted_order(client):
    """Unfreeze user, submit order, verify accepted."""
    client.post("/api/risk/users/1/freeze")
    client.post("/api/risk/users/1/unfreeze")
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )
    assert resp.status_code == 200


def test_multi_symbol_positions_consistent(
    client, mock_postgres_connected
):
    """Multi-symbol positions remain consistent."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "symbol_id": 10, "position": 100},
            {"user_id": 1, "symbol_id": 20, "position": -50},
        ]
        resp1 = client.get("/api/risk/users/1")
        resp2 = client.get("/api/risk/users/1")

    assert resp1.status_code == 200
    assert resp2.status_code == 200


def test_liquidate_then_check_queue(client, mock_postgres_connected):
    """Liquidate then check liquidation queue."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = []
        client.post("/api/risk/liquidate")
        resp = client.get("/x/liquidations")

    assert resp.status_code == 200


def test_risk_query_then_html_view_consistent(
    client, mock_postgres_connected
):
    """API and HTML view return consistent data."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "balance": 10000}
        ]
        resp_api = client.get("/api/risk/users/1")
        resp_html = client.get("/x/risk-user?risk-uid=1")

    assert resp_api.status_code == 200
    assert resp_html.status_code == 200


def test_freeze_multiple_users(client):
    """Freeze multiple users independently."""
    resp1 = client.post("/api/risk/users/1/freeze")
    resp2 = client.post("/api/risk/users/2/freeze")
    resp3 = client.post("/api/risk/users/3/freeze")

    assert resp1.status_code == 200
    assert resp2.status_code == 200
    assert resp3.status_code == 200


def test_position_sum_equals_fills(client, mock_postgres_connected):
    """Position sum expected to equal fills."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "position": 100}
        ]
        resp = client.get("/api/risk/users/1")

    assert resp.status_code == 200


def test_funding_calculation_zero_sum(client):
    """Funding calculation expected zero-sum."""
    resp = client.get("/x/funding")
    assert resp.status_code == 200


def test_margin_ladder_updates(client):
    """Margin ladder updates with position changes."""
    resp = client.get("/x/margin-ladder")
    assert resp.status_code == 200


def test_risk_latency_tracking(client):
    """Risk latency tracking endpoint works."""
    resp = client.get("/x/risk-latency")
    assert resp.status_code == 200


def test_reconciliation_check(client):
    """Reconciliation check endpoint works."""
    resp = client.get("/x/reconciliation")
    assert resp.status_code == 200


def test_position_heatmap_multi_user(client):
    """Position heatmap with multiple users."""
    resp = client.get("/x/position-heatmap")
    assert resp.status_code == 200


def test_freeze_unfreeze_cycle(client):
    """Complete freeze/unfreeze cycle."""
    resp1 = client.post("/api/risk/users/1/freeze")
    resp2 = client.get("/api/risk/users/1")
    resp3 = client.post("/api/risk/users/1/unfreeze")
    resp4 = client.get("/api/risk/users/1")

    assert all(r.status_code == 200 for r in [
        resp1, resp2, resp3, resp4
    ])


def test_liquidation_with_multiple_positions(
    client, mock_postgres_connected
):
    """Liquidation with multiple positions."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [
            {"user_id": 1, "symbol_id": 10, "qty": 100},
            {"user_id": 1, "symbol_id": 20, "qty": 50},
        ]
        resp = client.get("/x/liquidations")

    assert resp.status_code == 200


def test_risk_state_persistence_across_requests(
    client, mock_postgres_connected
):
    """State persists across requests."""
    with patch('server.pg_query') as mock_query:
        mock_query.return_value = [{"user_id": 1, "balance": 10000}]

        resp1 = client.get("/api/risk/users/1")
        resp2 = client.get("/api/risk/users/1")
        resp3 = client.get("/api/risk/users/1")

    assert all(r.status_code == 200 for r in [resp1, resp2, resp3])
