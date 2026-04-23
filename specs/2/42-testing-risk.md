---
status: shipped
---

# TESTING-RISK.md — Risk Engine Tests

Source spec: [RISK.md](RISK.md)

Binary: `rsx-risk` (one process per user shard)

## Table of Contents

- [Requirements Checklist](#requirements-checklist)
- [Unit Tests](#unit-tests)
- [Benchmarks](#benchmarks)
- [Correctness Invariants](#correctness-invariants)
- [Integration Points](#integration-points)

---

## Requirements Checklist

| # | Requirement | Source |
|---|-------------|--------|
| R1 | Fill ingestion: filter by shard, dedup by seq | §1 |
| R2 | Fee calculation on fill (taker + maker) | §1 |
| R3 | Position: long_qty, short_qty, entry_cost tracking | §2 |
| R4 | Account: collateral, frozen_margin tracking | §2 |
| R5 | Portfolio margin across all symbols per user | §3 |
| R6 | Exposure index: users with open positions per symbol | §3 |
| R7 | Index price: size-weighted mid from BBO | §4 |
| R8 | Mark price from DXS consumer (MARK.md) | §4 |
| R9 | Funding rate: f(mark, index), clamped, 8h interval | §5 |
| R10 | Pre-trade risk check before order to ME | §6 |
| R11 | Frozen margin reserved on order, released on done | §6 |
| R12 | Per-tick margin recalc for exposed users | §7 |
| R13 | Liquidation trigger: equity < maint_margin | §7 |
| R14 | Postgres persistence: write-behind 10ms flush | §persistence |
| R15 | Advisory lock: single writer per shard | §replication |
| R16 | Replica: buffer fills, apply on tip sync | §replication |
| R17 | Promotion: acquire lock, apply remaining, go live | §replication |
| R18 | Config updates from ME CONFIG_APPLIED events | §1 |
| R19 | Reduce-only/liquidation orders skip margin check | §6 |
| R20 | Main loop priority: fills > orders > mark > BBO | §main loop |
| R21 | Forward CONFIG_APPLIED to gateway for cache sync | §1 |
| R22 | Backpressure: stall on ring full / flush lag / replica lag | §persistence |
| R23 | Promotion invariant: apply fills up to last tip only | §replication |
| R24 | Replay via DXS consumer from tips + 1, CaughtUp signal | §replication |
| R25 | Missed funding intervals settled on next startup | §5 |
| R26 | ME failover: dedup by (symbol_id, seq), no restart | §ME failover |
| R27 | Account persistence: collateral, frozen_margin to Postgres | §persistence |
| R28 | Both-crash recovery: 100ms data loss bound | §recovery |
| R29 | Index price fallback: no BBO ever -> use mark price | §4 |
| R30 | Funding payments persisted append-only to Postgres | §5 |
| R31 | Fills persisted via COPY binary bulk insert | §persistence |

---

## Unit Tests

See `rsx-risk/tests/` — position_test.rs, margin_test.rs,
price_test.rs, funding_test.rs, fee_test.rs, shard_test.rs,
margin_recalc_test.rs, persist_test.rs, shard_e2e_test.rs,
replication_e2e_test.rs, liquidation_test.rs.

---

## Benchmarks

See `rsx-risk/benches/` for Criterion benchmarks.

Targets from RISK.md §performance:

| Path | Target |
|------|--------|
| Fill processing | <1us |
| Pre-trade check | <5us |
| Per-tick margin | <10us/user |
| BBO -> index price | <100ns |
| Postgres flush | every 10ms |
| Failover detection | ~500ms |
| Replay catch-up | <5s |

---

## Correctness Invariants

1. **Fills never lost** -- sum of applied fills = sum of ME-emitted
   fills (for shard users)
2. **Position = sum of fills** -- verified after every test scenario
3. **Tips monotonic** -- never decreases, even after recovery
4. **Margin consistent with positions** -- recalc from scratch matches
   incremental state
5. **Funding zero-sum** -- per symbol per interval
6. **Exposure index consistent** -- matches actual positions
7. **Advisory lock exclusive** -- at most one main per shard
8. **Seq dedup prevents double-counting** -- replay = no change
9. **Promotion invariant** -- replica applies fills only up to last
   tip from main, never beyond
10. **Backpressure stall** -- hot path stalls when persistence ring
    full, flush lag > 10ms, or replica ring full (100ms bound)
11. **Account balance consistent** -- collateral - fees + rebates +
    realized_pnl + funding = expected balance

---

## Integration Points

- Receives fills/BBO/OrderDone from matching engine via CMP/UDP
  (CONSISTENCY.md §1, event routing table)
- Receives orders from gateway via CMP/UDP (NETWORK.md §data flow)
- Mark prices arrive via CMP/UDP from rsx-mark (RECORD_MARK_PRICE, main.rs)
- Sends orders to matching engine via CMP/UDP (RISK.md §6)
- Sends fills/done to gateway via CMP/UDP (CONSISTENCY.md §1)
- Forwards CONFIG_APPLIED to gateway (RISK.md §1)
- Persists positions/accounts/fills/tips to Postgres via
  write-behind worker (RISK.md §persistence)
- Replica sync: replica.rs + replication_e2e_test.rs shipped
- Advisory lock via Postgres pg_advisory_lock (RISK.md §replication)
- Liquidation via embedded liquidator (LIQUIDATOR.md)
- Funding via embedded funding engine (RISK.md §5)
- ME failover: dedup by (symbol_id, seq) (RISK.md §ME failover)
- Backpressure: CMP flow control (CONSISTENCY.md §3)
- System-level: full crash/recovery tests (TESTING.md §3)
