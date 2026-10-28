# 57 — Dedicated Config Server

Status: **spec** (not implemented). Phase-3 roadmap item — replaces
direct-from-Postgres config polling with a dedicated config service that
disseminates versioned config over casting, like everything else on the
wire. See README §Roadmap phase 3.

## Why

Three process types read symbol config from the Postgres table
`symbol_config_schedule`: the matching engine, Risk and the Gateway. None of
them reads it on a hot path. The engine polls on a sidecar thread, Risk
fetches on its persist worker, and the Gateway fetches on a symbol sidecar.
Each hands the result to its loop over a ring.

What remains is coherence, not latency:

1. **Three readers, one table.** Each process holds its own Postgres client
   and its own query. The engine learns of a scheduled version at the next
   poll (`RSX_ME_CONFIG_POLL_S`, default 10 s). Risk and the Gateway learn of
   it from the engine's `CONFIG_APPLIED` record, and the Gateway also polls
   every `RSX_GW_SYMBOL_POLL_S` (default 10 s).
2. **`CONFIG_APPLIED` names a version and carries no value.**
   `ConfigAppliedRecord` in `rsx-messages/src/lib.rs` holds `symbol_id`,
   `config_version`, `effective_at_ms` and `applied_at_ns`. Risk and the
   Gateway accept it only above their applied version, then re-read the row
   from Postgres.
3. **Postgres is a config SPOF for a running engine.** A failed poll logs a
   warning and keeps the last-applied config until the next poll
   (`hand_off_due_configs` in `rsx-matching/src/config.rs`). The engine
   keeps trading on a stale tick and lot with no signal beyond the log.

A dedicated config server gives one producer of `config_version` per
symbol, pushed instead of polled, with the full payload on the record.

## Scope

**In scope:**
- A `rsx-config` service: one process, owns `symbol_static` +
  `symbol_config_schedule` + `symbol_config_applied` (still Postgres-backed
  — this is not a "replace Postgres" spec, it's "stop making ME talk to
  Postgres directly").
- A push channel (casting, same transport as everything else) carrying the
  **full** `SymbolConfig` payload — tick/lot/decimals **and** the fee/margin
  fields already named in `specs/2/19-metadata.md`'s data model
  (`maker_fee_bps`, `taker_fee_bps`, `initial_margin_rate_bps`,
  `maintenance_margin_rate_bps`, `max_leverage`, `funding_*`) — under a
  `config_version` that ME, Risk, and Gateway all key off.
- ME keeps `CONFIG_APPLIED` as the record that WAL-sequences *its own*
  application of a version (unchanged wire shape); the config server is
  upstream of that, not a replacement for it.
- Cold-start: config server serves "current config as of now" on connect
  (replaces each process's ad hoc Postgres/env bootstrap).

**Out of scope:**
- Secrets (JWT secret, DB credentials) — those stay env-var, this spec is
  market/instrument config only.
- Deploy/rollout mechanics for the config server binary itself (ops
  concern, not this spec).
- Removing Postgres — `symbol_config_schedule` stays the operator-facing
  edit surface (a dashboard or SQL writes future rows there); the config
  server is a cache + disseminator in front of it, not a replacement store.
- Extending `SymbolConfig`'s Rust struct fields itself (`rsx_types::SymbolConfig`
  holds `symbol_id, price_decimals, qty_decimals, tick_size, lot_size` only;
  Risk reads the fee and margin columns into its own symbol config in
  `rsx-risk/src/config.rs`). Carrying the fee, margin and funding fields of
  19-metadata.md's schema in one shared type is a prerequisite implementation
  task this spec assumes, not designs — see Success Criteria for the
  acceptance shape.

## Design

### Source of truth: unchanged, still Postgres

The config server does not introduce a new store. `symbol_config_schedule`
stays the write surface (operator or future dashboard inserts scheduled
rows, same shape as 19-metadata.md). The config server is the **only**
process left with a Postgres connection for config — ME, Risk, and Gateway
lose theirs for config. This removes `RSX_ME_DATABASE_URL` from
`rsx-matching` and the symbol-config queries from `rsx-risk` and
`rsx-gateway` (the env vars for *non-config* settings, e.g.
`RSX_ME_WAL_DIR`, are untouched).

### Dissemination: casting, versioned, push not poll

The config server casts a `ConfigUpdate` record per symbol on every applied
change (analogous to how ME casts fills/BBO today) — full `SymbolConfig`
payload plus `config_version` and `effective_at_ms`. It also serves
pull-on-start (a small TCP request/response, same shape as replication's
catch-up, not a new protocol) so a cold-started ME/Risk/Gateway gets
"current version for symbol S" without waiting for the next change event.

This mirrors the two-tier pattern replication already uses (hot
live stream + cold catch-up, `specs/2/10-replication.md`) instead of
inventing a third mechanism.

### Applying a version deterministically (the ordering invariant)

The critical invariant carried over from 19-metadata.md and CLAUDE.md's
correctness list (config must apply deterministically across replicas) is:
**a `config_version` bump is a WAL-sequenced event, not a side effect of
config arriving.** Concretely:

- ME still owns turning "config server says v6 is live" into "v6 is
  *applied*": it still writes `RECORD_CONFIG_APPLIED` to its own WAL at the
  point in its own event sequence where v6 takes effect (unchanged from
  today). The config server pushing the payload doesn't bypass this; it
  just replaces the sidecar's Postgres poll that currently feeds it.
- The ordering guarantee this buys: two ME instances (e.g. a symbol
  migrating between shards, or a future ME hot-standby) that both receive
  "config v6 payload" from the config server still apply it at whatever
  WAL seq their own event stream reaches `effective_at_ms <= now`, and both
  emit `CONFIG_APPLIED(v6, ...)` at that seq. Replay is deterministic
  because the WAL record is what's replayed, not the config-server push
  (the push is not persisted state; on replay ME never re-contacts the
  config server — `RECORD_CONFIG_APPLIED` already has everything needed to
  reconstruct the applied `SymbolConfig` from its own WAL).
- Risk and Gateway keep exactly their current rule — accept a
  `config_version` only above the applied one (`process_config_applied` in
  `rsx-risk/src/config.rs`, `is_config_newer` in `rsx-gateway/src/state.rs`)
  — but now the accepted record carries the actual field
  values (extending `ConfigAppliedRecord` or pairing it with the config
  server's own cast payload keyed by the same version), so acceptance also
  *applies* the values instead of triggering a Postgres re-read.

### Startup and failure behavior

- **Config server down at ME/Risk/Gateway boot:** each process falls back
  to its last-known-applied config (ME already persists this per-symbol in
  `symbol_config_applied`-equivalent state carried in its own WAL/snapshot;
  Risk/Gateway get it from `CONFIG_APPLIED` replay same as today). No
  process blocks its own boot waiting on the config server — this is the
  same "fail-fast at startup, degrade to last-good at runtime" posture ME
  already uses for Postgres (boot in `rsx-matching/src/app.rs`).
- **Config server down at runtime:** existing config keeps running
  unchanged (no live update available, not "config unknown" — the last
  applied version is still valid). No hot-path effect either way, since the
  push is off the hot path entirely (as the sidecar poll is today).
- **Config server restart:** rehydrates from Postgres (`symbol_config_schedule`
  + `symbol_config_applied`), same tables as today; nothing about the
  schema changes, only who reads it.

### What moves, what stays

| Concern | Today | After this spec |
|---|---|---|
| `symbol_config_schedule`/`symbol_config_applied` tables | Postgres, read/written by ME | Postgres, read/written by config server only |
| tick/lot/decimals propagation | ME polls PG every `RSX_ME_CONFIG_POLL_S` s (default 10) on a sidecar thread | Config server pushes via cast; ME pulls on cold start |
| fee/margin/leverage | Postgres columns, fetched by Risk on `CONFIG_APPLIED` | part of the same versioned `SymbolConfig` payload from the config server |
| `CONFIG_APPLIED` WAL record | emitted by ME, version-only | emitted by ME, version-only (unchanged) — now paired with a full-payload config-server record for the value itself |
| Risk/Gateway config application | version bump + Postgres re-read | version bump + apply pushed payload |

## Success criteria

1. `rsx-matching` boots and runs with no `RSX_ME_DATABASE_URL` set and no
   Postgres reachable from ME at all; config arrives solely from the config
   server's cast stream + cold-start pull.
2. Two ME instances (simulated shard handoff or a hot-standby test) fed the
   same config-server push both emit `CONFIG_APPLIED(v, seq)` such that
   replaying either WAL from before the change reconstructs the identical
   applied `SymbolConfig` — i.e. determinism is provable from the WAL alone,
   never from re-contacting the config server.
3. Killing the config server process during steady-state trading causes zero
   change in GW→ME→GW latency (nothing on the hot path depends on it) and
   zero new order rejections (last-applied config keeps validating orders).
4. A scheduled config change (new row in `symbol_config_schedule`,
   `effective_at_ms` in the past) reaches ME, Risk, and Gateway and is
   applied within a bounded window (target: within one cast round-trip +
   epsilon, sub-second — not today's poll window of up to
   `RSX_ME_CONFIG_POLL_S` seconds) and the fee/
   margin values Risk uses for the next order **actually change** (not just
   the version counter).
5. Config server restart mid-session: in-flight trading is unaffected;
   after restart, a fresh cold-start query against it returns the same
   version every consumer already converged on (no regression to a stale
   version).

## Current state baseline

- Three processes read `symbol_config_schedule`; none reads it in a loop
  that matches, checks margin or routes orders.
- **Matching engine.** `rsx-matching/src/app.rs` opens `RSX_ME_DATABASE_URL`
  at boot; its applied config is the book config, and an unreachable
  database stops boot. Without a url, polling is disabled. The `me-config`
  sidecar thread owns the client and a current-thread tokio runtime
  (`run_config_sidecar` in `rsx-matching/src/config.rs`). Every
  `RSX_ME_CONFIG_POLL_S` seconds (a whole number from 1; default 10; a bad
  value fails boot) it selects versions above the applied one with
  `effective_at_ms <= now` and pushes them on an rtrb ring of 64 slots. The
  loop applies them with `apply_due_configs`, which only pops the ring,
  emits `CONFIG_APPLIED` and pushes the applied row back. The sidecar upserts
  it into `symbol_config_applied`. A failed poll logs a warning and keeps
  the last-applied config until the next poll.
- **Risk.** `DATABASE_URL` is required. At boot `load_symbol_configs`
  installs each symbol's active row (the highest version with
  `effective_at_ms <= now`), fee, margin and leverage columns included. On
  `CONFIG_APPLIED` above its applied version, the loop queues a
  `ConfigReload` for the persist worker, which fetches that version. The
  loop never touches Postgres. Risk forwards the record to the Gateway.
- **Gateway.** `rsx-gateway/src/symbols.rs` loads the symbol table at boot.
  A symbol sidecar reloads it every `RSX_GW_SYMBOL_POLL_S` seconds (default
  10) and on a `CONFIG_APPLIED` newer than the table.
- `rsx_types::SymbolConfig` holds five fields: `symbol_id`, `price_decimals`,
  `qty_decimals`, `tick_size`, `lot_size`. The fee, margin and funding
  columns of `specs/2/19-metadata.md` reach Risk through its own symbol
  config, not through this type.
- `ConfigAppliedRecord` carries `(symbol_id, config_version,
  effective_at_ms, applied_at_ns)` and no config values.
