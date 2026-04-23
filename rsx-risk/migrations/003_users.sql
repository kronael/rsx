DO $migration$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM migrations WHERE id = '003_users'
    ) THEN

        CREATE TABLE users (
            user_id       SERIAL PRIMARY KEY,
            provider      TEXT NOT NULL,
            provider_sub  TEXT NOT NULL,
            email         TEXT,
            login         TEXT,
            created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
            last_login_at TIMESTAMPTZ,
            UNIQUE (provider, provider_sub)
        );

        -- accounts.user_id FK; if accounts has orphan rows
        -- pre-existing, this will fail — seed users first.
        -- Deferred to guarantee order: migrations run seed
        -- accounts via playground after users exists.
        -- NOTE: FK not enforced in v1 since seed_accounts
        -- inserts arbitrary user_ids. Re-enable after
        -- rsx-auth becomes the only writer.

        INSERT INTO migrations (id)
            VALUES ('003_users');

    END IF;
END;
$migration$;
