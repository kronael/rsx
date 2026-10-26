DO $migration$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM migrations WHERE id = '008_funding_intervals'
    ) THEN

        -- One row per settled (shard, symbol, interval). Append-only:
        -- the primary key makes a second settlement of the same
        -- interval fail the transaction that carries it, and the last
        -- settled interval is max(interval_id).
        CREATE TABLE IF NOT EXISTS funding_intervals (
            shard_id    INT         NOT NULL,
            symbol_id   INT         NOT NULL,
            interval_id BIGINT      NOT NULL,
            settled_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
            PRIMARY KEY (shard_id, symbol_id, interval_id)
        );

        -- Payments written before interval ids were recorded keep a
        -- NULL interval_id and stay outside both indexes.
        ALTER TABLE funding ADD COLUMN IF NOT EXISTS interval_id BIGINT;

        CREATE UNIQUE INDEX IF NOT EXISTS idx_funding_user_symbol_interval
            ON funding (user_id, symbol_id, interval_id)
            WHERE interval_id IS NOT NULL;

        CREATE INDEX IF NOT EXISTS idx_funding_symbol_interval
            ON funding (symbol_id, interval_id)
            WHERE interval_id IS NOT NULL;

        INSERT INTO migrations (id)
            VALUES ('008_funding_intervals')
            ON CONFLICT (id) DO NOTHING;

    END IF;
END;
$migration$;
