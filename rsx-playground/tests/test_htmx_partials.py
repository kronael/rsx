"""Test all HTMX partial endpoints return HTTP 200."""

import pytest
from fastapi.testclient import TestClient
from server import app

client = TestClient(app)

HTMX_PARTIALS = [
    "/x/auth-failures",
    "/x/book",
    "/x/book-stats",
    "/x/cast-flows",
    "/x/control-grid",
    "/x/core-affinity",
    "/x/current-scenario",
    "/x/error-agg",
    "/x/faults-grid",
    "/x/funding",
    "/x/gateway-counters",
    "/x/gateway-rejects",
    "/x/health",
    "/x/invariant-status",
    "/x/key-metrics",
    "/x/latency-regression",
    "/x/liquidations",
    "/x/live-fills",
    "/x/logs",
    "/x/maker-status",
    "/x/logs-tail",
    "/x/margin-ladder",
    "/x/matching-counters",
    "/x/matching-recovery",
    "/x/matching-rejects",
    "/x/order-trace",
    "/x/position-heatmap",
    "/x/proc-chip",
    "/x/processes",
    "/x/recent-orders",
    "/x/reconciliation",
    "/x/resource-usage",
    "/x/ring-pressure",
    "/x/risk-latency",
    "/x/risk-user",
    "/x/session-chip",
    "/x/stale-orders",
    "/x/stress-reports-list",
    "/x/trade-agg",
    "/x/verify",
    "/x/wal-detail",
    "/x/wal-files",
    "/x/wal-lag",
    "/x/wal-rotation",
    "/x/wal-status",
    "/x/wal-timeline",
]


@pytest.mark.parametrize("endpoint", HTMX_PARTIALS)
def test_htmx_partial_returns_200(endpoint):
    """Test that HTMX partial endpoint returns HTTP 200."""
    response = client.get(endpoint)
    assert response.status_code == 200, (
        f"{endpoint} returned {response.status_code}"
    )
    assert response.headers["content-type"].startswith("text/html")


def test_crate_pages_are_the_cargo_workspace():
    """/crates lists exactly the workspace members, no more, no fewer."""
    import tomllib
    from pathlib import Path

    import pages

    cargo = Path(__file__).resolve().parents[2] / "Cargo.toml"
    members = tomllib.loads(cargo.read_text())["workspace"]["members"]
    assert sorted(pages.CRATES) == sorted(members)


def test_component_pages_are_the_exchange_processes():
    """Every exchange process has a component page; the load generator and
    the dashboard itself have their own pages."""
    import pages
    from server import PROC_HINTS

    assert sorted(pages.COMPONENTS) == sorted(
        set(PROC_HINTS) - {"stress", "server"})
