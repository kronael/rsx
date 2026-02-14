# TODO: Spec Test Gaps (33 items)

Tests specified in TESTING-*.md but not yet implemented.

## Gateway Tests (11 items)

Source: specs/v1/TESTING-GATEWAY.md

### Heartbeat (3)
- [ ] heartbeat_sent_every_5s (config exists, no timer test)
- [ ] heartbeat_timeout_closes_at_10s (config exists, no timer)
- [ ] heartbeat_client_response_resets_timer (need handler)

### Config (2)
- [ ] symbol_not_found_rejects_early (need config cache)
- [ ] config_cache_updated_on_config_applied (need CONFIG_APPLIED)

### E2E (6)
- [ ] ws_new_order_accepted_and_filled
- [ ] concurrent_sessions_isolated
- [ ] fills_precede_order_done_on_wire
- [ ] liquidation_order_routed_correctly
- [ ] circuit_breaker_opens_on_gateway_overload
- [ ] rate_limit_per_user_enforced_e2e

## Risk Engine Tests (11 items)

Source: specs/v1/TESTING-RISK.md

### Integration (3)
- [ ] order_while_user_liquidated_rejected
- [ ] config_applied_event_updates_params
- [ ] config_applied_forwarded_to_gateway

### Phase 4: Replication (4)
- [ ] main_lease_acquired_at_startup
- [ ] replica_promoted_on_main_failure
- [ ] fill_buffering_during_promotion
- [ ] crash_recovery_replays_from_tip

### Phase 5: Full System (4)
- [ ] full_lifecycle_order_to_settlement
- [ ] liquidation_cascade_multiple_users
- [ ] me_failover_dedup_preserved
- [ ] funding_settlement_all_intervals

## Liquidator Tests (11 items)

Source: specs/v1/TESTING-LIQUIDATOR.md

### Multi-Position (3)
- [ ] liquidate_largest_position_first
- [ ] partial_liquidation_reduces_to_target
- [ ] multiple_symbols_liquidated_independently

### Integration (1)
- [ ] new_orders_rejected_during_liquidation

### E2E (7)
- [ ] price_drops_triggers_liquidation
- [ ] cascade_liquidation_across_users
- [ ] liquidation_persisted_to_postgres
- [ ] recovery_resumes_pending_liquidations
- [ ] order_failed_retries_with_slip
- [ ] insurance_fund_absorbs_deficit
- [ ] symbol_halt_on_repeated_failure
