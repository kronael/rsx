DO $migration$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM migrations WHERE id = '002_rename_tables'
    ) THEN

        IF EXISTS (SELECT 1 FROM pg_tables WHERE tablename = 'liquidation_events')
           AND NOT EXISTS (SELECT 1 FROM pg_tables WHERE tablename = 'liquidations') THEN
            ALTER TABLE liquidation_events RENAME TO liquidations;
        END IF;

        IF EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = 'idx_liquidation_user_symbol')
           AND NOT EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = 'idx_liquidations_user_symbol') THEN
            ALTER INDEX idx_liquidation_user_symbol RENAME TO idx_liquidations_user_symbol;
        END IF;

        IF EXISTS (SELECT 1 FROM pg_tables WHERE tablename = 'funding_payments')
           AND NOT EXISTS (SELECT 1 FROM pg_tables WHERE tablename = 'funding') THEN
            ALTER TABLE funding_payments RENAME TO funding;
        END IF;

        IF EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = 'idx_funding_user_symbol')
           AND NOT EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = 'idx_funding_user_symbol_new') THEN
            ALTER INDEX idx_funding_user_symbol RENAME TO idx_funding_user_symbol_new;
        END IF;

        INSERT INTO migrations (id)
            VALUES ('002_rename_tables')
            ON CONFLICT (id) DO NOTHING;

    END IF;
END;
$migration$;
