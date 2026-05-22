"""JWT issuance (HS256, same secret as gateway)."""
import time
import uuid
import jwt as pyjwt


def issue(
    user_id: int,
    provider: str,
    provider_sub: str,
    email: str | None,
    secret: str,
    ttl_s: int,
) -> str:
    now = int(time.time())
    claims = {
        "sub": f"{provider}:{provider_sub}",
        "user_id": user_id,
        "email": email,
        "aud": "rsx-gateway",
        "iss": "rsx-auth",
        "iat": now,
        "exp": now + ttl_s,
        # Per-token unique id, consumed by the gateway's
        # JtiTracker to reject replay. See
        # rsx-gateway/src/jwt.rs::JtiTracker.
        "jti": uuid.uuid4().hex,
    }
    return pyjwt.encode(claims, secret, algorithm="HS256")


def verify(token: str, secret: str) -> dict:
    """Raises pyjwt.InvalidTokenError on failure."""
    return pyjwt.decode(
        token,
        secret,
        algorithms=["HS256"],
        audience="rsx-gateway",
        issuer="rsx-auth",
    )
