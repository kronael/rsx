DO $migration$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM migrations WHERE id = '002_seed_symbols'
    ) THEN

        -- The eight symbols the playground scenarios spawn
        -- (rsx-playground/runtime.py SYMBOLS). Seeded so a fresh
        -- database is immediately usable instead of needing ad-hoc
        -- inserts before the first order.
        INSERT INTO symbol_static (symbol_id, symbol_name, description)
        VALUES
            (1,  'BTC',   'Bitcoin perpetual'),
            (2,  'ETH',   'Ether perpetual'),
            (3,  'SOL',   'Solana perpetual'),
            (10, 'PENGU', 'Pudgy Penguins perpetual'),
            (11, 'WIF',   'dogwifhat perpetual'),
            (12, 'BONK',  'Bonk perpetual'),
            (13, 'PEPE',  'Pepe perpetual'),
            (14, 'DOGE',  'Dogecoin perpetual')
        ON CONFLICT (symbol_id) DO NOTHING;

        -- Version 1, effective at epoch so it is always the active
        -- row. Large caps get 20x (5% initial / 2.5% maintenance);
        -- small caps get 10x (10% / 5%) because they gap harder.
        -- tick/lot/decimals mirror runtime.py exactly.
        INSERT INTO symbol_config_schedule (
            symbol_id, config_version, effective_at_ms,
            tick_size, lot_size, price_decimals, qty_decimals, status,
            min_notional, max_order_qty,
            maker_fee_bps, taker_fee_bps,
            initial_margin_rate_bps, maintenance_margin_rate_bps,
            max_leverage,
            funding_interval_sec, funding_rate_min_bps, funding_rate_max_bps,
            created_at_ms
        )
        VALUES
            (1,  1, 0, 50,   100,    2, 8, 'trading', 1000, 100000000,
             -1, 5,  500, 250, 20, 28800, -75, 75, 0),
            (2,  1, 0, 10,   1000,   2, 8, 'trading', 1000, 100000000,
             -1, 5,  500, 250, 20, 28800, -75, 75, 0),
            (3,  1, 0, 1,    10000,  4, 6, 'trading', 1000, 100000000,
             -1, 5,  500, 250, 20, 28800, -75, 75, 0),
            (10, 1, 0, 1,    100000, 6, 4, 'trading', 1000, 100000000,
             -1, 5, 1000, 500, 10, 28800, -75, 75, 0),
            (11, 1, 0, 1,    100000, 6, 4, 'trading', 1000, 100000000,
             -1, 5, 1000, 500, 10, 28800, -75, 75, 0),
            (12, 1, 0, 1,    100000, 6, 4, 'trading', 1000, 100000000,
             -1, 5, 1000, 500, 10, 28800, -75, 75, 0),
            (13, 1, 0, 1,    100000, 6, 4, 'trading', 1000, 100000000,
             -1, 5, 1000, 500, 10, 28800, -75, 75, 0),
            (14, 1, 0, 1,    100000, 6, 4, 'trading', 1000, 100000000,
             -1, 5, 1000, 500, 10, 28800, -75, 75, 0)
        ON CONFLICT (symbol_id, config_version) DO NOTHING;

        INSERT INTO migrations (id) VALUES ('002_seed_symbols');
    END IF;
END
$migration$;
