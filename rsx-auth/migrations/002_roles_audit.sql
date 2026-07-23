DO $migration$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM migrations WHERE id = 'auth_002_roles_audit'
    ) THEN

        -- Audit trail for rsx-auth's own writes (role assignment today;
        -- module distinguishes it from any other service's audit_log
        -- sharing this database). Every write is one row here, inserted
        -- in the same transaction as the business write (spec 7 §6).
        CREATE TABLE IF NOT EXISTS audit_log (
            id              SERIAL PRIMARY KEY,
            module          TEXT NOT NULL,
            actor           INTEGER NOT NULL,
            action          TEXT NOT NULL,
            target          TEXT NOT NULL,
            before_json     JSONB,
            after_json      JSONB,
            reason          TEXT NOT NULL,
            ticket_id       TEXT NOT NULL,
            request_id      TEXT NOT NULL,
            idempotency_key TEXT NOT NULL,
            ts              TIMESTAMPTZ NOT NULL DEFAULT now(),
            UNIQUE (idempotency_key)
        );

        CREATE INDEX IF NOT EXISTS idx_audit_log_actor
            ON audit_log (actor);
        CREATE INDEX IF NOT EXISTS idx_audit_log_target
            ON audit_log (target);

        INSERT INTO migrations (id)
            VALUES ('auth_002_roles_audit')
            ON CONFLICT (id) DO NOTHING;

    END IF;
END;
$migration$;
