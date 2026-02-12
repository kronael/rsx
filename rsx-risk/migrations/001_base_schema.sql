DO $migration$
BEGIN
    -- migrations tracking table
    CREATE TABLE IF NOT EXISTS migrations (
        id   TEXT PRIMARY KEY,
        ts   TIMESTAMPTZ NOT NULL DEFAULT now()
    );

    IF NOT EXISTS (
        SELECT 1 FROM migrations WHERE id = '001_base_schema'
    ) THEN

        CREATE TABLE positions (
            user_id          INT     NOT NULL,
            symbol_id        INT     NOT NULL,
            long_qty         BIGINT  NOT NULL DEFAULT 0,
            short_qty        BIGINT  NOT NULL DEFAULT 0,
            long_entry_cost  BIGINT  NOT NULL DEFAULT 0,
            short_entry_cost BIGINT  NOT NULL DEFAULT 0,
            realized_pnl     BIGINT  NOT NULL DEFAULT 0,
            last_fill_seq    BIGINT  NOT NULL DEFAULT 0,
            version          BIGINT  NOT NULL DEFAULT 0,
            PRIMARY KEY (user_id, symbol_id)
        );

        CREATE TABLE accounts (
            user_id       INT    NOT NULL PRIMARY KEY,
            collateral    BIGINT NOT NULL DEFAULT 0,
            frozen_margin BIGINT NOT NULL DEFAULT 0,
            version       BIGINT NOT NULL DEFAULT 0
        );

        CREATE TABLE fills (
            symbol_id      INT     NOT NULL,
            taker_user_id  INT     NOT NULL,
            maker_user_id  INT     NOT NULL,
            price          BIGINT  NOT NULL,
            qty            BIGINT  NOT NULL,
            taker_fee      BIGINT  NOT NULL DEFAULT 0,
            maker_fee      BIGINT  NOT NULL DEFAULT 0,
            taker_side     SMALLINT NOT NULL,
            seq            BIGINT  NOT NULL,
            timestamp_ns   BIGINT  NOT NULL
        );

        CREATE UNIQUE INDEX idx_fills_symbol_seq
            ON fills (symbol_id, seq);

        CREATE TABLE tips (
            instance_id INT NOT NULL,
            symbol_id   INT NOT NULL,
            last_seq    BIGINT NOT NULL DEFAULT 0,
            PRIMARY KEY (instance_id, symbol_id)
        );

        CREATE TABLE funding_payments (
            user_id       INT    NOT NULL,
            symbol_id     INT    NOT NULL,
            amount        BIGINT NOT NULL,
            rate          BIGINT NOT NULL,
            settlement_ts BIGINT NOT NULL
        );

        CREATE INDEX idx_funding_user_symbol
            ON funding_payments (user_id, symbol_id);

        CREATE TABLE insurance_fund (
            symbol_id INT    NOT NULL PRIMARY KEY,
            balance   BIGINT NOT NULL DEFAULT 0,
            version   BIGINT NOT NULL DEFAULT 0
        );

        CREATE TABLE liquidation_events (
            user_id      INT     NOT NULL,
            symbol_id    INT     NOT NULL,
            round        INT     NOT NULL,
            side         SMALLINT NOT NULL,
            price        BIGINT  NOT NULL,
            qty          BIGINT  NOT NULL,
            slippage_bps INT     NOT NULL,
            status       SMALLINT NOT NULL,
            timestamp_ns BIGINT  NOT NULL
        );

        CREATE INDEX idx_liquidation_user_symbol
            ON liquidation_events (user_id, symbol_id);

        INSERT INTO migrations (id)
            VALUES ('001_base_schema');

    END IF;
END;
$migration$;
