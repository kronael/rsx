---
status: shipped
---

# Plan: Post-Only Order Enforcement

## Context

Project: RSX perpetuals exchange (Rust)
Goal: Reject post-only orders that would cross the book
(i.e., would trade immediately). Post-only orders must
add liquidity only.

post_only field already exists in OrderRequest and wire
records. Just needs enforcement in matching algorithm.

---

### Stage 1: Enforce post-only in matching

**Goal**: If order has post_only=true and would cross
the book (price >= best ask for buy, or <= best bid for
sell), reject with OrderCancelled reason=POST_ONLY.
**Files**: rsx-book/src/matching.rs, rsx-book/src/event.rs
**Subagent**: improve
**Dependencies**: []
**Verification**:
- [ ] cargo check -p rsx-book passes
- [ ] cargo test -p rsx-book passes
- [ ] Post-only buy at/above best ask is cancelled
- [ ] Post-only sell at/below best bid is cancelled
- [ ] Post-only order that doesn't cross is inserted
- [ ] Non-post-only orders unaffected
