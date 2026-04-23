"""Postgres access: users + accounts."""
import asyncpg

POOL: asyncpg.Pool | None = None


async def init_pool(database_url: str) -> None:
    global POOL
    POOL = await asyncpg.create_pool(
        database_url, min_size=1, max_size=5)


async def close_pool() -> None:
    global POOL
    if POOL:
        await POOL.close()
        POOL = None


async def upsert_user(
    provider: str,
    provider_sub: str,
    email: str | None,
    login: str | None,
) -> int:
    """Insert or update user row; return internal user_id."""
    assert POOL is not None
    async with POOL.acquire() as conn:
        row = await conn.fetchrow(
            """
            INSERT INTO users (provider, provider_sub, email, login,
                               last_login_at)
            VALUES ($1, $2, $3, $4, now())
            ON CONFLICT (provider, provider_sub)
            DO UPDATE SET
                email = EXCLUDED.email,
                login = EXCLUDED.login,
                last_login_at = now()
            RETURNING user_id
            """,
            provider, provider_sub, email, login,
        )
        return int(row["user_id"])


async def seed_account_if_missing(
    user_id: int, starter_collateral: int,
) -> None:
    """Create accounts row on first login; no-op if exists."""
    assert POOL is not None
    async with POOL.acquire() as conn:
        await conn.execute(
            """
            INSERT INTO accounts
                (user_id, collateral, frozen_margin, version)
            VALUES ($1, $2, 0, 0)
            ON CONFLICT (user_id) DO NOTHING
            """,
            user_id, starter_collateral,
        )


async def get_user(user_id: int) -> dict | None:
    assert POOL is not None
    async with POOL.acquire() as conn:
        row = await conn.fetchrow(
            "SELECT user_id, provider, provider_sub, email, "
            "login, created_at, last_login_at "
            "FROM users WHERE user_id = $1",
            user_id)
        return dict(row) if row else None
