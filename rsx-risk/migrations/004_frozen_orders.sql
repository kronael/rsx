DO $migration$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM migrations WHERE id = '004_frozen_orders'
    ) THEN

        -- Per-order frozen margin reservations. Source of
        -- truth for "what margin is locked for which open
        -- order". Account-level aggregate is derived
        -- (sum over user_id) and no longer persisted.
        CREATE TABLE IF NOT EXISTS frozen_orders (
            user_id      INT     NOT NULL,
            order_id_hi  BIGINT  NOT NULL,
            order_id_lo  BIGINT  NOT NULL,
            symbol_id    INT     NOT NULL,
            amount       BIGINT  NOT NULL,
            PRIMARY KEY (user_id, order_id_hi, order_id_lo)
        );

        CREATE INDEX IF NOT EXISTS idx_frozen_orders_user
            ON frozen_orders (user_id);

        ALTER TABLE accounts DROP COLUMN IF EXISTS frozen_margin;

        INSERT INTO migrations (id)
            VALUES ('004_frozen_orders')
            ON CONFLICT (id) DO NOTHING;

    END IF;
END;
$migration$;
