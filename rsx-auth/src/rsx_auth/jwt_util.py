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
    role: str | None = None,
) -> str:
    """`role` is the management RBAC role (rbac.ROLES, or None — a
    regular trader has no management role at all), read fresh from the
    DB at issuance time (see db.bootstrap_role_from_admin_login). It
    rides in the token so a party with no DB access of its own (e.g. the
    gateway) can trust it without a callback; rsx-auth's own endpoints
    re-check the DB instead of trusting their own claim, since a
    long-lived token can outlive a role change."""
    now = int(time.time())
    claims = {
        "sub": f"{provider}:{provider_sub}",
        "user_id": user_id,
        "email": email,
        "role": role,
        "aud": "rsx-gateway",
        "iss": "rsx-auth",
        "iat": now,
        "exp": now + ttl_s,
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
