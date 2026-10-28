# CLAUDE.md — rsx-risk

Local to `rsx-risk/`; the root `../CLAUDE.md` applies. Risk is the pre-trade
gate, one shard per `user_id` range: margin, positions, funding, liquidation,
and the exchange's Postgres. Specs: `specs/2/28-risk.md`, `13-liquidator.md`,
`6-consistency.md`, `19-metadata.md`; tests `42-testing-risk.md`,
`38-testing-liquidator.md`.

## Rules

- Risk decides BEFORE the match and only processes AFTER it. An order risk
  accepted at pre-flight gets its fill applied unconditionally; there is no
  "rejecting a fill". NEVER add a post-fill reject path.
- One pinned busy-spin loop (`cast_loop.rs`) over the std non-blocking
  `CastReceiver`, shaped like matching: no async runtime, no sleep, no poll,
  one copy off the network buffer per record. Postgres write-behind and lease
  renewal run on the persist side-thread (`persist.rs`, tokio) fed over the
  rtrb persist ring. It is a thread of this process; NEVER describe or build
  it as a sidecar process.
- Side-threads (persist, health, config reload) are spawned BEFORE the hot
  thread pins, so none inherits `RSX_RISK_CORE_ID`.
- Risk owns Postgres: the schema, every migration (`migrations/`, rules in
  `migrations/CLAUDE.md`), the symbol catalog and `symbol_config_schedule`.
  Matching and the playground are readers. NEVER a migration or a seed
  outside `migrations/`; a new config version is a new row and the active
  row is never updated.
- Every per-symbol parameter (margin rates, leverage cap, fees, funding
  bounds, tick and lot) comes from the active `symbol_config_schedule` row
  through one apply path (`config.rs`) shared by boot, promotion and live
  reload. An unconfigured symbol rejects with `SymbolNotConfigured`. NEVER an
  env override or a hardcoded default for a symbol parameter. Env is wiring
  only (`RSX_RISK_SHARD_ID`, `RSX_RISK_CORE_ID`, cast and replication
  addresses, WAL dir); a malformed value fails the boot.
- Live reload stays off the pinned core: a `CONFIG_APPLIED` newer than the
  applied version → `PersistEvent::ConfigReload` → the persist thread reads
  that exact version → the config ring → `tick` drains it first.
- Risk recovers only from the ME replication stream and never reads a WAL
  back. Its egress streams, the gateway's and one per ME, are WAL-backed
  (`EgressStream` in `rings.rs`: `prepare`, `append_framed`, `send_framed`,
  flushed on the loop's 10 ms tick) for NAK retransmit, a new main's
  continued numbering and the gateway's replay, served by a
  `ReplicationService` side-thread spawned before the hot thread pins. A warm
  standby catches up over replication, then takes `pg_try_advisory_lock`, and
  is promoted only once caught up; the advisory lock is the sole single-main
  fence (invariant 10). Two warm replicas per shard is the deployment shape.
- Funding settles each interval exactly once: the last settled interval is
  persisted and missed intervals are caught up once at boot. Socialized loss
  re-triggers until the position is covered, throttled per (user, symbol).
- The liquidator is part of this process: limit orders that chase the market
  with increasing slippage per backoff until the account is no longer
  underwater. It never runs as a separate daemon.
- The DoS answer to resting orders is a minimum order size (about 1 USD
  notional, or a per-token minimum in the symbol config), enforced here from
  config; there is no resting cap at the gateway.
- Layout: `app.rs` boots, `cast_loop.rs` is the hot loop, `promotion.rs` the
  warm-standby protocol, `replay.rs` the ME replication consumer,
  `persist.rs` the side-thread, `rings.rs` egress; domain modules `margin`,
  `position`, `account`, `funding`, `liquidation`, `insurance`, `lease`,
  `db`, `schema`, `config`. A five-line struct does not earn a module; a loop
  always does.
- Tests: `tests/` drive a real shard over sockets; Postgres paths run under
  `make integration` (testcontainers, migrations applied twice to prove
  idempotence); crash, handoff, promotion and replay are integration tests,
  not unit tests. Benches live in `benches/`.

## Docs (which file answers which question)

| File | Question |
|---|---|
| `README.md` | what the tile does, `Running` (env), Postgres + migrations setup, `How to read this crate` |
| `ARCHITECTURE.md` | how it is built: the pre-trade gate, hot loop, margin math, fill/funding/liquidation, persist ring → tokio thread, warm-standby promotion |
| `notes/spsc.md`, `notes/uds.md` | why rtrb rings over channels/mutex/shmem; why not Unix sockets or shared memory |
| `migrations/CLAUDE.md` | the idempotent-migration rule and the symbol catalog |

Dated numbers live in root `reports/`.

## Keeper sections — do not regress

- The `How to read this crate` index and the Postgres/migrations section in
  `README.md`.
- The pre-trade-gate framing in `ARCHITECTURE.md`: risk keeps one
  over-leveraged trader from making the exchange insolvent, not "it checks
  margin".
- The hot-path / off-path split stated explicitly: pinned std busy-spin loop,
  tokio side-thread, rtrb handoff. Never "it's async".
- `notes/spsc.md` and `notes/uds.md`: dropping them re-opens a settled
  decision. A new IPC decision → a `notes/` file (Problem → Fix → Cost) + a
  row in `notes/README.md`; a new number → `reports/YYYYMMDD_*.md`, quoted
  with bench name and caveat.
