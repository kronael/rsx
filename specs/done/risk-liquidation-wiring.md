# Plan: Wire Liquidation into Risk Main Loop

## Context

Project: RSX perpetuals exchange (Rust, CMP/UDP)
Goal: Wire existing liquidation engine into risk shard's
main loop so underwater positions get liquidated.

rsx-risk/src/liquidation.rs already has check_liquidation()
and generate_liquidation_order(). Just needs to be called
from the main loop and forwarded to ME.

---

### Stage 1: Wire liquidation check into risk main loop

**Goal**: After processing fills, check if any user needs
liquidation. If so, generate liquidation order and send
to ME via accepted_producer.
**Files**: rsx-risk/src/main.rs, rsx-risk/src/shard.rs
**Subagent**: improve
**Dependencies**: []
**Verification**:
- [ ] cargo check -p rsx-risk passes
- [ ] cargo test -p rsx-risk passes
- [ ] Liquidation check called after process_fill
- [ ] Generated liquidation orders sent to ME via CMP
