"""Symbol risk-config tab: admin gate, active-config read, scheduling."""


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
