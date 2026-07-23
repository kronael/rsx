"""Postgres access: users + roles + audit."""
import json
from pathlib import Path

import asyncpg

from . import rbac

POOL: asyncpg.Pool | None = None

MIGRATIONS_DIR = Path(__file__).resolve().parents[2] / "migrations"


async def init_pool(database_url: str) -> None:
    global POOL
    POOL = await asyncpg.create_pool(
        database_url, min_size=1, max_size=5)
    await apply_migrations()


async def apply_migrations() -> None:
    """Apply rsx-auth's own migrations (idempotent, version-guarded
    per rsx-auth/migrations/CLAUDE.md conventions)."""
    assert POOL is not None
    async with POOL.acquire() as conn:
        for path in sorted(MIGRATIONS_DIR.glob("*.sql")):
            await conn.execute(path.read_text())


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
                (user_id, collateral, version)
            VALUES ($1, $2, 0)
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


async def get_role(user_id: int) -> str | None:
    """Return the user's management role, or None if no row — a normal
    trader has no row and therefore no management role at all
    (deny-by-default; see rbac.can)."""
    assert POOL is not None
    async with POOL.acquire() as conn:
        row = await conn.fetchrow(
            "SELECT role FROM user_roles WHERE user_id = $1", user_id)
        return row["role"] if row else None


async def set_role(
    user_id: int,
    role: str,
    conn: asyncpg.Connection | None = None,
) -> str:
    """Insert or update a user's role. Caller must validate `role`
    against rbac.ROLES first. Pass `conn` to compose inside a caller's
    own transaction (see set_role_audited); omitted, acquires its own."""
    assert role in rbac.ROLES

    async def _do(c: asyncpg.Connection) -> str:
        row = await c.fetchrow(
            """
            INSERT INTO user_roles (user_id, role, updated_at)
            VALUES ($1, $2, now())
            ON CONFLICT (user_id) DO UPDATE SET
                role = EXCLUDED.role,
                updated_at = now()
            RETURNING role
            """,
            user_id, role,
        )
        return row["role"]

    if conn is not None:
        return await _do(conn)
    assert POOL is not None
    async with POOL.acquire() as c:
        return await _do(c)


async def set_role_audited(
    *,
    actor: int,
    target_user_id: int,
    role: str,
    reason: str,
    ticket_id: str,
    request_id: str,
    idempotency_key: str,
) -> str:
    """Set a user's role and insert the audit row atomically (spec 7
    §6). Caller must validate `role` against rbac.ROLES and that `actor`
    is authorized (rbac.can(actor_role, "manage_roles")) first."""
    assert POOL is not None
    async with POOL.acquire() as conn:
        async with conn.transaction():
            before_row = await conn.fetchrow(
                "SELECT role FROM user_roles WHERE user_id = $1",
                target_user_id)
            before = before_row["role"] if before_row else None
            after = await set_role(target_user_id, role, conn=conn)
            await insert_audit(
                conn,
                module="roles",
                actor=actor,
                action="set_role",
                target=str(target_user_id),
                before={"role": before},
                after={"role": after},
                reason=reason,
                ticket_id=ticket_id,
                request_id=request_id,
                idempotency_key=idempotency_key,
            )
            return after


async def bootstrap_role_from_admin_login(
    user_id: int,
    login: str | None,
    admin_logins: frozenset[str],
) -> str | None:
    """Called on every OAuth login. A login on the trusted admin list is
    (re-)upserted to 'admin' — self-healing, matches "trusted admin
    list" semantics. A login REMOVED from the env var does not
    auto-revoke an existing DB row (that's a `set_role` action, done
    deliberately, not an env-var side effect). Returns the caller's
    current role (possibly unchanged) for embedding in the fresh JWT."""
    if login and login.lower() in admin_logins:
        return await set_role(user_id, rbac.ADMIN)
    return await get_role(user_id)


async def list_users(
    query: str | None = None,
    limit: int = 50,
    cursor: int = 0,
) -> list[dict]:
    """Users joined with their role (None = no management role),
    optionally filtered by a case-insensitive login/email substring,
    paginated by ascending user_id cursor."""
    assert POOL is not None
    limit = max(1, min(limit, 200))
    async with POOL.acquire() as conn:
        rows = await conn.fetch(
            """
            SELECT u.user_id, u.provider, u.email, u.login,
                   u.created_at, u.last_login_at, r.role
            FROM users u
            LEFT JOIN user_roles r ON r.user_id = u.user_id
            WHERE u.user_id > $1
              AND ($2::text IS NULL
                   OR u.login ILIKE '%' || $2 || '%'
                   OR u.email ILIKE '%' || $2 || '%')
            ORDER BY u.user_id
            LIMIT $3
            """,
            cursor, query, limit,
        )
        return [dict(row) for row in rows]


async def insert_audit(
    conn: asyncpg.Connection,
    *,
    module: str,
    actor: int,
    action: str,
    target: str,
    before: dict | None,
    after: dict | None,
    reason: str,
    ticket_id: str,
    request_id: str,
    idempotency_key: str,
) -> None:
    """Insert one audit row on the given connection — callers compose
    this inside their own transaction so the business write and the
    audit row commit together (spec 7 §6). A replayed idempotency_key
    (retried request) is a no-op, not a duplicate row."""
    await conn.execute(
        """
        INSERT INTO audit_log
            (module, actor, action, target, before_json, after_json,
             reason, ticket_id, request_id, idempotency_key)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
        ON CONFLICT (idempotency_key) DO NOTHING
        """,
        module, actor, action, target,
        json.dumps(before) if before is not None else None,
        json.dumps(after) if after is not None else None,
        reason, ticket_id, request_id, idempotency_key,
    )


async def list_audit(
    actor: int | None = None,
    target: str | None = None,
    limit: int = 50,
    cursor: int = 0,
) -> list[dict]:
    """rsx-auth's own audit trail (module='roles' — role assignment).
    Other modules, if this database ever carries them, are out of
    scope here."""
    assert POOL is not None
    limit = max(1, min(limit, 200))
    async with POOL.acquire() as conn:
        rows = await conn.fetch(
            """
            SELECT id, module, actor, action, target, before_json,
                   after_json, reason, ticket_id, request_id,
                   idempotency_key, ts
            FROM audit_log
            WHERE id > $1
              AND ($2::int IS NULL OR actor = $2)
              AND ($3::text IS NULL OR target = $3)
              AND module = 'roles'
            ORDER BY id
            LIMIT $4
            """,
            cursor, actor, target, limit,
        )
        return [dict(row) for row in rows]
