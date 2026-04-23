"""Configuration loaded from environment variables."""
import os
from dataclasses import dataclass


@dataclass(frozen=True)
class Config:
    listen: str
    jwt_secret: str
    jwt_ttl_s: int
    github_client_id: str
    github_client_secret: str
    redirect_uri: str
    starter_collateral: int
    database_url: str
    trade_ui_url: str

    @classmethod
    def from_env(cls) -> "Config":
        return cls(
            listen=os.environ.get(
                "RSX_AUTH_LISTEN", "0.0.0.0:8082"),
            jwt_secret=os.environ.get("RSX_GW_JWT_SECRET", ""),
            jwt_ttl_s=int(os.environ.get(
                "RSX_AUTH_JWT_TTL_S", 7 * 24 * 3600)),
            github_client_id=os.environ.get(
                "RSX_AUTH_GITHUB_CLIENT_ID", ""),
            github_client_secret=os.environ.get(
                "RSX_AUTH_GITHUB_CLIENT_SECRET", ""),
            redirect_uri=os.environ.get(
                "RSX_AUTH_REDIRECT_URI",
                "http://localhost:8082/oauth/github/callback"),
            starter_collateral=int(os.environ.get(
                "RSX_AUTH_STARTER_COLLATERAL", 0)),
            database_url=os.environ.get("DATABASE_URL", ""),
            trade_ui_url=os.environ.get(
                "RSX_AUTH_TRADE_UI_URL",
                "http://localhost:5173/trade"),
        )

    def validate(self) -> list[str]:
        """Return list of missing required config items."""
        missing = []
        if not self.jwt_secret:
            missing.append("RSX_GW_JWT_SECRET")
        if not self.github_client_id:
            missing.append("RSX_AUTH_GITHUB_CLIENT_ID")
        if not self.github_client_secret:
            missing.append("RSX_AUTH_GITHUB_CLIENT_SECRET")
        if not self.database_url:
            missing.append("DATABASE_URL")
        return missing
