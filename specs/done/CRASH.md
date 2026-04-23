---
status: shipped
---

# RSX Crash Analysis (Value Flow + Failure Critique)

Audit date: 2026-02-11
Scope: current codepaths in `rsx-gateway`, `rsx-risk`, `rsx-matching`, `rsx-dxs`, `rsx-marketdata`.
Intent: analyze crash safety of value flow, identify loss windows, and separate immediate/simple fixes from deeper design work.

This document is an updated analysis for the evolved implementation. It complements `CRASH-SCENARIOS.md` (broad incident catalog) and focuses specifically on end-to-end value conservation and crash consistency.

---

## 0) Critique Of Prior Crash Writeup

1. It mixed confirmed behavior and design inference without clear labeling.
2. It listed immediate fixes but did not separate `done now` vs `open`.
3. It treated persistence-loss scenarios as static even after code changes.
4. It lacked a strict priority order tied to blast radius and fix effort.

### Addressed in this revision

1. Confirmed-from-code statements are explicit and include file references.
2. Immediate remediations are status-tagged in section 5.
3. Two crash-hardening fixes were applied now:
- replay release on `RECORD_ORDER_FAILED`
- persist worker retries failed batches instead of dropping

---

## 1) Value Flow (Current)

### 1.1 Primary value pipeline

1. Client submits order to Gateway WS (`rsx-gateway/src/handler.rs`).
2. Gateway emits `RECORD_ORDER_REQUEST` to Risk CMP.
3. Risk validates and reserves margin in-memory (`process_order`) and emits:
- `OrderFreezeUpsert`
- updated `Account`
4. Accepted order is forwarded to Matching (`rsx-risk/src/main.rs` -> `OrderMessage`).
5. Matching emits lifecycle records and fills to WAL/CMP.
6. Risk ingests ME records:
- `FILL`: updates positions/accounts/tips, persists snapshots
- `ORDER_DONE` / `ORDER_CANCELLED` / `ORDER_FAILED`: releases frozen margin, persists freeze delete + account
- `CONFIG_APPLIED`: updates in-memory config version + env-derived overrides
7. Gateway forwards user-facing events from Risk.

### 1.2 Durability boundaries

- Durable source of truth for trade execution is ME WAL (`rsx-dxs/src/wal.rs`).
- Risk Postgres persistence is async/batched every 10ms (`rsx-risk/src/persist.rs`).
- Risk cold start loads Postgres state then replays ME WAL (`rsx-risk/src/replay.rs`).
- Frozen order ledger exists in Postgres (`order_freezes`) and is loaded on cold start.

### 1.3 Shard ownership

- Shard routing rule is `user_id % shard_count == shard_id` (`rsx-risk/src/shard.rs`).
- Cold-start SQL uses same modulo rule (`rsx-risk/src/replay.rs`).

Inference: value consistency is only as strong as the handshake between Risk in-memory reservation state and ME lifecycle replay coverage.

---

## 2) Crash Matrix (Value-Centric)

| ID | Failure | Immediate Value Risk | Recovery Source | Residual Risk |
|---|---|---|---|---|
| C1 | Gateway crash | Client-facing ack/state loss only | Reconnect + Risk/ME still running | User confusion / duplicate submit pressure |
| C2 | Risk crash after reserve, before send-to-ME | Frozen margin can become stranded | Partial: Postgres `order_freezes`; no ME terminal event | **High**: no automatic release path if order never reached ME |
| C3 | Risk crash during fill processing | Position/account snapshots lag | ME WAL replay | Replay coverage gaps for non-fill lifecycle |
| C4 | Persist flush error (DB transient) | Persistence delay during retries | Retry same in-memory batch | **Medium/High**: unbounded delay if DB remains down |
| C5 | Risk restart after `ORDER_FAILED` path | Freeze release might be missed in replay | Replay now handles failed/done/cancel | **Low/Medium**: only if WAL record missing/corrupt |
| C6 | Main+replica risk churn | Slower catch-up / rebootstrap | Postgres + WAL replay | Promotion path not truly hot-state preserving |
| C7 | Postgres outage with sustained load | Persist ring backpressure | Stall by design | Availability degradation, potential order rejects upstream |
| C8 | ME crash with intact WAL | Short order disruption | Replay + restart | In-flight non-WAL order state lost |
| C9 | ME WAL corruption/loss | Fill/history durability compromised | None without backup/archive | Catastrophic manual recovery |
| C10 | Config update near crash | Runtime config drift | Eventual next CONFIG_APPLIED | Non-durable config version state |
| C11 | Split-brain Risk main (lock failure mode) | Double processing attempts | Manual intervention | Duplicate DB writes, undefined client outcomes |
| C12 | Marketdata crash | No balance/value impact | Replay/snapshot rebuild | Visibility-only degradation |

---

## 3) Detailed Scenarios

## C1: Gateway Process Crash

### Trigger
- Gateway panic/kill/restart.

### Effect
- WS sessions drop.
- Pending map is in-memory; correlation for in-flight client IDs is lost.

### Value impact
- No direct balance or position loss.
- May cause client resubmits and duplicate intent pressure.

### Recovery
- Reconnect clients.
- Rely on server-assigned order ids and terminal events from Risk/ME.

### Critique
- Correctly value-neutral, but UX-level ambiguity can amplify load during incidents.

---

## C2: Risk Crash After Margin Reserve But Before Forwarding to ME

### Trigger
- `process_order` accepts and freezes margin.
- Crash occurs before accepted order is reliably sent to ME.

### Current behavior evidence
- Freeze happens in `rsx-risk/src/shard.rs` (`process_order`).
- Forwarding to ME happens later in loop via `accepted_cons.pop()` and `send_raw` in `rsx-risk/src/main.rs`.
- No transactional coupling between reserve and send-to-ME.

### Value impact
- Frozen margin can remain reserved for an order that never existed in ME.

### Recovery path today
- Cold start restores `order_freezes` from Postgres.
- Replay releases only when terminal lifecycle record exists.
- If order never reached ME, no lifecycle record exists to release.

### Severity
- **P0 correctness risk** (stranded collateral / over-reservation).

### Immediate fix options
1. Emit a durable `ORDER_ACCEPTED` record from Risk and replay orphan accepts with timeout-based compensating release.
2. Add local outbox semantics: reserve + send-intent persisted atomically; replay outbox on restart.
3. If `send_raw` fails synchronously, immediately reverse freeze and emit reject.

---

## C3: Risk Crash During Fill Processing

### Trigger
- Crash after applying some fills in memory, before Postgres flush.

### Current behavior evidence
- Fills update positions/accounts/tips in memory (`rsx-risk/src/shard.rs::process_fill`).
- Persistence is async batch every 10ms (`run_persist_worker`).

### Value impact
- Postgres can lag, but ME WAL replay can restore fill-derived state.

### Recovery
- `load_from_postgres` baseline + `replay_from_wal` from tip.

### Critique
- Fill recovery path is conceptually sound.
- Correctness depends on tip monotonicity and complete replay of all lifecycle side effects tied to fills.

---

## C4: Persist Flush Error And Retry Behavior

### Trigger
- `flush_batch` fails (transient PG error, lock, connection reset).

### Current behavior evidence
- On error, worker logs warning and retries the same pending batch (`rsx-risk/src/persist.rs`).
- Batch is retained in-memory until a successful commit.

### Value impact
- Event drop risk is materially reduced for transient DB failures.
- During prolonged outage, state commit latency grows and risk backpressure/stall becomes dominant.

### Severity
- **P1 availability risk** (can degrade to P0 if outage + operator actions cause unsafe failover choices).

### Immediate/simple fix
1. Add exponential backoff + jitter to retry loop to reduce DB thrash.
2. Add durable local spill file when retries exceed threshold.
3. Expose metric/alert: `persist_flush_failed_batches_total` and `persist_oldest_unflushed_ms`.

---

## C5: `ORDER_FAILED` Replay Release Coverage

### Trigger
- Order fails in ME path; release may be applied in live run.
- Crash before release persistence completes.

### Current behavior evidence
- Live path releases on `RECORD_ORDER_FAILED` (`rsx-risk/src/main.rs`).
- Replay now also handles `RECORD_ORDER_FAILED` (`rsx-risk/src/replay.rs`).

### Value impact
- Restart replay now releases frozen margin for failed orders as well.
- Residual risk remains only when lifecycle WAL entries are missing/corrupt.

### Severity
- **Resolved as immediate gap; residual P2/P1 depending on WAL integrity**.

### Immediate/simple fix
1. Add regression test: fail-order -> crash -> replay -> frozen margin returns to expected value.
2. Add invariants at startup: `account.frozen_margin >= sum(active freezers?)` sanity checks.

---

## C6: Replica Promotion Path Is Not Hot-State Durable

### Trigger
- Main loses lease, replica promotes.

### Current behavior evidence
- Replica buffers fills and applies tips in-memory.
- On promotion, code calls `run_main(...)`, constructing a new shard and reloading baseline.

### Value impact
- Not direct loss if WAL intact.
- Promotion is effectively rebootstrap, not seamless hot takeover.

### Critique
- Safety can be acceptable, but RTO and determinism are weaker than implied by buffered promotion logic.

### Improvement
1. Promote current shard state directly, then continue main loop.
2. Or explicitly remove pseudo-hot promotion behavior and document as restart-based failover.

---

## C7: Postgres Outage / Slowdown

### Trigger
- DB unavailable or very slow.

### Current behavior evidence
- Persist ring can fill; `backpressured` stalls risk `run_once`.

### Value impact
- Designed to protect durability by stalling hot path.
- Throughput collapse and order acceptance degradation under sustained outage.

### Critique
- Correct durability bias.
- Needs explicit operational SLO and alerts to prevent silent slow bleed.

### Improvement
1. Add clear reject mode once backpressure threshold exceeded (instead of indefinite stall).
2. Export `persist_ring_slots` and `backpressured` metrics.

---

## C8: Matching Engine Crash (WAL Intact)

### Trigger
- ME process crash/restart.

### Value impact
- In-flight non-WAL state lost; persisted WAL state replayable.
- Risk can catch up via replay.

### Critique
- Core durability model is WAL-first and mostly sound.

### Improvement
1. Ensure all order terminal events are WAL-emitted and replay-consumed by Risk.

---

## C9: WAL Corruption / Disk Loss

### Trigger
- Corrupt WAL segment, storage loss.

### Value impact
- Potentially unrecoverable fills and derived state.

### Critique
- Highest-impact catastrophic class.

### Improvement
1. CRC scan job + quarantine on first detection.
2. Off-host archive and restore drills.
3. Start-up verifier hard-fail on sequence gap/corruption.

---

## C10: Config Applied Around Crash

### Trigger
- `CONFIG_APPLIED` arrives near process restart.

### Current behavior evidence
- Gateway and Risk track versions in memory and reload env-based overrides.
- No durable persisted config version store in these services.

### Value impact
- Validation/fee params can temporarily drift after restart.
- Indirect value risk (wrong rejects/accepts) rather than ledger loss.

### Improvement
1. Persist effective config version per symbol in Postgres.
2. Replay/apply config records from WAL on restart.

---

## C11: Risk Split-Brain (Dual Main)

### Trigger
- Advisory lock anomalies, operator error, or network partition edge.

### Value impact
- Concurrent processing ambiguity, duplicate writes.
- `fills` table has no uniqueness constraint on `(symbol_id, seq)`.

### Severity
- **P0 operational hazard**.

### Immediate/simple fix
1. Add unique constraint on `fills(symbol_id, seq)`.
2. Add startup guardrail: hard fail if lock state unexpected.
3. Alert on duplicate seq insert violations.

---

## C12: Marketdata Crash

### Trigger
- Marketdata service restart/failure.

### Value impact
- No direct balances/positions impact.

### Recovery
- Rebuild shadow state via replay + snapshots.

### Critique
- Visibility outage only; acceptable if bounded by RTO target.

---

## 4) Worst Five Issues (Terse)

1. Reserve-to-send gap can strand margin for orders never reaching ME (`C2`).
2. Persist retries are in-memory only; prolonged DB outage can still create large commit delay (`C4`).
3. Risk split-brain is under-defended; fills table lacks seq uniqueness (`C11`).
4. Config version/application is runtime-only and non-durable (`C10`).
5. Promotion path behaves like rebootstrap, not true hot takeover (`C6`).

---

## 5) Immediate + Simple Fix Set

These are high-value and low-complexity relative to impact:

1. `DONE`: replay handling for `RECORD_ORDER_FAILED` in `rsx-risk/src/replay.rs`.
2. `DONE`: retry failed persist batches instead of dropping them in `run_persist_worker`.
3. `DONE`: add DB uniqueness on `fills(symbol_id, seq)` migration.
4. `OPEN`: add explicit metrics/alerts for persist batch failures and backpressure duration.
5. `OPEN`: add startup invariant log block proving shard ownership and modulo routing (`shard_id`, `shard_count`, derived rule).

---

## 6) Deeper Structural Fixes (Non-trivial)

1. Outbox/2-phase intent between risk reserve and ME submit to close reserve-send crash gap.
2. Durable, replayable config stream/state across Gateway and Risk.
3. True hot promotion path for replica without throwing away buffered in-memory state.
4. End-to-end reconciliation job that verifies:
- `account.frozen_margin == sum(order_freezes by user)`
- position/account state matches fill-derived recomputation at tip.

---

## 7) Crash Test Plan (Must Exist in CI)

1. `reserve_then_crash_before_me_send`:
- Accept order, crash Risk before ME send, restart, verify no permanent freeze leak.

2. `order_failed_then_crash_before_flush`:
- Produce `ORDER_FAILED`, crash before flush, restart, replay, verify freeze released.

3. `persist_flush_transient_error`:
- Inject one failing transaction, ensure batch retry and eventual commit.

4. `dual_main_guard`:
- Simulate lock split condition, verify secondary refuses to process.

5. `config_applied_restart`:
- Apply config, restart Gateway/Risk, verify same effective symbol config after recovery.

---

## 8) Bottom Line

The system has strong WAL-centric foundations for fill durability, and the added `order_freezes` ledger materially improved crash recoverability. The remaining high-risk gaps are mostly at boundaries between reservation, forwarding, and persistence error handling. Closing those boundary gaps will produce the largest reliability gain per unit of engineering effort.
