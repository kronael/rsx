---
status: shipped
---

# Plan: Risk Mark Price DXS Consumer

## Context

Project: RSX perpetuals exchange (Rust, CMP/UDP)
Goal: Risk engine receives mark prices from Mark process
so liquidation checks use real mark prices instead of
skipping when no mark price available.

Mark process publishes MarkPriceEvent records via CMP.
Risk needs a CMP receiver for mark prices, decode them,
and store in shard state for use by check_liquidation.

---

### Stage 1: Add mark price CMP receiver to risk

**Goal**: Risk receives MarkPriceEvent from Mark process
via CMP, stores latest mark price per symbol in shard.
**Files**: rsx-risk/src/main.rs, rsx-risk/src/shard.rs
**Subagent**: improve
**Dependencies**: []
**Verification**:
- [ ] cargo check -p rsx-risk passes
- [ ] cargo test -p rsx-risk passes
- [ ] Mark price CMP receiver created in main
- [ ] Mark prices stored per symbol in shard state
- [ ] check_liquidation uses stored mark price
