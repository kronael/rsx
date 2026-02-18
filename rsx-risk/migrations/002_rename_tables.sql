DO $migration$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM migrations WHERE id = '002_rename_tables'
    ) THEN

        ALTER TABLE liquidation_events RENAME TO liquidations;

        ALTER INDEX idx_liquidation_user_symbol
            RENAME TO idx_liquidations_user_symbol;

        ALTER TABLE funding_payments RENAME TO funding;

        ALTER INDEX idx_funding_user_symbol
            RENAME TO idx_funding_user_symbol_new;

        INSERT INTO migrations (id)
            VALUES ('002_rename_tables');

    END IF;
END;
$migration$;
