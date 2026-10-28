"""Isolation conftest for live-cluster scenario tests.

The parent tests/conftest.py installs per-test fixtures that import and
reset the in-process `server` app. pytest resolves fixtures from the
nearest conftest, so redefining them here (same names) overrides them
with no-ops for every test under tests/live/. These tests talk to the
live dashboard over HTTP only; they never import or mutate `server`.
"""

import pytest


@pytest.fixture(autouse=True)
def cleanup_state():
    # No-op: live tests own no in-process server state.
    yield


@pytest.fixture(autouse=True)
def mock_gateway_running():
    # No-op: the live gateway is real; do not patch scan_processes.
    yield


@pytest.fixture(autouse=True)
def track_5xx():
    # No-op: live tests use httpx directly and assert status codes
    # themselves; the parent 5xx-fail-fast plugin patches the
    # in-process TestClient, which these tests never use.
    yield
