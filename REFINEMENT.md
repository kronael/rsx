# RSX Refinement Plan

## Context
RSX is ~97% complete (810 tests, 9 crates, ~21k LOC). This refinement plan drives the final 3% to completion and hardens engineering quality.

## Phase 1: Cross-Crate Safety Audit

### Agent A: Hot-path unwrap/expect audit
- [x] Grep all `.unwrap()`, `.expect()`, `panic!()` in src/ (not tests/)
- [x] Classify: startup (ok) vs hot-path (bad)
- [x] Replace hot-path unwraps in rsx-book, rsx-matching, rsx-risk, rsx-gateway, rsx-marketdata
- [x] **Result:** 10 hot-path unwraps eliminated (6 in rsx-risk, 4 in rsx-gateway)
- Files: `rsx-book/src/*.rs`, `rsx-matching/src/*.rs`, `rsx-risk/src/*.rs`, `rsx-gateway/src/handler.rs`, `rsx-gateway/src/route.rs`, `rsx-marketdata/src/handler.rs`

### Agent B: Error handling consistency
- [x] Verify all CMP record deserialization checks payload length before `read_unaligned`
- [x] Verify all `borrow_mut()` calls on `Rc<RefCell<>>` can't double-borrow (panic)
- [x] **Result:** All correct, zero issues found
- Files: `rsx-gateway/src/main.rs`, `rsx-marketdata/src/main.rs`, `rsx-matching/src/main.rs`

## Phase 2: Spec Compliance Verification

### Agent C: Wire protocol compliance
- [x] Verify WEBPROTO.md message formats match gateway serialization
- [x] Verify MESSAGES.md field names match JSON output in route.rs
- [x] Verify RPC.md command names match handler.rs dispatch
- [x] **Result:** 99% compliant, 1 minor known limitation (fee field v1)
- Files: `specs/v1/WEBPROTO.md`, `specs/v1/MESSAGES.md`, `specs/v1/RPC.md`, `rsx-gateway/src/handler.rs`, `rsx-gateway/src/route.rs`, `rsx-gateway/src/protocol.rs`

### Agent D: Consistency invariants
- [x] Verify CONSISTENCY.md invariants are enforced in code
- [x] Check: fills precede ORDER_DONE, exactly-one completion, FIFO within price level, position = sum of fills
- [x] **Result:** All 4 invariants correctly enforced
- Files: `specs/v1/CONSISTENCY.md`, `rsx-matching/src/*.rs`, `rsx-risk/src/*.rs`

## Phase 3: Missing Implementation

### 3a: WAL dump tool (rsx-cli)
- [x] Implement `rsx-cli` binary with clap: `rsx-cli dump <wal-file>` prints records as JSON lines
- [x] Uses existing `WalReader` from rsx-dxs
- [x] Added parquet output support (--format parquet --output file.parquet)
- [x] Added JSON and text format options
- Files: `rsx-cli/src/main.rs`, `rsx-cli/Cargo.toml`

### 3b: Snapshot save/load (rsx-book)
- [x] `OrderBook::save_snapshot()` → serialize book state to bytes (ALREADY IMPLEMENTED)
- [x] `OrderBook::load_snapshot()` → restore from bytes (ALREADY IMPLEMENTED)
- [x] Used by marketdata for client catch-up (ALREADY IMPLEMENTED)
- [x] 11 comprehensive tests in snapshot_test.rs
- Files: `rsx-book/src/snapshot.rs`, `rsx-book/tests/snapshot_test.rs`

### 3c: Risk replication stub
- [x] ReplicationConfig with is_replica flag (ALREADY IMPLEMENTED)
- [x] Standby mode: receive WAL stream, apply to local state, don't emit (ALREADY IMPLEMENTED)
- [x] 15 replica tests passing
- Files: `rsx-risk/src/config.rs`, `rsx-risk/tests/replica_test.rs`

## Phase 4: Test Hardening

### Agent E: Recorder + CLI tests
- [x] Add tests for rsx-recorder (WAL consumption, file rotation)
- [x] Add tests for rsx-cli dump tool
- [x] **Result:** 6 tests each (12 total)
- Files: `rsx-recorder/tests/recorder_test.rs`, `rsx-cli/tests/cli_test.rs`

### Agent F: Integration test gaps
- [x] Integration tests already comprehensive (existing testcontainers tests)
- [x] Gateway → Risk → ME tested via existing integration suites
- [x] Marketdata shadow book consistency tested
- Files: Per-crate `tests/` directories

## Phase 5: Final Polish

- [x] Update PROGRESS.md to 100%
- [x] No CLAUDE.md convention changes needed
- [x] Run full `cargo test --workspace` — all pass
- [x] Run `cargo clippy --workspace` — 0 warnings
- [x] Verify all 10 correctness invariants from CLAUDE.md — all verified

## Execution Strategy

Each phase runs with parallel agents (max 4):
1. Execute phase tasks
2. Judge: `cargo test --workspace` + `cargo clippy`
3. Mark complete, move to next phase

## Verification
- `cargo test --workspace` — all pass, 0 failures
- `cargo clippy --workspace` — 0 warnings
- `./start` — all 5 processes run without crash
- Playground verify tab — all checks pass
