"""Risk API endpoint tests: account lookup, the overview and its panels.

Accounts, positions, the insurance fund and the liquidations come from
`rsx-cli risk overview` / `account show`, stubbed here at `risk_cli`.

Run with: cd rsx-playground && uv run pytest tests/api_risk_test.py -v
"""

from unittest.mock import AsyncMock

import pytest

import pages
import risk_cli
import server
from test_utils import account
from test_utils import config_row
from test_utils import overview
from test_utils import position
from test_utils import symbol_config

PENGU_MARK = 50_000
ACCOUNT_SHOW = risk_cli.account_show


def stub_overview(monkeypatch, *accounts, **fields):
    stub = AsyncMock(return_value=overview(accounts, **fields))
    monkeypatch.setattr(risk_cli, "risk_overview", stub)
    return stub


def stub_account(monkeypatch, found):
    stub = AsyncMock(return_value=found)
    monkeypatch.setattr(risk_cli, "account_show", stub)
    return stub


# ── Account lookup ────────────────────────────────────────


def test_api_risk_user_returns_the_account(client, monkeypatch):
    stub = stub_account(monkeypatch, account(
        7, collateral=5_000, positions=[position(10, -300, 49_000, 12)]))
    monkeypatch.setattr(server, "bbo_marks", lambda: {10: PENGU_MARK})

    resp = client.get("/api/risk/users/7")

    assert resp.status_code == 200
    assert resp.json() == {
        "user_id": 7, "collateral": 5_000, "frozen": [],
        "positions": [{
            "symbol_id": 10, "net_qty": -300, "avg_px": 49_000,
            "upnl": 12}],
    }
    assert stub.await_args.args[1:] == (7, {10: PENGU_MARK})


@pytest.mark.parametrize("user_id", [0, 999999, 2147483647])
def test_api_risk_user_without_an_account_is_404(client, user_id):
    resp = client.get(f"/api/risk/users/{user_id}")

    assert resp.status_code == 404
    assert "has no account" in resp.json()["error"]


@pytest.mark.allow_5xx
def test_api_risk_user_cli_failure_is_502(client, mock_postgres_down):
    resp = client.get("/api/risk/users/1")

    assert resp.status_code == 502
    assert "connection refused" in resp.json()["error"]


def test_x_risk_user_shows_collateral_position_and_upnl(
        client, monkeypatch):
    stub_account(monkeypatch, account(
        1, collateral=10_000_000_000,
        positions=[position(10, 25_000, 50_000, 50_000_000_000)],
        frozen=[risk_cli.Frozen("ab", 10, 20_000_000_000)]))

    resp = client.get("/x/risk-user?risk-uid=1")

    assert resp.status_code == 200
    assert ">user 1</div>" in resp.text
    assert "$1.00" in resp.text
    assert "$2.00</span> frozen" in resp.text
    assert "PENGU" in resp.text
    assert "+2.5" in resp.text
    assert "+$5.00" in resp.text


def test_x_risk_user_without_upnl_shows_a_dash(client, monkeypatch):
    stub_account(monkeypatch, account(
        1, positions=[position(10, 25_000, 50_000, None)]))

    resp = client.get("/x/risk-user?risk-uid=1")

    assert "<td" in resp.text
    assert "--" in resp.text


def test_x_risk_user_flat_account(client, monkeypatch):
    stub_account(monkeypatch, account(5, collateral=1_000_000_000))

    resp = client.get("/x/risk-user?risk-uid=5")

    assert "user 5" in resp.text
    assert "flat" in resp.text.lower()


def test_x_risk_user_without_an_account_says_so(client):
    resp = client.get("/x/risk-user?user_id=42")

    assert resp.status_code == 200
    assert "user 42 has no account" in resp.text


def test_x_risk_user_cli_failure_reaches_the_page(
        client, mock_postgres_down):
    resp = client.get("/x/risk-user?risk-uid=1")

    assert resp.status_code == 200
    assert "Failed: rsx-cli: connection refused" in resp.text


def test_x_risk_user_invalid_uid_asks_nothing(client):
    resp = client.get("/x/risk-user?risk-uid=invalid")

    assert resp.status_code == 200
    assert "no data" in resp.text
    risk_cli.account_show.assert_not_awaited()


@pytest.mark.parametrize("user_id", [-1, 2**32])
async def test_account_show_never_asks_for_an_id_no_account_has(user_id):
    with pytest.raises(risk_cli.NoAccount, match="has no account"):
        await ACCOUNT_SHOW("postgres://unused", user_id, {})


# ── Overview ──────────────────────────────────────────────


def overview_user(body, user_id):
    return next(user for user in body["users"] if user["user_id"] == user_id)


def test_overview_margins_at_the_active_config_rates(client, monkeypatch):
    """IM and MM come from the symbol's row in force, not a constant."""
    stub_overview(
        monkeypatch,
        account(1, collateral=1_000_000, positions=[
            position(10, 1_000, 40_000, upnl=-5_000)]),
        marks={10: PENGU_MARK})
    monkeypatch.setattr(risk_cli, "config_show", AsyncMock(return_value=[
        symbol_config("PENGU", config_row(
            10, initial_margin_rate_bps=800,
            maintenance_margin_rate_bps=300))]))

    user = overview_user(client.get("/api/risk/overview").json(), 1)

    notional = 1_000 * PENGU_MARK
    pos = user["positions"][0]
    assert pos["notional"] == notional
    assert pos["mark_px"] == PENGU_MARK
    assert pos["entry_px"] == 40_000
    assert pos["im"] == notional * 800 // 10_000
    assert pos["mm"] == notional * 300 // 10_000
    assert user["upnl"] == -5_000
    assert user["equity"] == 1_000_000 - 5_000
    assert user["margin_ratio"] == round(
        (1_000_000 - 5_000) / (notional * 300 // 10_000), 3)


def test_overview_marks_at_entry_without_a_bbo(client, monkeypatch):
    stub_overview(monkeypatch, account(
        1, positions=[position(10, 100, 40_000, upnl=None)]))

    pos = overview_user(
        client.get("/api/risk/overview").json(), 1)["positions"][0]

    assert pos["mark_px"] == 40_000
    assert pos["upnl"] == 0


def test_overview_available_is_collateral_less_frozen(client, monkeypatch):
    stub_overview(monkeypatch, account(
        1, collateral=1_000,
        frozen=[risk_cli.Frozen("a", 10, 300), risk_cli.Frozen("b", 10, 200)]))

    user = overview_user(client.get("/api/risk/overview").json(), 1)

    assert user["frozen"] == 500
    assert user["available"] == 500


def test_overview_counts_longs_shorts_and_near_liquidation(
        client, monkeypatch):
    stub_overview(
        monkeypatch,
        account(1, collateral=10_000_000, positions=[position(10, 1_000)]),
        account(2, collateral=10_000_000, positions=[position(10, -2_000)]),
        account(3, collateral=100, positions=[position(10, 1_000)]),
        account(4),
        marks={10: PENGU_MARK})

    system = client.get("/api/risk/overview").json()["system"]

    assert system["long_notional"] == 2_000 * PENGU_MARK
    assert system["short_notional"] == 2_000 * PENGU_MARK
    assert system["total_oi"] == 4_000 * PENGU_MARK
    assert system["accounts_with_positions"] == 3
    assert system["accounts_near_liq"] == 1


def test_overview_counts_accounts_with_fill_activity(client, monkeypatch):
    """The Risk page never shows fewer positions than the Maker tab."""
    stub_overview(monkeypatch, account(1), account(2), account(3))
    monkeypatch.setattr(server.wal_records, "fills", lambda *_: [
        {"taker_user_id": 1, "maker_user_id": 2, "symbol_id": 10,
         "qty": 5, "price": 100}])

    system = client.get("/api/risk/overview").json()["system"]

    assert system["accounts_with_positions"] == 2


@pytest.mark.allow_5xx
def test_overview_cli_failure_is_502(client, mock_postgres_down):
    resp = client.get("/api/risk/overview")

    assert resp.status_code == 502
    assert "connection refused" in resp.json()["error"]


def test_x_risk_overview_renders_the_panels(client, monkeypatch):
    stub_overview(
        monkeypatch, account(1, positions=[position(10, 1_000)]),
        funds=[risk_cli.Fund(10, 2_000_000_000, 3)],
        insurance_total=2_000_000_000, marks={10: PENGU_MARK})

    resp = client.get("/x/risk-overview")

    assert resp.status_code == 200
    assert ">user 1</div>" in resp.text
    assert "Insurance Fund" in resp.text
    assert "simulated" not in resp.text


def test_x_risk_overview_cli_failure_reaches_the_page(
        client, mock_postgres_down):
    resp = client.get("/x/risk-overview")

    assert "Failed: rsx-cli: connection refused" in resp.text


# ── Heatmap and margin ladder ─────────────────────────────


def test_heatmap_sums_long_and_short_per_symbol(client, monkeypatch):
    stub_overview(
        monkeypatch,
        account(1, positions=[position(10, 30_000)]),
        account(2, positions=[position(10, -10_000)]),
        account(3, positions=[position(1, 5)]))

    resp = client.get("/x/position-heatmap")

    assert "<th" in resp.text
    assert "PENGU" in resp.text and "BTC" in resp.text
    assert ">3<" in resp.text
    assert ">1<" in resp.text
    assert "+2<" in resp.text


def test_heatmap_without_positions_says_so(client, monkeypatch):
    stub_overview(monkeypatch, account(1), account(2))

    assert "no open positions" in client.get("/x/position-heatmap").text


def test_ladder_lists_accounts_closest_to_liquidation_first(
        client, monkeypatch):
    stub_overview(
        monkeypatch,
        account(1, collateral=50_000_000, positions=[position(10, 1_000)]),
        account(2, collateral=5_000_000, positions=[position(10, 1_000)]),
        account(3),
        marks={10: PENGU_MARK})

    text = client.get("/x/margin-ladder").text

    assert "User" in text
    assert "<td class" in text
    first = text.index(">2</td>")
    second = text.index(">1</td>")
    assert first < second
    assert ">3</td>" not in text


def test_ladder_without_positions_says_so(client):
    assert "no open positions" in client.get("/x/margin-ladder").text


@pytest.mark.parametrize("path", ["/x/position-heatmap", "/x/margin-ladder"])
def test_risk_views_report_a_cli_failure(client, mock_postgres_down, path):
    resp = client.get(path)

    assert resp.status_code == 200
    assert "Failed: rsx-cli: connection refused" in resp.text


# ── Liquidations and insurance ────────────────────────────


def test_liquidations_come_from_the_overview(client, monkeypatch):
    rows = [{"user_id": 4, "symbol_id": 10, "round": 1, "status": 0}]
    stub_overview(monkeypatch, liquidations=rows)

    body = client.get("/api/risk/liquidations").json()

    assert body == {"liquidations": rows, "source": "postgres"}


def test_liquidations_fall_back_to_the_wal(client, monkeypatch):
    record = {"user_id": 4, "symbol_id": 10}
    monkeypatch.setattr(
        server.wal_records, "liquidations", lambda *_: [record])

    body = client.get("/api/risk/liquidations").json()

    assert body == {"liquidations": [record], "source": "wal"}


def test_liquidations_none(client):
    body = client.get("/api/risk/liquidations").json()

    assert body == {"liquidations": [], "source": "none"}


def test_x_liquidations_table_of_the_rows(client, monkeypatch):
    stub_overview(monkeypatch, liquidations=[
        {"user_id": 4, "symbol_id": 10, "qty": 50}])

    resp = client.get("/x/liquidations")

    assert "<th" in resp.text
    assert ">50<" in resp.text


def test_x_liquidations_none(client):
    assert "no active liquidations" in client.get("/x/liquidations").text


def test_x_liquidations_cli_failure_reaches_the_page(
        client, mock_postgres_down):
    assert "Failed: rsx-cli" in client.get("/x/liquidations").text


def test_insurance_funds_and_total(client, monkeypatch):
    stub_overview(
        monkeypatch, funds=[risk_cli.Fund(10, 7, 2), risk_cli.Fund(1, 3, 1)],
        insurance_total=10)

    body = client.get("/api/risk/insurance").json()

    assert body["total"] == 10
    assert body["source"] == "postgres"
    assert body["funds"] == [
        {"symbol_id": 10, "balance": 7, "version": 2},
        {"symbol_id": 1, "balance": 3, "version": 1}]


def test_insurance_none(client):
    assert client.get("/api/risk/insurance").json() == {
        "funds": [], "total": 0, "source": "none"}


# ── Other panels ──────────────────────────────────────────


@pytest.mark.parametrize("path", [
    "/x/funding", "/x/risk-latency", "/x/reconciliation"])
def test_other_risk_panels_render(client, path):
    resp = client.get(path)

    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


# ── Routes that had no exchange effect are gone ───────────


@pytest.mark.parametrize("method, path", [
    ("post", "/api/risk/liquidate"),
    ("post", "/api/risk/users/1/freeze"),
    ("post", "/api/risk/users/1/unfreeze"),
    ("post", "/api/users/create"),
    ("post", "/api/users/deposit"),
    ("post", "/api/users/1/deposit"),
])
def test_stub_routes_do_not_exist(client, method, path):
    assert getattr(client, method)(path).status_code in (404, 405)


# ── Risk overview account selection ───────────────────────


def _overview_user(user_id, positions=None, frozen=0, margin_ratio=999.0):
    return {
        "user_id": user_id,
        "collateral": 1_000_000_000_000,
        "frozen": frozen,
        "available": 1_000_000_000_000 - frozen,
        "equity": 1_000_000_000_000,
        "upnl": 0,
        "im_required": 0,
        "mm_required": 0,
        "margin_ratio": margin_ratio,
        "positions": positions or [],
    }


def _overview_position(symbol_id=10):
    return {
        "symbol_id": symbol_id, "net": 5, "entry_px": 100,
        "mark_px": 100, "upnl": 0, "notional": 500, "im": 50,
        "mm": 25,
    }


def _render_overview(users, liquidations=None):
    import pages
    return pages.render_risk_overview(
        {"users": users, "system": {}},
        {"funding": []},
        {"liquidations": liquidations or []},
        {"funds": []},
    )


def test_risk_overview_shows_only_accounts_needing_attention():
    """Flat accounts stay in the totals but get no card."""
    users = [_overview_user(uid) for uid in range(1, 1002)]
    users[6] = _overview_user(7, positions=[_overview_position()])
    users[41] = _overview_user(
        42, frozen=600_000_000_000)
    users[99] = _overview_user(100)
    html = _render_overview(
        users, liquidations=[{"user_id": 100, "symbol_id": 10}])

    for uid in 7, 42, 100:
        assert f">user {uid}</div>" in html
    assert html.count(">user ") == 3
    assert "showing 3 of 3 accounts" in html
    assert "1001 accounts in total" in html
    assert len(html) < 60_000


def test_risk_overview_caps_cards_and_orders_worst_first():
    """Cards are capped; the account with the lowest margin ratio leads."""
    import pages
    users = [
        _overview_user(uid, positions=[_overview_position()],
                       margin_ratio=2.0 + uid)
        for uid in range(1, 51)
    ]
    users[30] = _overview_user(
        31, positions=[_overview_position()], margin_ratio=0.5)
    html = _render_overview(users)

    assert html.count(">user ") == pages.RISK_OVERVIEW_MAX_ACCOUNTS
    assert html.index(">user 31</div>") < html.index(">user 1</div>")
    assert f"showing {pages.RISK_OVERVIEW_MAX_ACCOUNTS} of 50" in html


def test_risk_overview_lists_a_liquidated_account_first():
    users = [
        _overview_user(1, positions=[_overview_position()],
                       margin_ratio=1.0),
        _overview_user(2),
    ]
    html = _render_overview(
        users, liquidations=[{"user_id": 2, "symbol_id": 10}])

    assert html.index(">user 2</div>") < html.index(">user 1</div>")


def test_risk_overview_flat_accounts_say_so():
    html = _render_overview([_overview_user(uid) for uid in range(1, 4)])

    assert ">user " not in html
    assert "no position, margin pressure or liquidation in 3 accounts" in html


def test_risk_overview_no_accounts():
    html = _render_overview([])

    assert "no account data" in html




# ── Open defects (BUGS.md) ────────────────────────────────


def test_format_notional_uses_the_risk_unit():
    assert pages.format_notional(10**10) == "$1.00"
    assert pages.format_notional(-25_007_860_000_000) == "-$2,500.78"


def test_liquidation_panel_shows_the_slippage_of_a_row():
    html = _render_overview([], liquidations=[{
        "user_id": 4, "symbol_id": 10, "round": 1, "status": 0,
        "slippage_bps": 25}])

    assert "25 bps" in html


def test_liquidations_wal_shows_the_record_slip_bps():
    html = pages.render_liquidations_wal([{
        "user_id": 4, "symbol_id": 10, "slip_bps": 25}])

    assert "25 bps" in html


def test_risk_overview_shows_negative_equity_signed_and_red():
    user = _overview_user(4, positions=[_overview_position()])
    user["equity"] = -250_000_000_000
    html = _render_overview([user])

    assert 'text-red-400 font-semibold">-$25.00</div>' in html


def test_trade_agg_shows_the_newest_fill_price():
    html = pages.render_trade_agg([
        {"symbol_id": 10, "qty": 1, "price": 30200},
        {"symbol_id": 10, "qty": 1, "price": 30100},
    ])

    assert ">30200</td>" in html
