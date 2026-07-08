# 57 — Dedicated Config Server

Status: **spec** (not implemented). Phase-3 roadmap item — replaces
direct-from-Postgres config polling with a dedicated config service that
disseminates versioned config over casting, like everything else on the
wire. See README §Roadmap phase 3.

## Why

Today exactly one process type talks to Postgres for config: the matching
engine. `rsx-matching/src/main.rs:230` opens `RSX_ME_DATABASE_URL` at boot
and `rsx-matching/src/main.rs:807-843` polls `symbol_config_schedule` every
600s **inline in the hot loop**, via `rt.block_on(poll_scheduled_configs(...))`
on a `tokio::runtime::Builder::new_current_thread()` (`main.rs:231-235`)
that lives in the same thread as matching. A config poll is a synchronous
network round-trip to Postgres sitting between two iterations of the match
loop — the one place in ME where an external dependency can stall the hot
path, contradicting the tile model's "no runtime on the hot path" rule that
already holds for Risk (see CLAUDE.md Networking Stack; Risk keeps Postgres
off the busy-spin loop via a sidecar tokio thread + SPSC handoff — ME has
no such sidecar for config).

That's the latency problem. There's also a correctness/coherence problem:
config is fragmented across three uncoordinated sources, not one:

1. **Postgres** `symbol_config_schedule` / `symbol_config_applied` — tick
   size, lot size, price/qty decimals. Read and written only by ME
   (`rsx-matching/src/config.rs`).
2. **Env vars** `RSX_SYMBOL_{id}_TAKER_FEE_BPS`, `_MAKER_FEE_BPS`,
   `_INITIAL_MARGIN_RATE`, `_MAINTENANCE_MARGIN_RATE`, `_MAX_LEVERAGE` — read
   by Risk's `reload_symbol_overrides` (`rsx-risk/src/shard.rs:706-738`).
3. **Env vars** `RSX_SYMBOL_{i}_TICK_SIZE` / `_LOT_SIZE` — read once at
   Gateway boot by `load_symbol_configs` (`rsx-gateway/src/config.rs:43-66`),
   never updated after that.

`CONFIG_APPLIED` (`RECORD_CONFIG_APPLIED`, `rsx-messages/src/lib.rs:224-249`)
is the one thing that is WAL-sequenced and cast-fanned-out from ME today,
and Risk (`shard.rs:694-704`) / Gateway (`state.rs:77-85`) both apply it
monotonically (`version < current` dropped). But the record itself carries
only `(symbol_id, config_version, effective_at_ms, applied_at_ns)` — no
payload. Risk's handler *re-reads env vars* on receipt
(`reload_symbol_overrides`); Gateway's handler bumps a version counter and
nothing else. So the one coherence primitive that exists (a monotonic,
WAL-sequenced version bump) doesn't actually carry the values it's
supposedly versioning — fee/margin changes require an env var edit **and**
a process restart on Risk/Gateway, `CONFIG_APPLIED` alone does nothing for
them.

Net: PG is a config SPOF for ME (poll fails → stale tick/lot silently kept,
`main.rs:838` just logs a warning), a latency hazard on ME's hot path, and
not even the full story — two more config surfaces live in env vars with no
propagation path at all. A dedicated config server fixes the ordering
problem (one producer of `config_version` per symbol, pushed, not polled)
and the coherence problem (one payload, not three).

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
- Extending `SymbolConfig`'s Rust struct fields itself (currently
  `rsx-types/src/lib.rs:92-98`: `symbol_id, price_decimals, qty_decimals,
  tick_size, lot_size` only — no fee/margin fields). Wiring the fee/margin
  fields already named in 19-metadata.md's schema into `SymbolConfig` (or a
  superset type) is a prerequisite implementation task this spec assumes,
  not designs — see Success Criteria for the acceptance shape.

## Design

### Source of truth: unchanged, still Postgres

The config server does not introduce a new store. `symbol_config_schedule`
stays the write surface (operator or future dashboard inserts scheduled
rows, same shape as 19-metadata.md). The config server is the **only**
process left with a Postgres connection for config — ME, Risk, and Gateway
lose theirs entirely. This directly removes `RSX_ME_DATABASE_URL` from
`rsx-matching` and the `RSX_SYMBOL_{id}_*` config env vars from
`rsx-risk`/`rsx-gateway` (the env vars for *non-config* settings, e.g.
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
  today — `main.rs:878-891`). The config server pushing the payload doesn't
  bypass this; it just removes the Postgres round-trip that currently
  gates it.
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
  `config_version` only if `>= current` (`shard.rs:699-701`,
  `state.rs:82`) — but now the accepted record carries the actual field
  values (extending `ConfigAppliedRecord` or pairing it with the config
  server's own cast payload keyed by the same version), so acceptance also
  *applies* the values instead of triggering an env-var re-read.

### Startup and failure behavior

- **Config server down at ME/Risk/Gateway boot:** each process falls back
  to its last-known-applied config (ME already persists this per-symbol in
  `symbol_config_applied`-equivalent state carried in its own WAL/snapshot;
  Risk/Gateway get it from `CONFIG_APPLIED` replay same as today). No
  process blocks its own boot waiting on the config server — this is the
  same "fail-fast at startup, degrade to last-good at runtime" posture ME
  already uses for Postgres (`main.rs:269-276`).
- **Config server down at runtime:** existing config keeps running
  unchanged (no live update available, not "config unknown" — the last
  applied version is still valid). No hot-path effect either way, since the
  push is off the hot path entirely (unlike today's inline poll).
- **Config server restart:** rehydrates from Postgres (`symbol_config_schedule`
  + `symbol_config_applied`), same tables as today; nothing about the
  schema changes, only who reads it.

### What moves, what stays

| Concern | Today | After this spec |
|---|---|---|
| `symbol_config_schedule`/`symbol_config_applied` tables | Postgres, read/written by ME | Postgres, read/written by config server only |
| tick/lot/decimals propagation | ME polls PG every 600s inline | Config server pushes via cast; ME pulls on cold start |
| fee/margin/leverage | env vars, static per-process, no live update | part of the same versioned `SymbolConfig` payload from the config server |
| `CONFIG_APPLIED` WAL record | emitted by ME, version-only | emitted by ME, version-only (unchanged) — now paired with a full-payload config-server record for the value itself |
| Risk/Gateway config application | version bump + env re-read (fee/margin) or no-op (tick/lot) | version bump + apply pushed payload |

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
   epsilon, sub-second — not today's up-to-600s poll window) and the fee/
   margin values Risk uses for the next order **actually change** (not just
   the version counter).
5. Config server restart mid-session: in-flight trading is unaffected;
   after restart, a fresh cold-start query against it returns the same
   version every consumer already converged on (no regression to a stale
   version).

## Current state baseline

- Only ME talks to Postgres for config, via `RSX_ME_DATABASE_URL`
  (`rsx-matching/src/main.rs:230`). Risk and Gateway do not poll Postgres
  for config (`specs/2/19-metadata.md` already specifies this — "Risk and
  Gateway sync from matcher events (not direct DB polling)" — and the code
  matches that for the *version* signal, just not for the *values*).
- ME polls `symbol_config_schedule` every 600s, **inline in the hot loop**
  via a current-thread tokio runtime's `block_on`
  (`rsx-matching/src/main.rs:231-235, 807-843`,
  `rsx-matching/src/config.rs::poll_scheduled_configs`). On poll failure it
  logs a warning and keeps the last-applied config (`main.rs:838-840`) —
  no retry backoff, no fallback path other than "wait for next 600s tick".
- Applied tick/lot/decimals config is written back to Postgres
  (`symbol_config_applied`, one row per symbol, upsert-on-conflict,
  `rsx-matching/src/config.rs::write_applied_config`) so Risk/Gateway
  cold-starts could in principle bootstrap from it per 19-metadata.md —
  but nothing in `rsx-risk`/`rsx-gateway` today reads that table; they
  don't have a `DATABASE_URL` connection to config data at all except
  Risk's own `DATABASE_URL` (`rsx-risk/src/config.rs:229`), which is for
  position/margin persistence, not symbol config.
- `SymbolConfig` in `rsx-types/src/lib.rs:92-98` has exactly five fields:
  `symbol_id, price_decimals, qty_decimals, tick_size, lot_size`. None of
  the fee/margin/funding fields from `specs/2/19-metadata.md`'s
  `symbol_config_schedule` data model (`maker_fee_bps`, `taker_fee_bps`,
  `initial_margin_rate_bps`, `maintenance_margin_rate_bps`, `max_leverage`,
  `funding_interval_sec`, `funding_rate_min_bps`, `funding_rate_max_bps`)
  exist in the Rust type — those live only as env vars
  (`RSX_SYMBOL_{id}_TAKER_FEE_BPS` etc., read by
  `rsx-risk/src/shard.rs:706-738`) or as static Gateway boot-time env vars
  (`rsx-gateway/src/config.rs:43-66`, tick/lot only, no live update path).
- `RECORD_CONFIG_APPLIED` (`rsx-messages/src/lib.rs:224-249`) is
  WAL-sequenced by ME (`rsx-matching/src/main.rs:874-901`) and forwarded
  cast-side to Risk (`rsx-risk/src/main.rs:616-624`) and Gateway
  (`rsx-gateway/src/main.rs:425-435`), both of which apply it
  monotonically by version. The record carries `(symbol_id,
  config_version, effective_at_ms, applied_at_ns)` only — no config
  values. Risk's handler re-reads env vars on receipt
  (`rsx-risk/src/shard.rs:694-704, reload_symbol_overrides`); Gateway's
  handler (`rsx-gateway/src/state.rs:77-85`) only advances the version
  counter and does not touch `symbol_configs` at all.
