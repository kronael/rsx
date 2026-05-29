# Failure-mode test coverage audit (2026-05-29)

Audit of every failure mode on the order path (see `specs/2/28-risk.md`
Return Path → Failure handling) against the actual test suite. Three
parallel reviewers read test **bodies** (not names) across risk,
transport/replication, and gateway/ME/marketdata + scenario docs.

## Coverage matrix

| FM | Failure mode | Status | Evidence / gap |
|----|--------------|--------|----------------|
| FM1 | GW input reject (auth/malformed/symbol/tick-lot-zero) | ✅ COVERED | gateway `jwt_test`, `records_test`, `prevalidation_test`, `convert_test`; ME re-check `order_processing_test` |
| FM2 | GW crash → stateless reconnect/resubmit | 🟡 PARTIAL | `gateway/replay_drain_test` (WAL rebuild), `pending_test` (timeout). No client reconnect→resubmit round-trip; statelessness only implicit |
| FM3 | casting/UDP datagram loss + recovery | 🟡 PARTIAL | NAK recovery tested (`cast_test::nak_retransmit_from_wal`) but **no test drops a real datagram** — loss is simulated by injecting a NAK |
| FM4 | Risk pre-trade margin REJECT | ✅ COVERED | `margin_test`, `shard_test` (insufficient, reduce-only, liquidation, not-in-shard) |
| FM5 | Risk crash → failover/promotion/advisory-lock/PG-reload | ✅ COVERED (split) | `replica_test`, `promotion_e2e_test`, `persist_test`. Gap: full `load→load_state→replay` chain never one test |
| **FM6** | **Orphan freeze (freeze pre-send, crash before ME accept)** | **❌ GAP + code bug** | No test; recovery has no reconcile step. **bugs.md ORPHAN-FREEZE.** Highest priority |
| FM7 | Risk→ME loss → resubmit; freeze self-heals | 🟡 PARTIAL | self-heal depends on FM11 replay-rebuild which is itself untested |
| FM8 | ME outcomes: post-only→CANCELLED, IOC→DONE, FOK→FAILED, reduce-only→FAILED | ✅ COVERED | `book/post_only_test`, `book/matching_test`, `matching/order_processing_test` (full matrix) |
| FM9 | ME crash → WAL-authoritative replay rebuild | 🟡 PARTIAL | `replay_after_snapshot_test`, `replay_after_fault_test` rebuild — but `book_state()` equality is **sorted/lossy**: drops FIFO time-priority + slab identity. "Bit-identical" overstated |
| FM10 | ME→GW confirmation lost, fill durable → reconcile via fills-since-seq | 🟡 PARTIAL | drain-from-seq mechanism tested; no test models a *lost confirmation* + redrain asserting no double-fill |
| **FM11** | **ME→Risk settle: replay rebuilds freezes from WAL OrderAccepted** | **❌ GAP** | `replay_freeze_order` (OrderAccepted→re-freeze) has **zero tests**; replay release-on-Cancelled/Failed untested (only OrderDone) |
| FM12 | Duplicate-order (order-id) dedup on risk | ❌ GAP | All dedup tested is by fill **seq**, not order-id; freeze double-apply guard untested |
| FM13a | exactly-once execution under retry (no double-fill) | 🟡 PARTIAL | `DedupTracker` unit-tested; **no GW→ME duplicate-order e2e asserting single fill** (invariant #2 — high value) |
| FM13b | invariants: fills<done, one-completion, FIFO, no-cross | ✅ COVERED | `matching/invariant_test`, `book/matching_test`, `gateway/stream_ordering_test` |
| FM14 | replay idempotency (replay-twice=same), tip monotonic, from tip+1 | 🟡 PARTIAL | dedup/tip/resume COVERED; **double-`replay_from_wal` = same state never tested** (invariant #8) |

Additional transport gaps:
- **Two-tier NAK not separated** — both NAK tests fall through to cold WAL (FillRecord 128B overflows the ring slot); hot-ring hit never proven.
- **Full FAULTED→TCP-replay→resume loop never wired e2e** (the SEQ-1 area; `cmp_ingest_test` asserts fault is *never* taken → no regression guard for SEQ-1).
- **Endpoint-list federation fallthrough** (`RECORD_REPLICATION_NOT_AVAILABLE`, v0.4.0) — **zero coverage**; every consumer test uses a single endpoint.
- **No real packet-loss injection** anywhere; spec's 5%-loss harness is aspirational.

Scenario docs:
- **0 of 19 `CRASH-SCENARIOS.md` entries have a process-level crash test.** S13 (marketdata shadow-book rebuild) + partial S1/S8/S12 have in-process WAL-replay analogues; the rest are doc-only. All durability-bound claims + SQL verification queries are unexecuted.

Spec drift found (fix for consistency):
- `35-testing-cast.md` references retired `StatusMessage`/flow-control + nonexistent files (`cmp_e2e_test.rs`, fault-injection harness).
- `36-testing-replication.md` points at `tests/wal_test.rs` paths that are actually `src/*_test.rs`. Specs overstate coverage.

## Prioritized test plan

**P0 — correctness gaps (write now):**
1. **FM6 orphan freeze** — recovery test: seed PG `frozen_orders` row with no matching WAL `OrderAccepted`; assert recovery drops it. Gated on the `bugs.md` fix (reconcile-from-WAL). Test + fix together.
2. **FM11 replay_freeze_order** — replay a WAL with `OrderAccepted` (reduce_only=0) and assert `frozen_for_user` rebuilt; add replay release-on-Cancelled + on-Failed.
3. **FM13a exactly-once under retry** — submit duplicate order_id through GW→ME (or ME process_new_order + DedupTracker) and assert single fill, single completion, freeze applied once.
4. **FM14 double-replay idempotency** — call `replay_from_wal` twice on the same WAL; assert identical shard state.

**P1 — integration/regression:**
5. **SEQ-1 regression** — wire gap-detect → FAULTED → TCP replay-from-tip+1 → reset → live resume in one consumer test (would catch SEQ-1 reintroduction).
6. **FM12 duplicate-order freeze guard** on risk (companion to FM13a).
7. **Endpoint federation fallthrough** — two endpoints, first returns NOT_AVAILABLE, second serves.
8. **FM9 FIFO-identity on replay** — extend replay equality to assert intra-level time priority, not just sorted (price,qty,side).

**P2 — harnesses (larger):**
9. Real packet-loss injection (socket wrapper dropping N% datagrams) → transparent NAK recovery.
10. Process-level crash harness for top `CRASH-SCENARIOS.md` entries (S1 ME, S3 Risk, S13 MD).
11. Two-tier NAK separation (small-payload hot-ring hit vs cold-WAL miss).

**P3 — consistency:**
12. Fix `35-testing-cast.md` + `36-testing-replication.md` drift (retired types, wrong paths, overstated coverage).
