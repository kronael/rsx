DO $migration$
BEGIN
    -- migrations tracking table
    CREATE TABLE IF NOT EXISTS migrations (
        id   TEXT PRIMARY KEY,
        ts   TIMESTAMPTZ NOT NULL DEFAULT now()
    );

    IF NOT EXISTS (
        SELECT 1 FROM migrations WHERE id = '001_symbol_config'
    ) THEN

        CREATE TABLE symbol_static (
            symbol_id   INT PRIMARY KEY,
            symbol_name TEXT NOT NULL,
            description TEXT
        );

        CREATE TABLE symbol_config_schedule (
            symbol_id                   INT    NOT NULL,
            config_version              BIGINT NOT NULL,
            effective_at_ms             BIGINT NOT NULL,
            tick_size                   BIGINT NOT NULL,
            lot_size                    BIGINT NOT NULL,
            price_decimals              SMALLINT NOT NULL,
            qty_decimals                SMALLINT NOT NULL,
            status                      TEXT   NOT NULL,
            min_notional                BIGINT,
            max_order_qty               BIGINT,
            maker_fee_bps               INT,
            taker_fee_bps               INT,
            initial_margin_rate_bps     INT,
            maintenance_margin_rate_bps INT,
            max_leverage                INT,
            funding_interval_sec        INT,
            funding_rate_min_bps        INT,
            funding_rate_max_bps        INT,
            created_at_ms               BIGINT NOT NULL,
            PRIMARY KEY (symbol_id, config_version)
        );

        CREATE INDEX idx_schedule_effective
            ON symbol_config_schedule (symbol_id, effective_at_ms);

        CREATE TABLE symbol_config_applied (
            symbol_id                   INT    NOT NULL PRIMARY KEY,
            config_version              BIGINT NOT NULL,
            effective_at_ms             BIGINT NOT NULL,
            applied_at_ns               BIGINT NOT NULL,
            tick_size                   BIGINT NOT NULL,
            lot_size                    BIGINT NOT NULL,
            price_decimals              SMALLINT NOT NULL,
            qty_decimals                SMALLINT NOT NULL,
            status                      TEXT   NOT NULL,
            min_notional                BIGINT,
            max_order_qty               BIGINT,
            maker_fee_bps               INT,
            taker_fee_bps               INT,
            initial_margin_rate_bps     INT,
            maintenance_margin_rate_bps INT,
            max_leverage                INT,
            funding_interval_sec        INT,
            funding_rate_min_bps        INT,
            funding_rate_max_bps        INT
        );

        INSERT INTO migrations (id) VALUES ('001_symbol_config');
    END IF;
END
$migration$;
