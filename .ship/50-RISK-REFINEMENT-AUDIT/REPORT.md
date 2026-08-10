# 50 — rsx-risk refinement audit

**Question:** is rsx-risk as minimal, simplified and refined as it can be
while staying correct, non-hanging, and fast?

**Verdict: not done — one focused refinement pass remains.** The domain core
(margin math, position math, funding zero-sum, freeze lifecycle, warm-standby
promotion) is careful, well-reasoned, and well-tested. What is not finished is
the **failure-path layer**: the live gap-replay discards state, the persist
sidecar can die silently under a Live shard, a warm standby cannot be
SIGTERMed, and the pinned loop log-storms and heap-allocates under exactly the
conditions where it must not. Plus a docs layer (README/ARCHITECTURE) that has
drifted from the code on several load-bearing claims. None of this is a
rewrite; all of it is amendments to mechanisms that already exist.

## Scope and method

- Audited at `98910cd5` (rsx-risk `src/` identical from `9689aa38` through
  `98910cd5`; concurrent commits in that window touched only
  `rsx-risk/tests/` and `rsx-risk/demo/`). All line numbers are at
  `98910cd5`. Shared-box caveat: HEAD moves concurrently; verify ancestry
  before relying on hashes.
- Static audit, read-only: all 21 `src/*.rs` files, `tests/` (24 files),
  `benches/` (5), `README.md`, `ARCHITECTURE.md`, `CLAUDE.md`, `notes/`,
  `migrations/`, plus the rsx-cast/gateway/matching call sites the hot loop
  depends on (`cast.rs`, `replication_client.rs`, `cast_recv.rs`,
  `rsx-matching/src/main.rs`). **I did not run tests or clippy** — two agents
  were editing the tree concurrently; a mid-edit tree gives false signal.
- Cross-referenced against BUGS.md (the 49-FAILURE-MODE-AUDIT and
  crash-guarantee entries landed mid-audit) and `.ship/51-RISK-MODULE-SPLIT/`.
  Findings below are marked **[NEW]** or **[already in BUGS.md as ...]** so
  nothing gets double-entered.

---

## 1. Correctness and hang-freedom (ranked by production bite)

### 1.1 [NEW — HIGH] Live FAULTED/RECONNECT replay discards ME records → permanent position divergence

`failover.rs:38-102` `handle_replay` drains the gap from the ME replication
server but its apply callback **only logs** (`failover.rs:72-80`,
`tracing::debug!`) — replayed records are never folded into the shard. The
live loop then `reset_after_replay(new_tip)` (`main.rs:663,675`) and resumes
past the gap.

Consequence chain, verified in code:

- Fills in the gap never reach `process_fill` → positions ≠ sum of fills
  (invariant #4). ORDER_DONE/CANCELLED/FAILED in the gap never release
  freezes → permanent in-memory margin over-reservation.
- The next live fill advances `tips[sid]` **past** the gap
  (`shard.rs:306`) and persists that tip, so a restart's
  replay-from-tip+1 never re-covers the gap either. The divergence is
  **durable**, not until-restart.
- The doc comment ("Matching independently restores authoritative state
  from risk's WAL", `failover.rs:33-34`) does not describe the code: risk
  has no WAL, and matching's recovery does not restore risk's positions.

The warm-catchup path already does this correctly — it applies every record
via `replay::apply_record` (`failover.rs:216,281`). The fix is an amendment,
not a mechanism: pass an `apply_record`-based closure (plus GW forwarding)
into the live `handle_replay` for the ME streams. Note BUGS.md already records
the *identical defect class* for marketdata (**MD-REPLAY-APPLY-IS-NOOP**) and
a seq-domain bug in the same reset path
(**RISK-CMP-SEQ-DOMAIN-CONFLATION**) — but the rsx-risk noop-apply itself is
not recorded. No test covers state application on the live-fault path
(`replay_drain_test.rs` tests drain mechanics only, with a counting callback).

For the GW order stream the same log-only replay is acceptable by design
(dropped pre-ack orders are client-resent and ME-deduped —
`rsx-matching/src/main.rs:703-732` documents this), and for mark it is
harmless (latest-wins). The severity is specifically the **ME fill stream**.

### 1.2 [already in BUGS.md as RISK-PERSIST-CIRCUIT-OPEN-SILENT-DEATH, RISK-PUSH-PERSIST-RING-FULL-DROPS-EVENT] Persist-worker death leaves a Live shard with no persistence and no liquidation

Confirmed independently; two additions to the recorded entries:

- **The stall is worse than "no persistence": `tick()` stops running.**
  When the worker dies, the ring fills, `push_persist` sets
  `backpressured` (`shard.rs:166-173`), and `tick()` early-returns forever
  (`shard.rs:881-892`) — so **liquidation sweeps, funding settlement, and
  BBO-driven margin scans halt permanently** while `process_order` keeps
  accepting orders (`main.rs:692-724` has no backpressure gate on ingest).
  The risk engine keeps trading with its risk controls off.
- **A poison batch makes this reachable with healthy PG and healthy
  lease.** `flush_batch` retries the *same* `pending` batch
  (`persist.rs:381-441`, pending never cleared on error), so one
  deterministic per-row error (e.g. constraint violation) → 8 consecutive
  failures → circuit opens → worker exits. The lease keeps renewing
  (separate connection, `pg_locks` SELECT), so no demote ever happens —
  unlike a full PG outage, where the lease path (3 errors ≈ 3 s,
  `lease.rs:167-175`) demotes first.
- The one gauge that would reveal it is dead: `persist_ring_used` is read
  by `metrics.rs:15` but **never stored anywhere** (only `persist_ring_cap`
  is, `main.rs:391`) — `/metrics` reports 0 occupancy forever.

Fix direction (amend, don't add): mirror the existing `lease_held` pattern —
a worker-death/lag flag the live loop checks next to `lease_held`
(`main.rs:965`) and treats as `return Err` (restart budget → standby
takeover), plus store ring occupancy each loop.

### 1.3 [NEW — MED-HIGH] Warm standby ignores SIGTERM/SIGINT — unkillable except SIGKILL

`run_warm_catchup` (`failover.rs:196-299`) never observes the shutdown flag.
`main.rs` checks it only before `run_main` re-entry (`main.rs:204`) and inside
the live loop (`main.rs:479`); `install_shutdown_handler`
(`rsx-types/src/macros.rs:23-37`) only sets the flag — there is no
second-signal force-exit. A standby (lock held elsewhere) loops in warm
catchup indefinitely and never exits on SIGTERM. Fix: pass `shutdown` into
`run_warm_catchup` and check it per iteration (one added condition).

Related hang surface, noted not fixable here: the replication client has **no
read timeout** (`rsx-cast/src/replication_client.rs` — bare
`read_exact(...).await`). A wedged-but-open server connection blocks
`run_once` forever: in warm catchup that wedges promotion; in live
`handle_replay` it wedges the pinned loop. rsx-cast is frozen — this is a
caller-policy question (timeout wrapper around `block_on`) to raise, not code
to write in rsx-cast.

### 1.4 [NEW — MED-HIGH] Log storms on the pinned thread under backpressure

Two `tracing::warn!` sites fire at busy-spin frequency in precisely the
degraded states above, doing synchronous, mutex-guarded stdout writes on the
pinned core:

- `shard.rs:169` — "persist ring full, stalling": once per **dropped
  event** (up to 6 per fill) while the ring is full.
- `shard.rs:900-906` — "downstream ring full, stalling": once per **main
  loop iteration** (`tick` is called every pass, `main.rs:838`) while an
  egress ring is full — millions of lines/sec on a spin loop.

The repo has `rsx-log` for exactly this. Minimum fix: log on state
*transition* only (the receiver's own `last_drop_warn` 5 s debounce,
`rsx-cast/src/cast.rs`, is the in-repo precedent).

### 1.5 [NEW — MED] `release_frozen_for_order` owner-mismatch removes without restoring — code contradicts its own comment

`shard.rs:595-603`: the entry is `remove`d from `frozen_orders`, then on
`owner != user_id` the comment says "restore primary" but nothing reinserts
it. Result: primary and `frozen_by_user` index permanently diverge — the true
owner's `frozen_for_user` keeps counting margin whose backing entry is gone,
and the owner's later release/confirm finds nothing. Reachability is low
(requires an order-lifecycle record whose `user_id` differs from the freeze
owner for the same 128-bit order id), but this is a latent
index-consistency bug and a fail-loud violation (silent return on an
impossible-state signal). Fix: reinsert on mismatch (make the comment true)
and log `error!`.

### 1.6 [NEW — MED, verify against spec] Socialized loss: full notional, repeatedly, and the position never terminates

`process_socialized_loss` (`shard.rs:834-870`) deducts `qty * price` — the
position's **full notional at mark** — where `specs/2/13-liquidator.md` §9
says "the **remaining loss** is socialized" (the shortfall). And nothing
closes or transfers the position afterwards: the `LiquidationState` goes
`Done` and is removed (`liquidation.rs:140-158`, `shard.rs:963`), but the
underwater position remains, so the next BBO margin scan re-enqueues the user
(`shard.rs:748-763` → `check_liquidation_for` → `enqueue`), rounds run again,
and the insurance fund is deducted **again** — roughly every
`Σ round·base_delay` (~5.5 s at defaults) for as long as the liquidation IOCs
can't fill. An illiquid symbol drains the fund without bound. I could not find
a test asserting post-socialization behavior. If the spec intends
close-out/ADL at bankruptcy price, that mechanism is missing; if not, the
deduction amount still looks wrong. Needs a spec decision before code.

### 1.7 [NEW — LOW-MED] Fail-late and fail-silent config

- `RSX_GW_REPLICATION_ADDR` / `RSX_MARK_REPLICATION_ADDR` are read with
  `.expect()` **at fault time inside the live loop** (`main.rs:762,775,809,822`)
  — a misconfigured process runs fine until its first gap, then panics.
  These belong in `RuntimeConfig::from_env` (config.rs:349), whose doc
  comment claims all topology is "parsed ONCE at boot, fail-fast".
- `env_u32/env_u64/env_usize` (`config.rs:65-84`) silently fall back to
  defaults on a *malformed* value (`parse().ok().unwrap_or(default)`) — a
  typo'd `RSX_RISK_SHARD_ID=abc` silently becomes shard 0. Inconsistent
  with the crate's own fail-fast policy one screen down, and a fail-loud
  violation. (The advisory lock fences the worst outcome, but the process
  should refuse to boot.)
- Related, already recorded: **RISK-SINGLE-REPLAY-ENDPOINT-PANICS-TRIO**
  covers the `handle_replay` panic-on-refused-replay (`failover.rs:82-87`).
  Note also that all mid-run panics bypass `main()`'s restart-backoff loop
  entirely — the panic handler is `process::exit(1)`
  (`rsx-types/src/macros.rs:6-11`), so the carefully built backoff ladder
  (`main.rs:224-245`) only ever sees `Err` returns. Failover still covers
  it (lock released on connection drop), but the restart budget is dead
  weight for the panic class.

### 1.8 [NEW — LOW] Egress seq discipline is inconsistent, and risk's retransmit path is structurally empty

`forward_to_gw` advances the seq even when `send_raw` fails
(`failover.rs:124-127`), burning a seq → downstream NAK → gap-skip; the ME
order path advances only on `Ok` (`main.rs:910-919`). One of the two is wrong;
pick the ME convention.

Bigger picture, worth one deliberate decision rather than code: risk's egress
uses `send_raw`, which populates **neither** the retransmit ring
(`rsx-cast/src/cast.rs:372-377` vs `send_framed` `:207-241`) **nor** any WAL
(nothing in rsx-risk writes one), and risk runs no `ReplicationService`. So
every NAK against risk's streams is unserveable ("not in ring or wal"), and
recovery is: ME → skip + client-resend (documented, fine); GW →
`RSX_RISK_REPLICATION_ADDR` unset → skip, "**in-flight fills lost**"
(`rsx-gateway/src/cast_recv.rs:94-104`). ME and Mark pair WAL +
ReplicationService; risk is the one producer that doesn't. Whether a
higher-level reconciliation makes lost fill *pushes* acceptable is a
system-level question (see "Could not determine").

### Hang-freedom, otherwise: clean

The specific hazards the shape invites are handled: the egress-ring-full case
correctly stalls ingest instead of dropping (loop-head gate `main.rs:689-711`,
R-N2) and stalls liquidation-order emission (`shard.rs:894-955`); the two
`expect`s on ring pushes are genuinely guarded by the loop-head check; no
locks exist on the tile (single-threaded, documented in ARCHITECTURE "Lock
Order: None" — verified, no Mutex/RwLock/DashMap in src); `stop_persist_worker`
has a bounded 5 s join (`main.rs:984-1012`); `stop_lease_thread` joins a loop
that observes its stop flag at 1 s cadence (`lease.rs:150-157`); demote tears
down both sidecar threads before re-entry (no thread/connection leak). I found
no deadlock, no livelock, and no unbounded growth on the hot structures
(liquidation `active` is bounded by (user,symbol) pairs; `stashed_bbo` is
fixed-size latest-wins).

---

## 2. Minimization and orthogonality

The big-file problem is already owned: `.ship/51-RISK-MODULE-SPLIT/PLAN.md`
(founder-directed, 2026-08-10) plans `main.rs` → app/promotion/cast_loop/
persist_loop and names the `run_main` monolith precisely. I endorse that plan
and add only what it does not cover:

**Dead code (delete):**
- `MarkPriceUpdate` (`rings.rs:5-11`) — zero usages anywhere.
- `RiskShard::is_backpressured` (`shard.rs:176-184`) — never called (only a
  bench comment mentions it).
- `AdvisoryLease::is_acquired` (`lease.rs:99-101`) — never called.
- `replay::acquire_advisory_lock` (`replay.rs:343-348`) — duplicates
  `AdvisoryLease::acquire`; used only by `persist_test.rs`. Second path for
  the same mechanism — fold into the original.
- `RiskShard::halt_liquidation` (`shard.rs:427-429`) — test-only passthrough
  (`halt_liquidation_for_user` calls the engine directly).

**Duplication (amend the original):**
- `rand_jitter` twice with different moduli (`main.rs:274-281`,
  `persist.rs:447-453`).
- `order_key` (`shard.rs:975-977`) re-derived inline in
  `load_from_postgres` (`replay.rs:223`).
- The three Faulted/Reconnect match arms in the live loop are ~120 lines of
  near-identical boilerplate (`main.rs:646-677, 752-786, 799-833`) — one
  helper over (label, replay_addr, stream_id, receiver).
- Two inline `OrderFailedRecord` construction sites (`main.rs:864-880,
  935-945`).
- `replay_freeze_order` (`shard.rs:620-655`) re-implements the margin-freeze
  arithmetic from `PortfolioMargin::check_order` (`margin.rs:101-125`) —
  extract the shared `order_im + fee` computation so they can't drift.

**Vestigial path:**
- `replay_from_wal` (`replay.rs:316-341`, called `main.rs:334`) is a
  **no-op in every real deployment**: it reads `wal_dir/<symbol_id>/`
  (`rsx-cast/src/wal.rs:372-375`), nothing in rsx-risk ever writes a WAL,
  and the playground gives risk `./tmp/wal` while MEs write
  `./tmp/wal/<name>/` (`rsx-playground/runtime.py:224,267`). Missing dirs
  yield an empty file list (`wal.rs:488-491`), so it silently replays 0.
  Warm catchup does the real work. Either delete it (and the "Replay the
  local WAL" boot-sequence step in README/ARCHITECTURE) or make it real.
  It is exercised only by tests that hand-write WALs into tempdirs.

**Two jobs in one place:**
- `reload_symbol_overrides` (`shard.rs:706-738`): runtime config-by-env-var
  polling living inside the state machine, forming a second config path
  beside `load_shard_config`'s hardcoded boot defaults. A restarted node
  has default fees/margins until a CONFIG_APPLIED record is replayed to it,
  and `config_versions` are not persisted — overrides older than the
  replication window are silently lost. Worth one design decision
  (config's one owner) during the split.

**Convention nits:** nine `// SAFETY:` comments on safe `.expect()` code
(`main.rs:151,262,267,305,347,419,446,453`, `config.rs:395`) — the crate has
zero `unsafe`; per repo Rust conventions `SAFETY` is reserved for unsafe
blocks (reason belongs in the expect message). Unit tests all live in
`tests/` instead of `src/<module>_test.rs` — already flagged as a gap by the
51 plan; they do run under `make test` (`--tests`), so no coverage hole.

`shard.rs` itself, minus the above, is legitimate domain complexity: the
fill/freeze/funding/liquidation orchestration is inherent, margin and
position math are cleanly delegated to `margin.rs`/`position.rs`/`funding.rs`,
and the secondary indexes (`positions_by_user`, `frozen_by_user`) are
maintained at the same mutation points as their primaries with a rebuild on
`set_state` — the right shape.

---

## 3. Speed

The order path itself is clean: no allocation in `process_order` /
`process_fill` (persist events are by-value into the ring; the
liquidation-check iteration is deliberately allocation-free,
`shard.rs:382-402`), FxHashMap throughout, fallback marks cached behind a
dirty flag rather than recomputed per order (`shard.rs:660-672`). Items:

- **[NEW] Two heap allocations per busy-spin iteration.**
  `LiquidationEngine::maybe_process` allocates two
  `Vec::with_capacity(4)` (`liquidation.rs:110-111`) and `tick` calls it
  every main-loop pass (`main.rs:838`) — millions of malloc/free per
  second on the pinned core, against the crate's zero-heap claim. Fix:
  reusable scratch buffers on the engine (or `Vec::new()` + early return
  when `active` is empty). Bench home: `benches/loop_arch_bench.rs`.
- **O(n) scans on the fill path**, fine at current scale, unbounded in a
  cascade: `ExposureIndex::add_user/remove_user` are linear
  `contains`/`retain` per fill (`margin.rs:150-160`);
  `LiquidationEngine::enqueue`/`cancel_if_recovered` scan `active` per
  fill (`liquidation.rs:84-98,192-195`). Would show up in a
  `risk_flood_bench` variant parameterized by exposed-user count — extend
  that bench before changing the structures.
- **ME streams drain at ≤1 record per stream per loop iteration**
  (`main.rs:493-494` — single `try_recv_with` per stream) while the GW
  loop drains to Empty (`main.rs:681-786`). Each fill in a burst pays a
  full loop iteration (including `tick` and the GW drain). This mirrors
  rsx-matching's 1-per-iteration shape, so it may be deliberate; whether
  it keeps up with ME's burst fill rate is a bench question
  (`risk_flood_bench`), not something to assert.
- **Persist drain rate bounds burst tolerance**: row-at-a-time
  `execute` per event (`persist.rs:79-289`), no COPY, no prepared-statement
  reuse; ~6 events/fill against an 8192 ring ≈ 1.3k-fill headroom between
  10 ms flushes. Off the hot path, but it is the input pressure behind
  finding 1.2's ring-full states. (ARCHITECTURE claims "Fills: COPY
  binary" — see docs drift.)
- Micro-nit: reject records stamp `ts_ns = now_secs * 1e9`
  (`main.rs:866,937`) — second-granularity timestamps on a ns field;
  `time_ns()` is equally cheap.

---

## 4. Docs drift (README / ARCHITECTURE vs code)

Load-bearing mismatches — worth a single doc-sync commit; the crate CLAUDE.md
keeper sections (how-to-read index, pre-trade-gate framing, hot/off-path
split, notes rationale) are all intact:

- README:83 "migrations under `MIGRATION_LOCK_KEY`" — the lock was removed;
  `schema.rs:9-15` takes none and `migrations/CLAUDE.md` documents why.
  README also says 002 "relies on the lock"; 002 is guarded/idempotent.
- README:95-97 and ARCHITECTURE:73,345 "7 SPSC rings ... fill, order, mark,
  bbo (consumers)" — input rings were removed (`rings.rs:31-34`); there are
  3. ARCHITECTURE's own table (:78-83) says 3.
- README:95 "`Shard::run_once`" — no such method.
- README Gotchas: "Frozen margin is in-memory only" — stale; durable
  freezes exist (`004_frozen_orders.sql`, `confirm_freeze`,
  `shard.rs:566-588`). "Replica buffers fills in memory ... no cap" — stale;
  the warm replica applies records directly, nothing buffers.
- README env table: `RSX_RISK_REPLICA_ADDR` has zero usages in the
  codebase; meanwhile the actually-required replay/TLS/lease/liquidation
  vars are absent. "20 test files" — there are 24.
- ARCHITECTURE:309-313: "Fills: COPY binary" (row INSERTs);
  "PG positions: version CAS on UPSERT" (upserts are unconditional,
  `persist.rs:84-111` — and BUGS.md's RISK-TIPS-UPSERT-NO-MONOTONIC-GUARD
  now depends on that fact); "Backpressure: PG lag > 100ms stalls the hot
  path" (not implemented — also noted in the BUGS entry);
  `account.frozen_margin` (:135) and `MainTransition::Demote` (:281) —
  neither exists.
- `failover.rs:33-34` doc comment on `handle_replay` (see finding 1.1).

---

## 5. What is genuinely good (don't regress)

- The **freeze durability protocol** — durable freeze written only on ME's
  ORDER_ACCEPTED, precisely to make a phantom PG freeze impossible
  (`shard.rs:533-542`) — with the orphan-freeze test backing it.
- The **warm-standby promotion protocol**: single-fence advisory lock,
  catch-up only gating *when* to try it, final-drain before Live, and a
  re-enterable `run_main` that tears down everything it spawned. The
  no-double-main argument in ARCHITECTURE matches the code.
- **SEQ-1 discipline** (per-consumer re-sequencing) applied and explained
  at every fan-out point.
- **Funding residual handling** (`funding.rs:45-101`): exactly right for a
  sharded venue — kills intra-shard rounding leak without forcing a
  one-sided column to zero.
- Overflow discipline: i128 intermediates + saturate + `error!` throughout
  the money math; `calculate_fee` saturates toward the correct sign.
- Egress backpressure that prevents ghost orders (loop-head gate + push
  `expect`s that are actually guarded).
- Test breadth: 24 files including failover e2e, insurance/liquidation e2e,
  persist resilience (which documents the circuit-open behavior), and the
  new crash-guarantee suite.

## Could not determine

- Whether lost risk→GW fill *pushes* (finding 1.8) are covered by a
  higher-level client reconciliation (specs 29-rpc/49-webproto) — needs a
  gateway/spec read this audit did not do.
- Intended post-socialization semantics (finding 1.6) — spec §9 is silent
  on position close-out; needs the founder's call.
- Whether the ≤1-record-per-iteration ME drain keeps up at production fill
  rates — bench question, not answerable statically.
- Test/clippy status of the tree — deliberately not run (concurrent edits).
- Whether `RSX_RISK_REPLICATION_ADDR` (referenced by rsx-gateway) is a
  planned future risk WAL+ReplicationService or a dead reference.

## Recommended pass (in order)

1. Fix 1.1 (apply records in live ME replay) — small, highest stakes.
2. Fix 1.2's missing half (worker-death flag + persist_ring_used gauge);
   the BUGS.md entry already has sign-off-ready shape.
3. Fix 1.3 (shutdown flag in warm catchup) and 1.4 (debounce the two
   warns) — both tiny.
4. Decide 1.6 with the spec; fix 1.5 and 1.7 mechanically.
5. Run the 51-RISK-MODULE-SPLIT with the §2 deletions/dedups folded in.
6. One docs-sync commit for §4.

Items 1-4 are roughly a day; the split is already planned separately.
