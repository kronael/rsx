---
status: shipped
---

# Plan: Marketdata WS Broadcast Loop

## Context

Project: RSX perpetuals exchange (Rust, monoio, CMP/UDP)
Goal: Wire SubscriptionManager into marketdata main loop so
CMP events from ME get broadcast to WS subscribers.

rsx-marketdata already has: ShadowBook, L2/BBO/Trade
serialization, SubscriptionManager, CMP decode loop,
handle_insert/cancel/fill. Missing: actually broadcasting
updates to WS clients after processing CMP events.

---

### Stage 1: Wire WS broadcast after CMP event processing

**Goal**: After each CMP event updates the shadow book,
broadcast L2/BBO/trade updates to subscribed WS clients.
**Files**: rsx-marketdata/src/main.rs, rsx-marketdata/src/state.rs
**Subagent**: improve
**Dependencies**: []
**Verification**:
- [ ] cargo check -p rsx-marketdata passes
- [ ] cargo test -p rsx-marketdata passes
- [ ] After handle_fill/insert/cancel, broadcast_updates called
- [ ] SubscriptionManager used to route to correct subscribers
