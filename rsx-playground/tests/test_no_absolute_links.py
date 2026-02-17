"""Regression: rendered HTML must not contain root-absolute href/src.

All internal links must use relative paths (./foo or ../foo).
External resources (https://cdn.*, fonts.googleapis.com, etc.) are OK.
Root-absolute paths (/foo) break when mounted under a sub-path proxy.
"""

import re

import pytest
from fastapi.testclient import TestClient
from server import app

client = TestClient(app)

# All page routes
PAGE_ROUTES = [
    "/",
    "/overview",
    "/topology",
    "/book",
    "/risk",
    "/wal",
    "/logs",
    "/control",
    "/faults",
    "/verify",
    "/orders",
    "/stress",
    "/docs",
]

# All HTMX partial routes
HTMX_PARTIALS = [
    "/x/auth-failures",
    "/x/book",
    "/x/book-stats",
    "/x/cmp-flows",
    "/x/control-grid",
    "/x/core-affinity",
    "/x/current-scenario",
    "/x/error-agg",
    "/x/faults-grid",
    "/x/funding",
    "/x/health",
    "/x/invariant-status",
    "/x/key-metrics",
    "/x/latency-regression",
    "/x/liquidations",
    "/x/live-fills",
    "/x/logs",
    "/x/logs-tail",
    "/x/margin-ladder",
    "/x/order-trace",
    "/x/position-heatmap",
    "/x/processes",
    "/x/recent-orders",
    "/x/reconciliation",
    "/x/resource-usage",
    "/x/ring-pressure",
    "/x/risk-latency",
    "/x/risk-user",
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

# Root-absolute attributes: href="/...", src="/...", hx-get="/...", hx-post="/..."
# Allowlist: protocol-relative (https?://) and data: URIs are fine.
# Slash-only href="/" is also root-absolute and must not appear.
_ABS_ATTR = re.compile(
    r'(?:href|src|hx-get|hx-post|hx-put|hx-delete|hx-patch|action)'
    r'\s*=\s*["\']'
    r'(/(?![/\s])(?!/))'  # starts with / but not // (protocol-relative)
)

# Pattern to find any root-absolute attr value (value starts with /)
_ABS_VALUE = re.compile(
    r'(?:href|src|hx-get|hx-post|hx-put|hx-delete|hx-patch|action)'
    r'\s*=\s*["\']'
    r'(/[^"\']*)'
)

# External URLs that are allowed regardless
_EXTERNAL = re.compile(r'^https?://')


def _find_absolute_links(html: str) -> list[str]:
    """Return list of root-absolute attribute values found in html."""
    hits = []
    for m in _ABS_VALUE.finditer(html):
        val = m.group(1)
        if not _EXTERNAL.match(val) and not val.startswith("//"):
            hits.append(val)
    return hits


@pytest.mark.parametrize("route", PAGE_ROUTES)
def test_page_no_absolute_links(route):
    """Page routes must not contain root-absolute href/src."""
    resp = client.get(route, follow_redirects=True)
    # Skip non-HTML (redirects to /trade/ SPA etc.)
    ct = resp.headers.get("content-type", "")
    if "text/html" not in ct:
        return
    hits = _find_absolute_links(resp.text)
    assert not hits, (
        f"{route}: found root-absolute links (break sub-path proxy): {hits}"
    )


@pytest.mark.parametrize("partial", HTMX_PARTIALS)
def test_partial_no_absolute_links(partial):
    """HTMX partials must not contain root-absolute href/src."""
    resp = client.get(partial)
    ct = resp.headers.get("content-type", "")
    if "text/html" not in ct:
        return
    hits = _find_absolute_links(resp.text)
    assert not hits, (
        f"{partial}: found root-absolute links (break sub-path proxy): {hits}"
    )
