"""JWT issuance (HS256, same secret as gateway)."""
import time
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
        "iat": now,
        "exp": now + ttl_s,
    }
    return pyjwt.encode(claims, secret, algorithm="HS256")


def verify(token: str, secret: str) -> dict:
    """Raises pyjwt.InvalidTokenError on failure."""
    return pyjwt.decode(token, secret, algorithms=["HS256"])
