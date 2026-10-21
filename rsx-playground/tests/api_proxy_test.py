"""Contract tests for the symbol catalog and the gateway health field.

- /v1/symbols returns the configured symbols (tick, lot, decimals),
  sorted by id; the maker, demo scripts and Playwright specs read it.
- /healthz reports the gateway as reachable or not.

The gateway-down tests need the gateway stopped to exercise the
failure path. When the gate runs after process-control tests, the live
gateway is often still up, so they autoskip when reachable.

Run with:
  cd rsx-playground && .venv/bin/pytest tests/api_proxy_test.py -v
"""

import socket
import urllib.parse
import pytest

import server


def _port_open(host: str, port: int, timeout: float = 0.2) -> bool:
    try:
        with socket.create_connection(
            (host, port), timeout=timeout
        ):
            return True
    except OSError:
        return False


def _url_port(url: str, default: int) -> int:
    """Host port playground targets for `url`."""
    return urllib.parse.urlparse(url).port or default


def _skip_if_gateway_up():
    # Probe the RSX gateway playground targets (GATEWAY_HTTP, :8088 by
    # default). NOT :8080 — arizuko's routd container owns :8080, so a
    # fixed :8080 probe would always see "up" and falsely skip.
    port = _url_port(server.GATEWAY_HTTP, 8088)
    if _port_open("127.0.0.1", port):
        pytest.skip(
            f"live gateway on :{port} — 'gateway down' "
            "tests need it stopped to exercise failure path"
        )


# ── /v1/symbols local endpoint ────────────────────────────


def test_v1_symbols_returns_200(client):
    """/v1/symbols returns 200 with symbol list."""
    resp = client.get("/v1/symbols")
    assert resp.status_code == 200


def test_v1_symbols_returns_json(client):
    """/v1/symbols returns JSON with symbols key."""
    resp = client.get("/v1/symbols")
    body = resp.json()
    assert "symbols" in body
    assert isinstance(body["symbols"], list)


def test_v1_symbols_contains_pengu(client):
    """/v1/symbols includes PENGU entry."""
    resp = client.get("/v1/symbols")
    symbols = {s["symbol"]: s for s in resp.json()["symbols"]}
    assert "PENGU" in symbols


def test_v1_symbols_has_required_fields(client):
    """/v1/symbols entries include required config fields."""
    resp = client.get("/v1/symbols")
    for sym in resp.json()["symbols"]:
        assert "symbol" in sym
        assert "id" in sym
        assert "tick_size" in sym
        assert "lot_size" in sym
        assert "price_decimals" in sym
        assert "qty_decimals" in sym


def test_v1_symbols_sorted_by_id(client):
    """/v1/symbols list is sorted by symbol_id."""
    resp = client.get("/v1/symbols")
    ids = [s["id"] for s in resp.json()["symbols"]]
    assert ids == sorted(ids)


# ── /healthz gateway field ────────────────────────────────


def test_healthz_has_gateway_field(client):
    """/healthz response includes gateway boolean field."""
    resp = client.get("/healthz")
    body = resp.json()
    assert "gateway" in body
    assert isinstance(body["gateway"], bool)


def test_healthz_gateway_false_when_no_gateway(client):
    """/healthz reports gateway=false when the gateway port is closed."""
    _skip_if_gateway_up()
    resp = client.get("/healthz")
    # Gateway not running in test env → False
    assert resp.json()["gateway"] is False
