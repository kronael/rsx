"""Unit tests for /v1/ REST auth + rate limits.

Tests the verify_jwt dependency and rate limiting
behavior without requiring a running server.
"""
import os
import sys
import time
from pathlib import Path

import pytest
from fastapi import HTTPException
from fastapi.testclient import TestClient


sys.path.insert(0, str(Path(__file__).resolve().parent.parent))


@pytest.fixture(scope="module")
def app_instance():
    """Import server in test mode (no production guard)."""
    os.environ.setdefault("PLAYGROUND_MODE", "local")
    import server
    return server.app


@pytest.fixture
def client(app_instance):
    return TestClient(app_instance)


@pytest.fixture(autouse=True)
def reset_rate_buckets(app_instance):
    """Clear rate limiter state between tests."""
    import server
    server._rate_buckets.clear()
    yield
    server._rate_buckets.clear()


def test_v1_positions_rejects_no_auth(client):
    r = client.get("/v1/positions")
    assert r.status_code == 401
    body = r.json()
    assert "detail" in body
    assert body["detail"]["code"] == "no_auth"


def test_v1_account_rejects_no_auth(client):
    r = client.get("/v1/account")
    assert r.status_code == 401


def test_v1_orders_rejects_no_auth(client):
    r = client.get("/v1/orders")
    assert r.status_code == 401


def test_v1_fills_rejects_no_auth(client):
    r = client.get("/v1/fills")
    assert r.status_code == 401


def test_v1_funding_rejects_no_auth(client):
    r = client.get("/v1/funding")
    assert r.status_code == 401


def test_v1_symbols_public_no_auth_required(client):
    """Public endpoints do not require auth."""
    r = client.get("/v1/symbols")
    assert r.status_code == 200


def test_dev_fallback_x_user_id_accepted_when_no_secret(
    client, monkeypatch,
):
    """When RSX_GW_JWT_SECRET unset, x-user-id header works."""
    import server
    monkeypatch.setattr(server, "JWT_SECRET", "")
    r = client.get("/v1/positions", headers={"x-user-id": "1"})
    assert r.status_code == 200


def test_invalid_bearer_rejected(client):
    r = client.get(
        "/v1/positions",
        headers={"authorization": "Bearer not.a.jwt"},
    )
    assert r.status_code == 401
    assert r.json()["detail"]["code"] in (
        "invalid_token", "no_secret")


def test_valid_jwt_accepted(client, monkeypatch):
    """Well-formed HS256 JWT with user_id claim is accepted."""
    import jwt as pyjwt
    import server
    monkeypatch.setattr(server, "JWT_SECRET", "testsecret")
    token = pyjwt.encode(
        {"user_id": 42}, "testsecret", algorithm="HS256")
    r = client.get(
        "/v1/positions",
        headers={"authorization": f"Bearer {token}"},
    )
    assert r.status_code == 200


def test_sub_claim_fallback_for_user_id(client, monkeypatch):
    """If user_id missing, sub is used as fallback."""
    import jwt as pyjwt
    import server
    monkeypatch.setattr(server, "JWT_SECRET", "testsecret")
    token = pyjwt.encode(
        {"sub": "7"}, "testsecret", algorithm="HS256")
    r = client.get(
        "/v1/account",
        headers={"authorization": f"Bearer {token}"},
    )
    assert r.status_code == 200


def test_rate_limit_triggers_429(client, monkeypatch):
    """60 req/min; 61st fails with 429 + Retry-After."""
    import jwt as pyjwt
    import server
    monkeypatch.setattr(server, "JWT_SECRET", "testsecret")
    token = pyjwt.encode(
        {"user_id": 99}, "testsecret", algorithm="HS256")
    hdr = {"authorization": f"Bearer {token}"}
    for _ in range(server.RATE_LIMIT_PER_MINUTE):
        r = client.get("/v1/symbols")  # no rate limit
        assert r.status_code == 200
    # Now hammer a rate-limited endpoint
    ok_count = 0
    for _ in range(server.RATE_LIMIT_PER_MINUTE + 5):
        r = client.get("/v1/positions", headers=hdr)
        if r.status_code == 200:
            ok_count += 1
        elif r.status_code == 429:
            break
    assert ok_count <= server.RATE_LIMIT_PER_MINUTE
    # Next request must be 429
    r = client.get("/v1/positions", headers=hdr)
    assert r.status_code == 429
    body = r.json()
    assert body["detail"]["code"] == "rate_limited"
    assert body["detail"]["retry_after_s"] >= 1
