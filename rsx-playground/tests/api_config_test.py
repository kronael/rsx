"""Symbol risk-config tab: admin gate, active-config read, scheduling,
and the minimum order notional (raw risk units in Postgres, USD at the
edge). Config reads and writes go through `risk_cli`, stubbed here."""

import time
from unittest.mock import AsyncMock

import pytest
import risk_cli
import rsx_cli
import server
from test_utils import symbol_config


def test_config_page_requires_admin(client):
    """Non-admin callers get 403, not the config table."""
    resp = client.get("/config")
    assert resp.status_code == 403
    assert "admin access required" in resp.text


def test_config_schedule_requires_admin(client):
    """The write path is gated too, not just the view."""
    resp = client.post("/api/config/schedule", data={
        "symbol_id": "10",
        "initial_margin_rate_bps": "1000",
        "maintenance_margin_rate_bps": "500",
        "max_leverage": "10",
        "effective_in_sec": "60",
    })
    assert resp.status_code == 403


def test_config_page_renders_rows(monkeypatch):
    """A config row renders as percentages, not raw bps."""
    import pages
    html = pages.config_page([{
        "symbol_id": 10, "symbol_name": "PENGU", "config_version": 1,
        "tick_size": 1, "lot_size": 100000, "status": "trading",
        "maker_fee_bps": -1, "taker_fee_bps": 5,
        "initial_margin_rate_bps": 1000,
        "maintenance_margin_rate_bps": 500,
        "max_leverage": 10, "funding_interval_sec": 28800,
    }], "")
    assert "PENGU" in html
    assert "10.00%" in html
    assert "5.00%" in html
    assert "10x" in html
    assert "8h" in html
    assert "risk shards load the active row" in html
    assert "CONFIG_APPLIED" in html


def test_config_page_empty_states_are_explicit():
    """An empty table says why, rather than rendering blank."""
    html = __import__("pages").config_page([], "")
    assert "no symbol config" in html
    assert "migration" in html

    html_err = __import__("pages").config_page([], "connection refused")
    assert "connection refused" in html_err


# ── minimum order notional ────────────────────────────────
#
# Raw min_notional is risk's price x qty unit: 1 USD is
# 10^(price_decimals + qty_decimals). Postgres holds the raw integer;
# USD exists only in the form and the display.

BTC_ROW = {
    "symbol_id": 1, "symbol_name": "BTC", "config_version": 1,
    "effective_at_ms": 0, "tick_size": 50, "lot_size": 100,
    "price_decimals": 2, "qty_decimals": 8, "status": "trading",
    "min_notional": 10_000_000_000, "max_order_qty": None,
    "maker_fee_bps": -1, "taker_fee_bps": 5,
    "initial_margin_rate_bps": 500, "maintenance_margin_rate_bps": 250,
    "max_leverage": 20, "funding_interval_sec": 28800,
    "funding_rate_min_bps": -10, "funding_rate_max_bps": 10,
    "created_at_ms": 0,
}
# A hypothetical symbol whose decimals sum to 9, so the conversion
# cannot hide behind the presets' common sum of 10.
ODD_ROW = BTC_ROW | {
    "symbol_id": 99, "symbol_name": "ODD", "price_decimals": 5,
    "qty_decimals": 4, "min_notional": 1_500_000_000,
}


class FakeSchedule:
    """`config show` and `config schedule` over rows held in memory."""

    def __init__(self, *rows):
        self.rows = [dict(row) for row in rows]
        self.schedules = []

    def latest(self, symbol_id):
        mine = [r for r in self.rows if r["symbol_id"] == symbol_id]
        return max(mine, key=lambda r: r["config_version"], default=None)

    async def config_show(self, database_url):
        configs = []
        for symbol_id in sorted({r["symbol_id"] for r in self.rows}):
            mine = sorted(
                (r for r in self.rows if r["symbol_id"] == symbol_id),
                key=lambda r: r["config_version"])
            configs.append(symbol_config(
                mine[0]["symbol_name"], *[config_row_of(r) for r in mine]))
        return configs

    async def config_schedule(
            self, database_url, symbol_id, settings, effective_at_ms):
        self.schedules.append((symbol_id, settings, effective_at_ms))
        row = self.latest(symbol_id) | settings | {
            "config_version": self.latest(symbol_id)["config_version"] + 1,
            "effective_at_ms": effective_at_ms,
            "created_at_ms": effective_at_ms,
        }
        self.rows.append(row)
        return risk_cli.Scheduled(
            symbol_id, row["config_version"], effective_at_ms)


def config_row_of(row):
    return risk_cli.ConfigRow(
        **{k: v for k, v in row.items() if k != "symbol_name"})


@pytest.fixture
def admin(monkeypatch):
    async def fake_role(request):
        return "admin"

    monkeypatch.setattr(server, "_caller_role", fake_role)


def fake_cli(monkeypatch, *rows):
    schedule = FakeSchedule(*rows)
    monkeypatch.setattr(
        risk_cli, "config_show", AsyncMock(side_effect=schedule.config_show))
    monkeypatch.setattr(
        risk_cli, "config_schedule",
        AsyncMock(side_effect=schedule.config_schedule))
    return schedule


def refuse_schedule(monkeypatch, message, code):
    refusal = AsyncMock(side_effect=rsx_cli.CliError(message, code))
    monkeypatch.setattr(risk_cli, "config_schedule", refusal)
    return refusal


def schedule_form(symbol_id, **overrides):
    form = {
        "symbol_id": str(symbol_id),
        "initial_margin_rate_bps": "1000",
        "maintenance_margin_rate_bps": "500",
        "max_leverage": "10",
        "effective_in_sec": "0",
    }
    return form | overrides


def test_config_page_shows_min_order_in_usd_per_symbol_decimals():
    """Each symbol's raw minimum is divided by its own decimals."""
    import pages
    html = pages.config_page(
        [BTC_ROW, ODD_ROW, BTC_ROW | {
            "symbol_id": 3, "symbol_name": "SOL", "min_notional": 1000,
            "price_decimals": 4, "qty_decimals": 6}], "")
    assert "Min order (USD)" in html
    assert 'data-testid="min-order-1">1.00<' in html
    assert 'data-testid="min-order-99">1.50<' in html
    # 1000 raw at 4 + 6 decimals is 1e-7 USD, shown exactly.
    assert 'data-testid="min-order-3">0.0000001<' in html


def test_config_page_min_order_missing_is_a_dash():
    import pages
    html = pages.config_page([BTC_ROW | {"min_notional": None}], "")
    assert 'data-testid="min-order-1">-<' in html


def test_config_form_takes_min_order_in_usd():
    import pages
    html = pages.config_page([BTC_ROW], "")
    assert 'name="min_notional_usd"' in html
    assert "USD" in html


def test_config_view_reads_min_order_through_the_cli(
        client, admin, monkeypatch):
    fake_cli(monkeypatch, BTC_ROW, ODD_ROW)
    resp = client.get("/config")
    assert resp.status_code == 200
    assert 'data-testid="min-order-1">1.00<' in resp.text
    assert 'data-testid="min-order-99">1.50<' in resp.text


def test_config_view_shows_the_active_row_not_one_scheduled_later(
        client, admin, monkeypatch):
    future = int(time.time() * 1000) + 3_600_000
    fake_cli(monkeypatch, BTC_ROW, BTC_ROW | {
        "config_version": 2, "effective_at_ms": future,
        "initial_margin_rate_bps": 900,
        "maintenance_margin_rate_bps": 400, "min_notional": 90_000_000_000})
    resp = client.get("/config")
    assert resp.status_code == 200
    assert ">v1</td>" in resp.text
    assert ">v2</td>" not in resp.text
    assert "5.00%" in resp.text
    assert "9.00%" not in resp.text
    assert 'data-testid="min-order-1">1.00<' in resp.text


def test_config_view_lists_no_symbol_without_a_row_in_force(
        client, admin, monkeypatch):
    future = int(time.time() * 1000) + 3_600_000
    fake_cli(monkeypatch, BTC_ROW | {"effective_at_ms": future})
    resp = client.get("/config")
    assert resp.status_code == 200
    assert "min-order-1" not in resp.text


def test_config_view_reports_a_cli_failure(
        client, admin, mock_postgres_down):
    resp = client.get("/config")
    assert resp.status_code == 200
    assert "connection refused" in resp.text
    assert "min-order-" not in resp.text


def test_schedule_writes_two_usd_as_raw_integer(client, admin, monkeypatch):
    """2 USD on BTC (2 + 8 decimals) is stored as 2 x 10^10."""
    cli = fake_cli(monkeypatch, BTC_ROW)
    resp = client.post("/api/config/schedule", data=schedule_form(
        1, min_notional_usd="2"))
    assert resp.status_code == 200
    assert "scheduled v2" in resp.text
    assert "2.00 USD" in resp.text
    symbol_id, settings, _ = cli.schedules[0]
    assert symbol_id == 1
    assert settings == {
        "initial_margin_rate_bps": 1000,
        "maintenance_margin_rate_bps": 500,
        "max_leverage": 10,
        "min_notional": 20_000_000_000,
    }
    assert type(settings["min_notional"]) is int
    assert cli.latest(1)["min_notional"] == 20_000_000_000
    # The previous row is still there: a new version, no update.
    assert cli.rows[0]["min_notional"] == 10_000_000_000
    page = client.get("/config")
    assert 'data-testid="min-order-1">2.00<' in page.text


def test_schedule_effective_time_is_now_plus_the_delay(
        client, admin, monkeypatch):
    cli = fake_cli(monkeypatch, BTC_ROW)
    before = int(time.time() * 1000)
    client.post("/api/config/schedule", data=schedule_form(
        1, effective_in_sec="60"))
    after = int(time.time() * 1000)
    effective_at = cli.schedules[0][2]
    assert before + 60_000 <= effective_at <= after + 60_000


@pytest.mark.parametrize("usd, symbol_row, raw", [
    ("1.5", ODD_ROW, 1_500_000_000),
    ("0.25", BTC_ROW, 2_500_000_000),
    (" 7 ", ODD_ROW, 7_000_000_000),
    ("0.0000000001", BTC_ROW, 1),
    ("2.00000000005", BTC_ROW, 20_000_000_001),
    ("922337203.6854775807", BTC_ROW, 2**63 - 1),
])
def test_schedule_converts_with_the_symbols_own_decimals(
        client, admin, monkeypatch, usd, symbol_row, raw):
    cli = fake_cli(monkeypatch, BTC_ROW, ODD_ROW)
    resp = client.post("/api/config/schedule", data=schedule_form(
        symbol_row["symbol_id"], min_notional_usd=usd))
    assert resp.status_code == 200, resp.text
    assert cli.schedules[0][1]["min_notional"] == raw


def test_schedule_blank_min_order_keeps_the_current_value(
        client, admin, monkeypatch):
    """A margin-only change must not reset a raised minimum."""
    cli = fake_cli(monkeypatch, BTC_ROW | {"min_notional": 50_000_000_000})
    resp = client.post("/api/config/schedule", data=schedule_form(
        1, min_notional_usd=""))
    assert resp.status_code == 200
    assert "min_notional" not in cli.schedules[0][1]
    assert cli.latest(1)["min_notional"] == 50_000_000_000


@pytest.mark.parametrize("usd", [
    "-1", "-0.01", "0", "0.0", "abc", "1,5", "nan", "inf", "-inf",
])
def test_schedule_rejects_non_positive_or_non_numeric_usd(
        client, admin, monkeypatch, usd):
    cli = fake_cli(monkeypatch, BTC_ROW)
    resp = client.post("/api/config/schedule", data=schedule_form(
        1, min_notional_usd=usd))
    assert resp.status_code == 400
    assert "minimum order must be" in resp.text
    assert cli.schedules == []


@pytest.mark.parametrize("usd", ["0.00000000004", "1e-999999999"])
def test_schedule_rejects_usd_below_the_smallest_unit(
        client, admin, monkeypatch, usd):
    """Under half a raw unit rounds to zero, which risk refuses."""
    cli = fake_cli(monkeypatch, BTC_ROW)
    resp = client.post("/api/config/schedule", data=schedule_form(
        1, min_notional_usd=usd))
    assert resp.status_code == 400
    assert "smaller than" in resp.text
    assert cli.schedules == []


@pytest.mark.parametrize("usd", [
    "922337203.6854775808", "922337203.7", "1e999999999"])
def test_schedule_rejects_usd_beyond_i64(client, admin, monkeypatch, usd):
    cli = fake_cli(monkeypatch, BTC_ROW)
    resp = client.post("/api/config/schedule", data=schedule_form(
        1, min_notional_usd=usd))
    assert resp.status_code == 400
    assert "too large" in resp.text
    assert cli.schedules == []


def test_schedule_min_order_unknown_symbol_is_404(
        client, admin, monkeypatch):
    cli = fake_cli(monkeypatch, BTC_ROW)
    resp = client.post("/api/config/schedule", data=schedule_form(
        77, min_notional_usd="1"))
    assert resp.status_code == 404
    assert "unknown symbol 77" in resp.text
    assert cli.schedules == []


def test_schedule_cli_refusal_is_a_400_with_the_cli_message(
        client, admin, monkeypatch):
    fake_cli(monkeypatch, BTC_ROW)
    refuse_schedule(
        monkeypatch, "initial_margin_rate_bps must be positive", 2)
    resp = client.post("/api/config/schedule", data=schedule_form(1))
    assert resp.status_code == 400
    assert "initial_margin_rate_bps must be positive" in resp.text
    assert "text-red-400" in resp.text


def test_schedule_does_not_check_maintenance_above_initial_itself(
        client, admin, monkeypatch):
    """The CLI owns the margin rules; Python forwards the form as typed."""
    fake_cli(monkeypatch, BTC_ROW)
    refusal = refuse_schedule(
        monkeypatch,
        "maintenance margin 1500 bps exceeds initial margin 1000 bps", 2)
    resp = client.post("/api/config/schedule", data=schedule_form(
        1, maintenance_margin_rate_bps="1500"))
    assert resp.status_code == 400
    assert "maintenance margin 1500 bps exceeds" in resp.text
    refusal.assert_awaited_once()
    settings = refusal.await_args.args[2]
    assert settings["maintenance_margin_rate_bps"] == 1500
    assert settings["initial_margin_rate_bps"] == 1000


@pytest.mark.allow_5xx
def test_schedule_cli_failure_with_another_code_is_a_502(
        client, admin, monkeypatch):
    fake_cli(monkeypatch, BTC_ROW)
    refuse_schedule(monkeypatch, "connection reset", 1)
    resp = client.post("/api/config/schedule", data=schedule_form(1))
    assert resp.status_code == 502
    assert "connection reset" in resp.text


@pytest.mark.allow_5xx
def test_schedule_without_postgres_is_a_502_with_the_reason(
        client, admin, mock_postgres_down):
    resp = client.post("/api/config/schedule", data=schedule_form(
        1, min_notional_usd="1"))
    assert resp.status_code == 502
    assert resp.json() == {"error": "rsx-cli: connection refused"}


# ── /v1/symbols carries the minimum ───────────────────────


def symbols_by_name(client):
    body = client.get("/v1/symbols").json()
    return body, {row["symbol"]: row for row in body["symbols"]}


def test_v1_symbols_carries_min_notional_raw_and_usd(client, monkeypatch):
    fake_cli(monkeypatch,
             BTC_ROW | {"min_notional": 20_000_000_000},
             BTC_ROW | {"symbol_id": 10, "symbol_name": "PENGU",
                        "price_decimals": 6, "qty_decimals": 4,
                        "min_notional": 15_000_000_000})
    _, symbols = symbols_by_name(client)
    assert symbols["BTC"]["min_notional"] == 20_000_000_000
    assert symbols["BTC"]["min_notional_usd"] == "2.00"
    assert symbols["PENGU"]["min_notional"] == 15_000_000_000
    assert symbols["PENGU"]["min_notional_usd"] == "1.50"


def test_v1_symbols_omits_a_minimum_the_row_does_not_have(
        client, monkeypatch):
    fake_cli(monkeypatch, BTC_ROW | {"min_notional": None})
    _, symbols = symbols_by_name(client)
    assert "min_notional" not in symbols["BTC"]
    assert "min_notional_usd" not in symbols["BTC"]
    assert symbols["BTC"]["lot_size"] == 100


def test_v1_symbols_reads_the_row_in_force_not_a_future_one(
        client, monkeypatch):
    future = int(time.time() * 1000) + 3_600_000
    fake_cli(monkeypatch, BTC_ROW, BTC_ROW | {
        "config_version": 2, "effective_at_ms": future,
        "min_notional": 90_000_000_000})
    _, symbols = symbols_by_name(client)
    assert symbols["BTC"]["min_notional"] == 10_000_000_000


def test_v1_symbols_lists_only_symbols_with_a_row_in_force(
        client, monkeypatch):
    future = int(time.time() * 1000) + 3_600_000
    fake_cli(monkeypatch, BTC_ROW, BTC_ROW | {
        "symbol_id": 10, "symbol_name": "PENGU", "effective_at_ms": future})
    _, symbols = symbols_by_name(client)
    assert list(symbols) == ["BTC"]
    assert symbols["BTC"]["tick_size"] == 50
    assert symbols["BTC"]["lot_size"] == 100


@pytest.mark.allow_5xx
def test_v1_symbols_without_postgres_is_a_502_with_the_reason(
        client, mock_postgres_down):
    resp = client.get("/v1/symbols")
    assert resp.status_code == 502
    assert resp.json() == {"error": "rsx-cli: connection refused"}


@pytest.mark.allow_5xx
def test_v1_symbols_cli_error_is_reported(client, monkeypatch):
    monkeypatch.setattr(
        risk_cli, "config_show",
        AsyncMock(side_effect=rsx_cli.CliError("connection reset", 1)))
    resp = client.get("/v1/symbols")
    assert resp.status_code == 502
    assert "connection reset" in resp.json()["error"]


def test_v1_symbols_minimum_is_cached_between_requests(client, monkeypatch):
    fake_cli(monkeypatch, BTC_ROW)
    client.get("/v1/symbols")
    client.get("/v1/symbols")
    assert risk_cli.config_show.await_count == 1
