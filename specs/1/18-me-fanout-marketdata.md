---
status: shipped
---

# Plan: ME Fanout to Marketdata

## Context

Project: RSX perpetuals exchange (Rust, monoio, CMP/UDP)
Goal: ME sends events to Marketdata in addition to Risk

**Current state:** ME only sends CMP events to Risk. Marketdata
main.rs already handles CMP decode, shadow book, WS broadcast —
just needs events. One CmpSender addition.

---

### Stage 1: Add Marketdata CmpSender to ME

**Goal**: ME sends Fill, OrderInserted, OrderCancelled, OrderDone
to marketdata in addition to risk.
**Files**: rsx-matching/src/main.rs
**Subagent**: improve
**Dependencies**: []
**Verification**:
- [ ] cargo check --workspace passes
- [ ] cargo test -p rsx-matching passes
- [ ] ME sends to both risk and marketdata addresses
