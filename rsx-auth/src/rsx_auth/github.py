"""GitHub OAuth client."""
import httpx

AUTHORIZE_URL = "https://github.com/login/oauth/authorize"
TOKEN_URL = "https://github.com/login/oauth/access_token"
USER_URL = "https://api.github.com/user"
EMAILS_URL = "https://api.github.com/user/emails"


def authorize_url(
    client_id: str, redirect_uri: str, state: str,
    scope: str = "read:user user:email",
) -> str:
    from urllib.parse import urlencode
    q = urlencode({
        "client_id": client_id,
        "redirect_uri": redirect_uri,
        "scope": scope,
        "state": state,
        "allow_signup": "true",
    })
    return f"{AUTHORIZE_URL}?{q}"


async def exchange_code(
    client_id: str,
    client_secret: str,
    code: str,
    redirect_uri: str,
) -> str:
    """Trade authorization code for GitHub access token."""
    async with httpx.AsyncClient(timeout=10) as client:
        r = await client.post(
            TOKEN_URL,
            data={
                "client_id": client_id,
                "client_secret": client_secret,
                "code": code,
                "redirect_uri": redirect_uri,
            },
            headers={"Accept": "application/json"},
        )
        r.raise_for_status()
        payload = r.json()
    token = payload.get("access_token")
    if not token:
        raise ValueError(
            f"github token exchange failed: {payload}")
    return token


async def fetch_user(token: str) -> dict:
    """Return {sub, login, email} from GitHub."""
    headers = {
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
    }
    async with httpx.AsyncClient(timeout=10) as client:
        user_resp = await client.get(USER_URL, headers=headers)
        user_resp.raise_for_status()
        user = user_resp.json()

        email = user.get("email")
        if not email:
            # Primary email may be hidden; fetch /user/emails
            emails_resp = await client.get(
                EMAILS_URL, headers=headers)
            if emails_resp.status_code == 200:
                emails = emails_resp.json()
                for e in emails:
                    if e.get("primary") and e.get("verified"):
                        email = e.get("email")
                        break

    return {
        "sub": str(user["id"]),
        "login": user.get("login"),
        "email": email,
    }
