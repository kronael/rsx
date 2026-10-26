DO $migration$
DECLARE
    latest_var symbol_config_schedule%ROWTYPE;
    floor_var  BIGINT;
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM migrations WHERE id = '007_min_order_notional'
    ) THEN

        -- 1 USD minimum order notional on every symbol. min_notional is in
        -- risk's notional unit, price * qty = 10^-(price_decimals +
        -- qty_decimals) quote, so 1 USD is 10^(price_decimals +
        -- qty_decimals) raw: 10_000_000_000 for every preset.
        --
        -- Per symbol, the highest-version row is copied into version
        -- max + 1 with only min_notional changed, and only when that row's
        -- minimum is NULL or below 1 USD. A higher operator-set minimum is
        -- kept. The highest version (not the active one) is the base so a
        -- future-dated version is never overtaken. The copy keeps the base
        -- row's effective_at_ms.
        FOR latest_var IN
            SELECT DISTINCT ON (symbol_id) *
            FROM symbol_config_schedule
            ORDER BY symbol_id, config_version DESC
        LOOP
            floor_var := (10::numeric ^ (latest_var.price_decimals + latest_var.qty_decimals))::bigint;
            IF latest_var.min_notional IS NULL OR latest_var.min_notional < floor_var THEN
                INSERT INTO symbol_config_schedule (
                    symbol_id, config_version, effective_at_ms,
                    tick_size, lot_size, price_decimals, qty_decimals, status,
                    min_notional, max_order_qty,
                    maker_fee_bps, taker_fee_bps,
                    initial_margin_rate_bps, maintenance_margin_rate_bps,
                    max_leverage,
                    funding_interval_sec, funding_rate_min_bps,
                    funding_rate_max_bps,
                    created_at_ms
                )
                VALUES (
                    latest_var.symbol_id, latest_var.config_version + 1,
                    latest_var.effective_at_ms,
                    latest_var.tick_size, latest_var.lot_size, latest_var.price_decimals,
                    latest_var.qty_decimals, latest_var.status,
                    floor_var, latest_var.max_order_qty,
                    latest_var.maker_fee_bps, latest_var.taker_fee_bps,
                    latest_var.initial_margin_rate_bps,
                    latest_var.maintenance_margin_rate_bps,
                    latest_var.max_leverage,
                    latest_var.funding_interval_sec, latest_var.funding_rate_min_bps,
                    latest_var.funding_rate_max_bps,
                    (extract(epoch FROM clock_timestamp()) * 1000)::bigint
                )
                ON CONFLICT (symbol_id, config_version) DO NOTHING;
            END IF;
        END LOOP;

        INSERT INTO migrations (id) VALUES ('007_min_order_notional')
        ON CONFLICT (id) DO NOTHING;
    END IF;
END
$migration$;
