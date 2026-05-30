"""Isolation conftest for live-cluster scenario tests.

The parent tests/conftest.py installs a SESSION-autouse fixture
(`cleanup_session`) that kills every rsx-* process at session start
and end, plus per-test fixtures that import + reset the in-process
`server` app. Both are fatal for tests that drive the ALREADY-RUNNING
live cluster: the kill fixture would tear the cluster down before the
test runs.

pytest resolves fixtures from the nearest conftest, so redefining the
parent fixtures here (same names) overrides them with no-ops for every
test under tests/live/. These tests talk to the live dashboard over
HTTP only; they never import or mutate `server`.
"""

import pytest


@pytest.fixture(scope="session", autouse=True)
def cleanup_session():
    # No-op: do NOT kill the live cluster.
    yield


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
