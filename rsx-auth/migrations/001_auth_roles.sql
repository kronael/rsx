DO $migration$
BEGIN
    -- migrations tracking table (rsx-auth may boot before
    -- rsx-risk has applied its own migrations against this
    -- database, e.g. in tests or a fresh dev DB)
    CREATE TABLE IF NOT EXISTS migrations (
        id   TEXT PRIMARY KEY,
        ts   TIMESTAMPTZ NOT NULL DEFAULT now()
    );

    IF NOT EXISTS (
        SELECT 1 FROM migrations WHERE id = 'auth_001_roles'
    ) THEN

        -- Product-level authorization role (spec 7 dashboard RBAC:
        -- viewer/operator/admin/auditor — rbac.ROLES), distinct from the
        -- infra-admin `_require_admin_request` concept in
        -- rsx-playground and from trading itself. No row = no
        -- management role (deny-by-default; rbac.can(None, ...) is
        -- always False) — so no default value here, a row is only ever
        -- inserted with an explicit role. No FK to users(user_id):
        -- rsx-auth may run its migrations before rsx-risk's 003_users
        -- has landed against a fresh database.
        CREATE TABLE IF NOT EXISTS user_roles (
            user_id    INTEGER PRIMARY KEY,
            role       TEXT NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
        );

        INSERT INTO migrations (id)
            VALUES ('auth_001_roles')
            ON CONFLICT (id) DO NOTHING;

    END IF;
END;
$migration$;
