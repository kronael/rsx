# migrations/

Risk-shard Postgres schema. Plain `.sql` files, applied in order at boot by
`run_migrations` (`../src/schema.rs`, wired via `include_str!`). No advisory
lock, no migration framework — the files are written to be safe on their own.

## The rule: every migration is idempotent and concurrency-safe

Each file must satisfy all three, so any node can run all migrations at boot,
concurrently, repeatedly, with no coordination:

1. **Version-guarded.** Wrap the body in
   `IF NOT EXISTS (SELECT 1 FROM migrations WHERE id = 'NNN_name') THEN … END IF;`
   and end with `INSERT INTO migrations (id) VALUES ('NNN_name') ON CONFLICT (id) DO NOTHING;`.
   The guard skips already-applied migrations (and stops a later rename from
   being undone by an earlier `CREATE` re-running). The `ON CONFLICT` makes the
   record itself race-safe.
2. **Idempotent DDL.** `CREATE TABLE IF NOT EXISTS`, `CREATE INDEX IF NOT EXISTS`,
   `ALTER TABLE … DROP COLUMN IF EXISTS`. Renames are not naturally idempotent —
   guard them with existence checks (`IF EXISTS(old) AND NOT EXISTS(new) THEN
   ALTER … RENAME …`). See `002_rename_tables.sql`.
3. **Atomic.** Each file is one `DO $migration$ … $migration$;` block — a single
   statement, so Postgres runs it in its own transaction. Either the whole
   migration commits or none of it does.

**Why this is enough (no lock).** Two nodes booting at once both pass the
version guard before either commits. With idempotent DDL the actual statements
serialize on Postgres's table locks and no-op on the loser; the `INSERT … ON
CONFLICT` records the version exactly once. So the old
`pg_advisory_lock(MIGRATION_LOCK_KEY)` wrapper was removed — it was papering
over non-idempotent DDL, not a real requirement. Migrations on an exchange must
be trivially safe; a lock that "usually works" is not that.

## Adding a migration

1. New file `NNN_name.sql` (next number), following the three rules above.
2. Add `const MIGRATION_NNN` + a `batch_execute` line in `../src/schema.rs`.
3. Verify with the testcontainers integration tests (`make integration`), which
   run migrations against a real Postgres from a clean slate AND twice (re-run
   must be a no-op).

## The symbol catalog lives here too

`005_symbol_config.sql`, `006_seed_symbols.sql` and
`007_min_order_notional.sql` carry the symbol catalog
(`symbol_static`) and the versioned, effective-dated risk config
(`symbol_config_schedule`): tick and lot size for the matching engine, margin
rates, leverage cap and fees for risk, funding bounds. Risk applies them and
reads the active row per symbol at boot (`../src/config.rs`,
`load_symbol_configs`); the matching engine and the playground Config tab
read the same tables. A new version is a new row — the active row is never
updated — so the config in force at any instant is reconstructible. The one
writer is `config::schedule_symbol_config` (`rsx-cli config schedule`): it
copies the symbol's highest version, changes the named columns and commits
only a row risk reads back as valid; it needs no migration.

`min_notional` is the smallest order, in the unit of `price * qty`:
`10^-(price_decimals + qty_decimals)` quote, so 1 USD is `10_000_000_000`
on every preset. `006` seeds a placeholder; `007` adds, per symbol, a copy
of the highest-version row with `min_notional` raised to 1 USD. It only
raises: a row already at or above 1 USD gets no new version. Risk refuses
any row whose `price_decimals + qty_decimals` is not
`config::NOTIONAL_DECIMALS` (10), because collateral and margin sum
`price * qty` across symbols in one unit.

## Not handled here: the master data server

These migrations serve one Postgres per deployment. There is no separate
master-data service that shards subscribe to for the user identity registry,
listing/delisting or cross-deployment catalog sync; that is a future addition,
out of scope for these migrations.
