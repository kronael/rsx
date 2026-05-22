"""Unit tests for JWT issuance + verification."""
import time
import pytest
import jwt as pyjwt

from rsx_auth import jwt_util


SECRET = "a-test-secret-at-least-32-bytes-long-!!"


def test_issue_and_verify_roundtrip():
    token = jwt_util.issue(
        user_id=42,
        provider="github",
        provider_sub="12345",
        email="user@example.com",
        secret=SECRET,
        ttl_s=3600,
    )
    claims = jwt_util.verify(token, SECRET)
    assert claims["user_id"] == 42
    assert claims["sub"] == "github:12345"
    assert claims["email"] == "user@example.com"
    assert claims["exp"] > claims["iat"]


def test_verify_rejects_bad_secret():
    token = jwt_util.issue(
        user_id=1, provider="github", provider_sub="1",
        email=None, secret=SECRET, ttl_s=3600)
    with pytest.raises(pyjwt.InvalidTokenError):
        jwt_util.verify(token, "wrong-secret")


def test_verify_rejects_expired():
    token = jwt_util.issue(
        user_id=1, provider="github", provider_sub="1",
        email=None, secret=SECRET, ttl_s=-10)
    with pytest.raises(pyjwt.ExpiredSignatureError):
        jwt_util.verify(token, SECRET)


def test_verify_rejects_garbage():
    with pytest.raises(pyjwt.InvalidTokenError):
        jwt_util.verify("not.a.jwt", SECRET)


def test_null_email_is_allowed():
    token = jwt_util.issue(
        user_id=7, provider="github", provider_sub="7",
        email=None, secret=SECRET, ttl_s=3600)
    claims = jwt_util.verify(token, SECRET)
    assert claims["email"] is None


def test_issue_emits_unique_jti():
    """Every minted token carries a unique `jti` claim so the
    gateway's JtiTracker can reject replay (CTO-REPORT.md R3,
    SYNTHESIS.md F2.1)."""
    t1 = jwt_util.issue(
        user_id=1, provider="github", provider_sub="1",
        email=None, secret=SECRET, ttl_s=3600)
    t2 = jwt_util.issue(
        user_id=1, provider="github", provider_sub="1",
        email=None, secret=SECRET, ttl_s=3600)
    c1 = jwt_util.verify(t1, SECRET)
    c2 = jwt_util.verify(t2, SECRET)
    assert "jti" in c1 and isinstance(c1["jti"], str)
    assert "jti" in c2 and isinstance(c2["jti"], str)
    assert c1["jti"] != c2["jti"]
    # uuid4().hex => 32 lowercase hex chars
    assert len(c1["jti"]) == 32
