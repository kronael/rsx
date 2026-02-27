# Plan: Phase 3+4 — Marketdata Fan-Out + Mark Price

## Context

Project: RSX perpetuals exchange (Rust, monoio, CMP/UDP)
Goal: Fix compile errors in rsx-marketdata and rsx-mark,
add ME→Marketdata CMP fanout, wire mark price deps.

Both crates have substantial implementations (1300+ LOC
each, 58 tests each) but don't compile due to missing
deps and a Clone issue.

---

### Stage 1: Fix rsx-marketdata compile error

**Goal**: Fix ShadowBook Clone issue in state.rs
**Files**: rsx-marketdata/src/state.rs
**Subagent**: improve
**Dependencies**: []
**Verification**:
- [ ] cargo check -p rsx-marketdata passes
- [ ] cargo test -p rsx-marketdata passes

**Details**:

The error is at state.rs:35:
```rust
books: vec![None; max_symbols],
```
`ShadowBook` doesn't implement Clone (and shouldn't — it
wraps an Orderbook). Fix by initializing with a loop:
```rust
books: (0..max_symbols).map(|_| None).collect(),
```
This avoids needing Clone. Read state.rs first to
understand the full context.

---

### Stage 2: Fix rsx-mark compile errors

**Goal**: Add missing dependencies to rsx-mark/Cargo.toml
**Files**: rsx-mark/Cargo.toml, rsx-mark/src/source.rs
**Subagent**: improve
**Dependencies**: []
**Verification**:
- [ ] cargo check -p rsx-mark passes
- [ ] cargo test -p rsx-mark passes

**Details**:

source.rs uses tokio, tokio-tungstenite, futures-util,
serde_json but they're not in Cargo.toml. The 24 compile
errors are all from this.

Read rsx-mark/Cargo.toml and rsx-mark/src/source.rs first.
Then add the missing deps:
```toml
tokio = { version = "1", features = ["rt-multi-thread", "time", "sync", "macros"] }
tokio-tungstenite = "0.24"
futures-util = "0.3"
serde_json = "1"
```

Check exact version constraints by looking at what other
crates in the workspace use (e.g., rsx-dxs/Cargo.toml
or rsx-risk/Cargo.toml for tokio version).

Also check if source.rs has any other issues beyond deps.

---

### Stage 3: ME → Marketdata CMP fanout

**Goal**: Add second CMP sender in ME for marketdata events
**Files**: rsx-matching/src/main.rs
**Subagent**: improve
**Dependencies**: [1]
**Verification**:
- [ ] cargo check -p rsx-matching passes
- [ ] ME sends OrderInserted, OrderCancelled, Fill to marketdata
- [ ] ME does NOT send OrderDone to marketdata (per MD20)

**Details**:

Currently ME has one CMP sender (to Risk at risk_addr).
Need a second CMP sender to Marketdata.

In main.rs:
1. Add env var `RSX_MD_CMP_ADDR` (default "127.0.0.1:9103")
2. Create second CmpSender to marketdata addr
3. After the existing event send loop, add a second loop
   that sends to marketdata — but only Fill, OrderInserted,
   OrderCancelled (NOT OrderDone per MARKETDATA.md spec).

Read rsx-matching/src/main.rs first to understand the
current structure. The pattern is identical to the existing
cmp_sender usage.

Add a helper function:
```rust
fn send_event_marketdata(
    sender: &mut CmpSender,
    event: &rsx_book::event::Event,
    symbol_id: u32,
    ts_ns: u64,
) -> io::Result<()> {
    match *event {
        // Fill, OrderInserted, OrderCancelled only
        // Skip OrderDone (MD20)
        // Reuse same record construction as send_event_cmp
    }
}
```

Also add tick/recv_control for the new sender in the loop.

---

### Stage 4: Verification + PROGRESS.md

**Goal**: Full workspace check, run all tests, update progress
**Files**: PROGRESS.md
**Subagent**: improve
**Dependencies**: [1, 2, 3]
**Verification**:
- [ ] cargo check --workspace passes (all 9 crates)
- [ ] cargo test for all non-docker crates passes
- [ ] PROGRESS.md updated
