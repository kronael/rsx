"""Symbol risk-config tab: admin gate, active-config read, scheduling,
and the minimum order notional (raw risk units in Postgres, USD at the
edge)."""

import time

import pytest
import server


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
    """symbol_config_schedule in memory behind server.pg_query."""

    def __init__(self, *rows):
        self.rows = [dict(row) for row in rows]
        self.inserts = []

    def latest(self, symbol_id):
        mine = [r for r in self.rows if r["symbol_id"] == symbol_id]
        return max(mine, key=lambda r: r["config_version"], default=None)

    def in_force(self, now_ms):
        found = {}
        for row in sorted(self.rows, key=lambda r: r["config_version"]):
            if row["effective_at_ms"] <= now_ms:
                found[row["symbol_id"]] = row
        return list(found.values())

    async def __call__(self, sql, *args):
        if sql == server.SYMBOL_DECIMALS_SQL:
            row = self.latest(args[0])
            return [] if row is None else [{
                "price_decimals": row["price_decimals"],
                "qty_decimals": row["qty_decimals"],
            }]
        if sql == server.ACTIVE_CONFIG_SQL:
            ids = sorted({r["symbol_id"] for r in self.rows})
            return [dict(self.latest(sid)) for sid in ids]
        if sql == server.IN_FORCE_MIN_NOTIONAL_SQL:
            return [dict(row) for row in self.in_force(args[0])]
        if sql == server.IN_FORCE_SYMBOLS_SQL:
            return [{
                "id": row["symbol_id"], "name": row["symbol_name"],
                "tick": row["tick_size"], "lot": row["lot_size"],
                "price_dec": row["price_decimals"],
                "qty_dec": row["qty_decimals"],
            } for row in self.in_force(args[0])]
        assert sql.lstrip().startswith("INSERT INTO symbol_config_schedule")
        self.inserts.append(args)
        symbol_id, effective_at, initial, maint, leverage, created, \
            min_notional = args
        base = self.latest(symbol_id)
        if base is None:
            return []
        row = base | {
            "config_version": base["config_version"] + 1,
            "effective_at_ms": effective_at,
            "initial_margin_rate_bps": initial,
            "maintenance_margin_rate_bps": maint,
            "max_leverage": leverage, "created_at_ms": created,
            "min_notional": base["min_notional"]
            if min_notional is None else min_notional,
        }
        self.rows.append(row)
        return [{"config_version": row["config_version"]}]


@pytest.fixture
def admin(monkeypatch):
    async def fake_role(request):
        return "admin"

    monkeypatch.setattr(server, "_caller_role", fake_role)
    monkeypatch.setattr(server, "_TTL_CACHE", {})


def fake_pg(monkeypatch, *rows):
    schedule = FakeSchedule(*rows)
    monkeypatch.setattr(server, "pg_query", schedule)
    return schedule


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


def test_config_view_reads_min_order_from_postgres(
        client, admin, monkeypatch):
    fake_pg(monkeypatch, BTC_ROW, ODD_ROW)
    resp = client.get("/config")
    assert resp.status_code == 200
    assert 'data-testid="min-order-1">1.00<' in resp.text
    assert 'data-testid="min-order-99">1.50<' in resp.text


def test_schedule_writes_two_usd_as_raw_integer(client, admin, monkeypatch):
    """2 USD on BTC (2 + 8 decimals) is stored as 2 x 10^10."""
    pg = fake_pg(monkeypatch, BTC_ROW)
    resp = client.post("/api/config/schedule", data=schedule_form(
        1, min_notional_usd="2"))
    assert resp.status_code == 200
    assert "scheduled v2" in resp.text
    assert "2.00 USD" in resp.text
    raw = pg.inserts[0][-1]
    assert raw == 20_000_000_000
    assert type(raw) is int
    assert pg.latest(1)["min_notional"] == 20_000_000_000
    # The previous row is still there: a new version, no update.
    assert pg.rows[0]["min_notional"] == 10_000_000_000
    page = client.get("/config")
    assert 'data-testid="min-order-1">2.00<' in page.text


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
    pg = fake_pg(monkeypatch, BTC_ROW, ODD_ROW)
    resp = client.post("/api/config/schedule", data=schedule_form(
        symbol_row["symbol_id"], min_notional_usd=usd))
    assert resp.status_code == 200, resp.text
    assert pg.inserts[0][-1] == raw


def test_schedule_blank_min_order_keeps_the_current_value(
        client, admin, monkeypatch):
    """A margin-only change must not reset a raised minimum."""
    pg = fake_pg(monkeypatch, BTC_ROW | {"min_notional": 50_000_000_000})
    resp = client.post("/api/config/schedule", data=schedule_form(
        1, min_notional_usd=""))
    assert resp.status_code == 200
    assert pg.inserts[0][-1] is None
    assert pg.latest(1)["min_notional"] == 50_000_000_000


@pytest.mark.parametrize("usd", [
    "-1", "-0.01", "0", "0.0", "abc", "1,5", "nan", "inf", "-inf",
])
def test_schedule_rejects_non_positive_or_non_numeric_usd(
        client, admin, monkeypatch, usd):
    pg = fake_pg(monkeypatch, BTC_ROW)
    resp = client.post("/api/config/schedule", data=schedule_form(
        1, min_notional_usd=usd))
    assert resp.status_code == 400
    assert "minimum order must be" in resp.text
    assert pg.inserts == []


@pytest.mark.parametrize("usd", ["0.00000000004", "1e-999999999"])
def test_schedule_rejects_usd_below_the_smallest_unit(
        client, admin, monkeypatch, usd):
    """Under half a raw unit rounds to zero, which risk refuses."""
    pg = fake_pg(monkeypatch, BTC_ROW)
    resp = client.post("/api/config/schedule", data=schedule_form(
        1, min_notional_usd=usd))
    assert resp.status_code == 400
    assert "smaller than" in resp.text
    assert pg.inserts == []


@pytest.mark.parametrize("usd", [
    "922337203.6854775808", "922337203.7", "1e999999999"])
def test_schedule_rejects_usd_beyond_i64(client, admin, monkeypatch, usd):
    pg = fake_pg(monkeypatch, BTC_ROW)
    resp = client.post("/api/config/schedule", data=schedule_form(
        1, min_notional_usd=usd))
    assert resp.status_code == 400
    assert "too large" in resp.text
    assert pg.inserts == []


def test_schedule_min_order_unknown_symbol_is_404(
        client, admin, monkeypatch):
    pg = fake_pg(monkeypatch, BTC_ROW)
    resp = client.post("/api/config/schedule", data=schedule_form(
        77, min_notional_usd="1"))
    assert resp.status_code == 404
    assert "unknown symbol 77" in resp.text
    assert pg.inserts == []


@pytest.mark.allow_5xx
def test_schedule_min_order_without_postgres_is_503(
        client, admin, mock_postgres_down):
    resp = client.post("/api/config/schedule", data=schedule_form(
        1, min_notional_usd="1"))
    assert resp.status_code == 503
    assert "postgres not connected" in resp.text


@pytest.mark.allow_5xx
def test_schedule_min_order_query_error_is_500_with_the_reason(
        client, admin, monkeypatch):
    async def failing(sql, *args):
        return {"error": "connection reset"}

    monkeypatch.setattr(server, "pg_query", failing)
    resp = client.post("/api/config/schedule", data=schedule_form(
        1, min_notional_usd="1"))
    assert resp.status_code == 500
    assert "connection reset" in resp.text


# ── /v1/symbols carries the minimum ───────────────────────


@pytest.fixture
def fresh_cache(monkeypatch):
    monkeypatch.setattr(server, "_TTL_CACHE", {})


def symbols_by_name(client):
    body = client.get("/v1/symbols").json()
    return body, {row["symbol"]: row for row in body["symbols"]}


def test_v1_symbols_carries_min_notional_raw_and_usd(
        client, fresh_cache, monkeypatch):
    fake_pg(monkeypatch,
            BTC_ROW | {"min_notional": 20_000_000_000},
            BTC_ROW | {"symbol_id": 10, "symbol_name": "PENGU",
                       "price_decimals": 6, "qty_decimals": 4,
                       "min_notional": 15_000_000_000})
    body, symbols = symbols_by_name(client)
    assert symbols["BTC"]["min_notional"] == 20_000_000_000
    assert symbols["BTC"]["min_notional_usd"] == "2.00"
    assert symbols["PENGU"]["min_notional"] == 15_000_000_000
    assert symbols["PENGU"]["min_notional_usd"] == "1.50"
    assert "min_notional_error" not in body


def test_v1_symbols_reads_the_row_in_force_not_a_future_one(
        client, fresh_cache, monkeypatch):
    future = int(time.time() * 1000) + 3_600_000
    fake_pg(monkeypatch, BTC_ROW, BTC_ROW | {
        "config_version": 2, "effective_at_ms": future,
        "min_notional": 90_000_000_000})
    _, symbols = symbols_by_name(client)
    assert symbols["BTC"]["min_notional"] == 10_000_000_000


def test_v1_symbols_lists_only_symbols_with_a_row_in_force(
        client, fresh_cache, monkeypatch):
    future = int(time.time() * 1000) + 3_600_000
    fake_pg(monkeypatch, BTC_ROW, BTC_ROW | {
        "symbol_id": 10, "symbol_name": "PENGU", "effective_at_ms": future})
    _, symbols = symbols_by_name(client)
    assert list(symbols) == ["BTC"]
    assert symbols["BTC"]["tick_size"] == 50
    assert symbols["BTC"]["lot_size"] == 100


def test_v1_symbols_without_postgres_omits_the_minimum_and_says_why(
        client, fresh_cache, mock_postgres_down):
    body, symbols = symbols_by_name(client)
    assert body["min_notional_error"] == "postgres not connected"
    assert all("min_notional" not in row for row in symbols.values())
    assert symbols["BTC"]["lot_size"] == 100


def test_v1_symbols_query_error_is_reported(
        client, fresh_cache, monkeypatch):
    async def failing(sql, *args):
        return {"error": "connection reset"}

    monkeypatch.setattr(server, "pg_query", failing)
    body, symbols = symbols_by_name(client)
    assert body["min_notional_error"] == "connection reset"
    assert all("min_notional" not in row for row in symbols.values())


def test_v1_symbols_minimum_is_cached_between_requests(
        client, fresh_cache, monkeypatch):
    pg = fake_pg(monkeypatch, BTC_ROW)
    calls = []
    original = pg.__call__

    async def counting(sql, *args):
        calls.append(sql)
        return await original(sql, *args)

    monkeypatch.setattr(server, "pg_query", counting)
    client.get("/v1/symbols")
    client.get("/v1/symbols")
    assert calls.count(server.IN_FORCE_MIN_NOTIONAL_SQL) == 1
