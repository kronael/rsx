# Bucket 4 findings (specs 38-49)

## 38-testing-liquidator.md

**Status recommendation**: shipped (heavy bloat)

- §Requirements Checklist — match (all 38 traceable)
- §Unit Tests Core/Edge/Config — bloat (~90 lines verbatim test names in `rsx-risk/tests/liquidation_test.rs`)
- §Implementation Status table — drift (stale; TODO items now DONE: `multiple_positions_all_get_orders`, `partial_fill_reduces_position`, `full_fill_closes_position`, `new_orders_rejected_during_liquidation`, `order_failed_symbol_halted_pauses_symbol`)
- §E2E / Integration / Benchmark code blocks — bloat
- §Correctness Invariants / Integration Points — match

**Actions:**
- Delete §Unit Tests / E2E / Benchmark code blocks
- Delete §Implementation Status table (belongs in git history)
- Keep Requirements Checklist and Correctness Invariants

## 39-testing-mark.md

**Status recommendation**: shipped (bloat)

- §Requirements K1-K21 — match (K20 Vec indexed by symbol_id confirmed; K14 minor: spec `ts`, code `ts_ns`)
- §Unit Tests Aggregation/Staleness/SymbolMarkState — match (tests exist in `aggregator_test.rs`); bloat (80 names)
- §Unit Tests Source Connectors — partial (no `binance_source_*` test file found; only `aggregator_test.rs`, `config_test.rs`, `types_test.rs`)
- §Unit Tests Config — match
- §E2E Tests / Benchmarks — bloat / unshipped (no e2e files; `mark_bench.rs` has benches)
- §Integration Points — match

**Actions:**
- Delete test name code blocks; reference test file paths
- Mark source connector tests as unshipped or remove from spec
- Collapse Benchmarks section

## 40-testing-marketdata.md

**Status recommendation**: shipped

- §Requirements MD1-? — match (MD1 shadow book uses rsx-book confirmed)
- §MD11 — drift (spec says epoll; marketdata uses monoio/io_uring per `main.rs:167-182`)
- §Unit Tests Shadow/BBO/Snapshot/Delta — bloat (50+ names; tests exist in `shadow_test.rs`, `snapshot_consistency_test.rs`, `subscription_test.rs`)
- §Unit Tests WS Frame/Event Routing/Backpressure — partial (`event_routing_test.rs`, `seq_gap_test.rs` exist; backpressure tests not found by name)
- §E2E recovery — drift (parenthetical says "DXS replay not wired in v1" but `replay_e2e_test.rs` and `replay_test.rs` exist)
- §Benchmarks — bloat (80 lines; `marketdata_bench.rs` exists)
- §Integration Points — match

**Actions:**
- Fix MD11 to "monoio (io_uring)"
- Delete test name code blocks
- Remove "DXS replay not wired" disclaimer (it IS wired)
- Delete Benchmarks section

## 41-testing-matching.md

**Status recommendation**: shipped (bloat)

- §Requirements M1-M31 — match (M5 dedup 5min confirmed; M19 5-zones confirmed; M23 zero-heap not auto-tested — soft unshipped)
- §Unit Tests Order Processing/Dedup/Event Fan-Out — bloat (~100 lines; tests in `order_processing_test.rs` and `event_test.rs`)
- §Unit Tests Compression Map/Slab/Best Bid — partial (tests in rsx-book crate, correctly attributed)
- §E2E Recentering/Smooshed Tick — match (`smooshed_test.rs`, `lifecycle_test.rs`)
- §Benchmarks — bloat (keep the target table, delete code block)
- §Integration Points — match

**Actions:**
- Delete test name code blocks
- Keep phase headers as structural notes
- Keep benchmark target table, delete bench function list

## 42-testing-risk.md

**Status recommendation**: shipped (drift)

- §Requirements R1-? — match (mostly)
- §R8 Mark price from DXS consumer — drift (Integration Points says "Mark prices from DXS are not wired into risk in v1" but `main.rs:660` processes RECORD_MARK_PRICE via CMP/UDP; arrive via CMP from rsx-mark, not DXS)
- §Integration Points Replica sync — drift ("not implemented in v1" false; `replica.rs` + `replication_e2e_test.rs` exist with promotion + lease)
- §Unit Tests Phase 1/Phase 2 — bloat (~160 lines; tests across 11 files)
- §Implementation Status table — drift (stale; listed TODOs now exist in `shard_test.rs`, `replication_e2e_test.rs`)
- §E2E / Integration / Benchmarks — bloat (~130 lines)
- §Correctness Invariants — match (keep)

**Actions:**
- Delete "not wired" and "not implemented" false disclaimers
- Delete §Unit Tests code blocks, §Implementation Status table
- Keep Correctness Invariants and benchmark target table

## 43-testing-smrb.md

**Status recommendation**: reference (or archive)

- Overall: tests a design concept (SMRB = Shared Memory Ring Buffer) that isn't a standalone crate. Actual SPSC = external `rtrb` crate. No rsx-authored SMRB test files exist (zero grep matches for listed function names).
- §Requirements S1-S9 — partial match (rtrb provides these; no custom code to verify)
- §S12 huge pages — unshipped (no huge-page code anywhere)
- §S13 no_std — unshipped (no `#![no_std]`)
- §S11 core pinning — match (`core_affinity` in matching/risk/mark mains)
- §Unit Tests / E2E Tests — bloat + unshipped (all ~40 listed don't exist)
- §Benchmarks — bloat (not found)
- §Integration Points — match

**Actions:**
- Reduce spec to requirements checklist + integration points
- Note: SPSC = external rtrb, link to rtrb docs
- Delete all test and benchmark sections

## 44-testing.md (umbrella strategy spec)

**Status recommendation**: shipped (drift)

- §Test Organization / Make Targets — match (all targets exist in Makefile:259-295)
- §Test count "877 tests passing" — drift (actual `#[test]` count: 1035)
- §Deferred Persistence/Replay Tests — drift (`persist_test.rs` exists)
- §Deferred Multi-Machine Replication — drift (`replica.rs`, `replica_test.rs`, `replication_e2e_test.rs` shipped)
- §Deferred WebSocket Market Data Tests — drift (multiple test files exist)
- §Example tests code blocks — bloat
- §Component Test Specs table — match
- §Test Framework / CI/CD / Correctness Invariants — match

**Actions:**
- Fix test count
- Remove 3 stale deferred items (all shipped)
- Delete example code blocks

## 45-tiles.md

**Status recommendation**: shipped (minor drift)

- §Overview / Processes / Tile Pattern — match
- §Tile Pattern diagram — drift (shows ME→SPSC→WAL Writer→DxsReplay as separate tiles; WalWriter is inline in ME main loop; DxsReplay runs on `std::thread::spawn`, not SPSC-connected tile)
- §Runtime Selection — match (monoio for gateway + marketdata, tokio for aux)
- §Networking Stack monoio — match
- §Reference Implementation path `/home/onvos/app/trader/monoio-client/` — unverifiable (external path)
- §Tiles Within Each Process WAL Writer — drift (says `wal.maybe_flush()` but actual method is `WalWriter::flush()`)
- §Performance Targets — match
- §Future Userspace Networking — match

**Actions:**
- Fix tile diagram annotation for WAL Writer (inline) vs DxsReplay (thread)
- Remove external path reference or move to internal note
- Fix `maybe_flush()` reference to `flush()`

## 46-trade-ui.md

**Status recommendation**: partial

- §Current State What Works — match (React SPA, playground proxy, direct port access)
- §What Is Broken Open positions not displayed — drift (SHIPPED: `usePrivateWs.ts` onopen calls `fetchPositions()`; `Positions.tsx` renders)
- §What Is Broken WS reconnect — drift (SHIPPED: exponential backoff 1000→30000ms, `WsStatus.RECONNECTING`, TopBar shows "reconnecting")
- §Fix Plan Fix 1 nginx WS upgrade — unshipped (no nginx config in repo)
- §Fix Plan Fix 2 Docs 502 — unshipped (no nginx config)
- §Fix Plan Fix 3, 4, 5 — match (shipped)
- §Acceptance Criteria — partial (positions/WS done; nginx open)
- §Implementation Tasks 1-2 — unshipped (nginx, external); Tasks 3-7 shipped

**Actions:**
- Mark Tasks 3-7 done; update "broken" list
- Mark nginx tasks 1-2 as external/deferred
- Consider moving spec to ops runbook

## 47-validation-edge-cases.md

**Status recommendation**: shipped (minor drift)

- §Validation Layers diagram — match (3-layer Gateway→Risk→ME)
- §1 Field Validation §1.4 TIF enum safety — drift (spec warns about `unsafe transmute`; actual code `protocol.rs:238-241` validates `if tif > 2 { return Err }`; unsafe path doesn't exist)
- §2.1 Notional Overflow — match (i128 intermediate + `i64::try_from`)
- §2.3 Position Flip — match
- §3.5 IOC/FOK Edge Cases — match (`rsx-book/src/matching.rs:113-172`)
- §3.6 Self-Trade Prevention — match (no STP in v1, documented)
- §6 Testing Strategy — bloat (~15 test function names that don't exist as files)
- §7 Monitoring & Alerts — match
- §Cross-References — match

**Actions:**
- Fix §1.4: show actual `if tif > 2` guard, delete unsafe example
- Delete code block in §6

## 48-wal.md

**Status recommendation**: shipped

- §Record Format header — match (`WalHeader` in `rsx-dxs/src/header.rs`)
- §Version Policy — match
- §Scope claim — match
- §Goals / Architecture / Local WAL Buffer — match (`WalWriter` in `rsx-dxs/src/wal.rs`)
- §WAL Flush 10ms guarantee — match (`maybe_flush()` IS a real public API method)
- §Hard Backpressure Rules — match
- §Offload Worker / Replica Sync — match
- §Critique and Verification — match
- §Replay Edge Cases — match

**Actions:**
- None; spec is clean and accurate

## 49-webproto.md

**Status recommendation**: shipped (drift)

- §Frame Shape / Types / Enums — match
- §N/C/U/F/E/H messages — match
- §Market Data Messages (S/X/BBO/B/D) — match
- §T Trade "Post-MVP: not implemented in v1" — drift (T IS implemented: `serialize_trade` in `rsx-marketdata/src/protocol.rs:46`; `main.rs:424-458` builds + sends trade messages)
- §M Metadata Query "Post-MVP" — match (no `{M:[]}` handler)
- §O/P/A/FL/FN queries "Post-MVP" — match (no gateway handlers). Note: `/v1/positions`, `/v1/account`, etc. ARE in playground `server.py:5924,6003,6074,5969` but NOT gateway
- §Reconnection O/P/A queries — drift (recommends `{O:[]}`, `{P:[]}`, `{A:[]}` which are post-MVP; should direct to REST endpoints on playground)
- §Q Liquidation Event — match

**Actions:**
- Remove "Post-MVP" disclaimer from T section (shipped)
- Fix Reconnection guidance (direct to REST, not unimplemented WS queries)
- Add note that `/v1/positions` etc. are playground endpoints, not gateway
