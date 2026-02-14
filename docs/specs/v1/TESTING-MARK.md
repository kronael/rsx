# TESTING-MARK.md — Mark Price Aggregator Tests

Source spec: [MARK.md](MARK.md)

Binary: `rsx-mark` (standalone service)

## Table of Contents

- [Requirements Checklist](#requirements-checklist)
- [Unit Tests](#unit-tests)
- [E2E Tests](#e2e-tests)
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

### Aggregation Logic

```rust
// single source
aggregate_single_source_uses_price_directly
aggregate_single_source_updates_mask_and_count
aggregate_source_update_replaces_previous

// multi-source median
aggregate_two_sources_median_is_avg
aggregate_three_sources_median_is_middle
aggregate_five_sources_median_correct
aggregate_even_count_picks_lower_median

// staleness
aggregate_stale_source_excluded
aggregate_all_sources_stale_no_publish
aggregate_one_fresh_one_stale_uses_fresh
aggregate_source_becomes_stale_triggers_reagg
staleness_threshold_exactly_10s

// edge cases
aggregate_source_id_out_of_range
aggregate_max_8_sources
aggregate_same_price_all_sources
aggregate_price_zero_handled
aggregate_large_price_difference_still_median
```

### Staleness Sweep

```rust
sweep_removes_newly_stale_source
sweep_reaggregates_and_publishes
sweep_no_change_if_all_fresh
sweep_no_publish_if_all_stale
sweep_interval_approximately_1s
sweep_100_symbols_iterates_all
```

### SymbolMarkState

```rust
mark_state_initial_all_none
mark_state_source_mask_correct_bitmask
mark_state_source_count_matches_fresh
mark_state_mark_price_updated_on_aggregate
```

### Source Connectors

```rust
binance_source_parses_mark_price_json
binance_source_maps_symbol_to_id
binance_source_unknown_symbol_ignored
binance_source_pushes_to_spsc
binance_reconnect_backoff_1_2_4_8_30
binance_reconnect_caps_at_30s
binance_reconnect_resets_on_message
coinbase_source_stub_implements_trait
```

### Source Mask

```rust
source_mask_single_source_sets_bit
source_mask_two_sources_sets_both_bits
source_mask_stale_source_clears_bit
source_mask_max_8_bits
```

### Config

```rust
config_parse_valid_env
config_staleness_ns_overrides_default
config_source_enabled_false_skipped
config_listen_addr_and_wal_dir
config_stream_id_set
config_reconnect_base_and_max_ms
```

---

## E2E Tests

```rust
// full pipeline
source_update_to_wal_append_lifecycle
two_sources_aggregated_median_published
source_disconnect_reconnect_resumes
source_stale_reaggregation_publishes

// DXS integration
wal_contains_mark_price_events
dxs_replay_serves_historical_marks
dxs_live_tail_receives_new_marks
dxs_consumer_connects_and_replays

// multi-symbol
100_symbols_all_receive_updates
symbol_update_only_affects_that_symbol
rapid_updates_same_symbol_latest_wins

// failure modes
all_sources_disconnect_no_publish
single_source_flapping_marks_stable
source_sends_garbage_price_handled

// config
disabled_source_not_started
staleness_ns_configurable_non_default
wal_flush_interval_approximately_10ms
main_loop_drain_then_sweep_then_flush
```

---

## Benchmarks

```rust
bench_aggregate_single_source      // target <100ns
bench_aggregate_3_sources_median   // target <500ns
bench_aggregate_8_sources_median   // target <500ns
bench_staleness_sweep_100_symbols  // target <50us
bench_source_to_publish_e2e        // target <100us
bench_wal_append_mark_event        // target <200ns
bench_main_loop_idle               // target <1us
bench_sustained_100_updates_sec    // target stable latency
bench_source_mask_computation      // target <50ns
```

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
- Async WS connector tasks on separate tokio runtime
  (MARK.md §6)
