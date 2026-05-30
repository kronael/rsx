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

## Not handled here: the master data server

These migrations cover one risk shard's *operational* state (positions,
accounts, fills, frozen orders, funding, liquidations) in its own Postgres.
They do **not** define a **master data server** — a central source of truth for
*reference/master data* shared across all shards and processes: the symbol /
instrument catalog (tick size, lot size, margin params), the canonical user
identity registry, fee schedules, listing/delisting. Today that data is passed
in by config/env and seeded ad hoc; there is no central master-data service
that shards subscribe to. That is a future addition, out of scope for these
per-shard migrations.
