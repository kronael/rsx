---
status: shipped
---

# TESTING-MARK.md — Mark Price Aggregator Tests

Source spec: [MARK.md](MARK.md)

Binary: `rsx-mark` (standalone service)

## Table of Contents

- [Requirements Checklist](#requirements-checklist)
- [Unit Tests](#unit-tests)
- [Benchmarks](#benchmarks)
- [Integration Points](#integration-points)

---

## Requirements Checklist

| # | Requirement | Source |
|---|-------------|--------|
| K1 | Aggregate mark prices from multiple exchange WS feeds | §1 |
| K2 | Median aggregation across non-stale sources | §4 |
| K3 | Staleness threshold: 10s per source | §4 |
| K4 | Single source: use that price directly | §4 |
| K5 | Zero sources: no publish (risk falls back to index) | §4 |
| K6 | Staleness sweep every 1s | §4 |
| K7 | WalWriter appends MarkPriceEvent records | §1, §5 |
| K8 | DxsReplay server for subscriber broadcast | §5 |
| K9 | PriceSource trait for exchange connectors | §3 |
| K10 | Reconnect backoff: 1/2/4/8s, cap 30s | §3 |
| K11 | Source connectors push via SPSC to aggregation | §1 |
| K12 | Main loop single-threaded, busy-spin | §6 |
| K13 | Recorder archives mark price stream | §1, DXS.md §8 |
| K14 | MarkPriceEvent: symbol_id, mark_price, ts, mask, count | §2 |
| K15 | Env config: staleness_ns, per-source enabled flag | §7 |
| K16 | Main loop: drain rings -> staleness sweep -> wal flush | §6 |
| K17 | Coinbase source disabled by default (enabled=false) | §7 |
| K18 | WS tasks on separate tokio runtime, main loop busy-spin | §6 |
| K19 | Source mask bitmask of contributing sources | §2 |
| K20 | SymbolMarkState indexed by symbol_id in Vec | §2 |
| K21 | WAL flush every 10ms via wal.maybe_flush() | §6 |

---

## Unit Tests

See `rsx-mark/tests/` — aggregation_test.rs, staleness_test.rs,
source_test.rs, config_test.rs.

Source connector tests (Binance, Coinbase) live in the same directory.
Coinbase connector is stubbed (disabled by default per K17).

---

## Benchmarks

See `rsx-mark/benches/` for Criterion benchmarks.

Targets from MARK.md §9:

| Path | Target |
|------|--------|
| Source to publish (end-to-end) | <100us |
| Publish to risk receipt (network) | <1ms |
| Aggregation per symbol | <500ns |
| Staleness sweep (100 symbols) | <50us |

---

## Integration Points

- Risk engines receive MarkPriceRecord via CMP/UDP.
- Recorder connects as DXS consumer for archival (DXS.md §8).
- WalWriter from rsx-dxs crate (DXS.md §3).
- DxsReplay server from rsx-dxs crate (DXS.md §5).
- SPSC rings from source connectors to aggregation loop
- System-level: mark price stale -> risk falls back to
  index price (RISK.md §4)
- System-level: mark price divergence triggers liquidation
  (TESTING-RISK.md, TESTING-LIQUIDATOR.md)
- Env config loaded at startup (MARK.md §7)
- Async WS connector tasks on separate tokio runtime (MARK.md §6)
