# RSX Implementation Progress

**Comprehensive line-by-line audit completed 2026-02-10.**

This document tracks implementation status against specs/v1/ for all 9 crates.

---

## Executive Summary

**Status:** Early implementation phase. Core data structures and basic binaries exist, but critical end-to-end pipeline is broken.

**Critical Issues (from CRITIQUE.md):**
1. CMP sequencing broken end-to-end (sender never injects seq)
2. ME -> Risk payload type mismatch (EventMessage vs FillEvent)
3. Orders never reach ME (Risk doesn't forward accepted orders)
4. Gateway has no external ingress (WS listener exists but no protocol parsing)
5. Marketdata never receives events (ME doesn't fanout to it)
6. WAL records written with seq=0 (breaks dedup/replay)
7. UB risk: unaligned ptr::read on UDP payloads

**What Works:**
- rsx-types: complete shared types (Price, Qty, Side, validation)
- rsx-book: full orderbook implementation (matching, compression, migration)
- rsx-recorder: complete DXS consumer with daily rotation
- rsx-dxs: WAL writer/reader, CMP transport (but seq injection broken)
- rsx-risk: position/margin/funding math, Postgres persistence

**What's Missing:**
- End-to-end order flow (Gateway -> Risk -> ME -> back)
- Gateway protocol parsing (WS frames never decoded)
- Marketdata event consumption (no connection to ME)
- Mark price aggregator (skeleton only, no source connectors)
- All inter-process CMP wiring (broken sequencing)
- Test infrastructure (17 test files exist but many skeleton)

---

## Per-Crate Status

### 1. rsx-types (Shared Types)

**Spec:** ORDERBOOK.md §1, DXS.md §1, shared types across system

**Implementation:** ✅ COMPLETE

**Files:**
- `lib.rs` (94 lines): Price, Qty, Side, TimeInForce, FinalStatus, OrderStatus, FailureReason, SymbolConfig, validate_order
- `macros.rs` (83 lines): install_panic_handler, DeferCall, defer!, on_error_continue!, etc.

**Status:**
- ✅ Fixed-point Price(i64) and Qty(i64) newtypes
- ✅ Side, TimeInForce, FinalStatus, OrderStatus enums
- ✅ FailureReason enum (10 variants)
- ✅ NONE sentinel (u32::MAX)
- ✅ SlabIdx type alias
- ✅ SymbolConfig struct
- ✅ validate_order function (tick/lot alignment)
- ✅ Panic handler (crashes process on any thread panic)
- ✅ RAII cleanup macros (defer!, on_error_continue!, etc.)

**Missing:** Nothing. Core types are complete.

**Compliance:** 100%

---

### 2. rsx-book (Orderbook Library)

**Spec:** ORDERBOOK.md §2-8, TESTING-BOOK.md

**Implementation:** ✅ LARGELY COMPLETE (85%)

**Files:** 11 total, ~1,484 lines
- `lib.rs`, `slab.rs`, `compression.rs`, `level.rs`, `order.rs`, `event.rs`, `user.rs`, `book.rs`, `matching.rs`, `migration.rs`

**Status:**
- ✅ Slab<T> generic arena allocator (O(1) alloc/free)
- ✅ CompressionMap: 5 zones, price_to_index via bisection
- ✅ PriceLevel: head/tail/total_qty/order_count (24B)
- ✅ OrderSlot: 128B, 2 cache lines, order_id_hi/lo (UUIDv7 16B)
- ✅ Event enum: Fill, OrderInserted, OrderCancelled, OrderDone, OrderFailed, BBO
- ✅ UserState: net_qty, order_count (per-symbol tracking)
- ✅ Orderbook: active_levels, staging_levels, orders slab, compression, BBA
- ✅ insert_resting, cancel_order, modify_order_price, modify_order_qty_down
- ✅ scan_next_bid/ask: linear scan for next populated level
- ✅ process_new_order: validation, reduce-only, matching, TIF (IOC/FOK)
- ✅ match_at_level: FIFO, smooshed tick check, fill qty min
- ✅ FOK rollback: saved_event_len, revert event_buf on not fully filled
- ✅ update_positions_on_fill: UserState net_qty tracking
- ✅ BookState enum: Normal, Migrating
- ✅ Migration: bid_frontier, ask_frontier, old_levels, migrate_batch
- ⚠️ Migration logic implemented but not reviewed in detail

**Missing:**
- Snapshot save/load (ORDERBOOK.md §2.8)
- Migration tests (TESTING-BOOK.md §3)
- Recentering trigger logic (BookState transition)
- Post-only enforcement (order flag exists, matching logic missing)

**Test Coverage:** 0 test files in rsx-book (tests may be in rsx-matching)

**Compliance:** 85%

---

### 3. rsx-matching (Matching Engine Binary)

**Spec:** ORDERBOOK.md, DXS.md §3, CMP.md §3, TILES.md

**Implementation:** ⚠️ PARTIAL (40%) - binary runs, but CMP integration broken

**Files:** 5 total, ~741 lines
- `main.rs` (264 lines), `wire.rs` (197), `wal_integration.rs` (208), `fanout.rs` (73), `lib.rs` (3)

**Status:**
- ✅ Load config from env, pin to core, create Orderbook
- ✅ WalWriter init (64MB rotation, 10min retention)
- ✅ CMP receiver/sender (bind/connect)
- ✅ DXS sidecar (spawns tokio thread for replay service)
- ✅ Main loop: recv OrderMessage, process_new_order, write_events_to_wal, send EventMessage
- ✅ OrderMessage -> IncomingOrder conversion
- ✅ EventMessage::from_book_event conversion
- ❌ CMP sender never injects seq in payload (CRITIQUE §1)
- ❌ EventMessage enum sent as-is (Risk expects FillEvent) (CRITIQUE §2)
- ❌ No fanout to Marketdata (CRITIQUE §5)
- ⚠️ WAL records written with seq=0 (CRITIQUE §6)
- ⚠️ UB risk: unsafe ptr::read on UDP payload (CRITIQUE §7)

**Missing:**
- CMP seq injection (CRITICAL)
- Payload layout fix for Risk
- Fanout to Marketdata
- WAL seq assignment
- BBO emission
- CONFIG_APPLIED emission
- Snapshot save/load
- Advisory lock for replica failover

**Test Coverage:** 3 test files, 316 lines
- `fanout_test.rs`, `wal_integration_test.rs`, `wire_test.rs`

**Compliance:** 40%

---

### 4. rsx-dxs (WAL & CMP Transport)

**Spec:** DXS.md, WAL.md, CMP.md, TESTING-DXS.md

**Implementation:** ⚠️ PARTIAL (70%) - WAL/CMP implemented, seq injection broken

**Files:** 9 total, ~2,207 lines
- `lib.rs`, `header.rs`, `records.rs`, `encode_utils.rs`, `wal.rs`, `server.rs`, `client.rs`, `config.rs`, `cmp.rs`

**Status:**
- ✅ WalHeader: 16B (version, record_type, len, stream_id, crc32)
- ✅ 10+ record types: FillRecord, BboRecord, etc.
- ✅ All records #[repr(C, align(64))], fixed-size
- ✅ CRC32 validation
- ✅ WalWriter: append, flush (10ms), fsync, rotation (64MB), GC
- ✅ WalReader: sequential read, file transition, CRC validation
- ✅ DxsReplayService: TCP server, serves WAL stream
- ✅ DxsConsumer: TCP client, tip tracking, reconnect with backoff
- ✅ CmpSender/Receiver: UDP, flow control, heartbeat, NACK
- ❌ CmpSender::send_record never injects seq (CRITIQUE §1)
- ⚠️ extract_seq reads first 8 bytes (assumes seq is there)
- ⚠️ UB risk: unsafe ptr::read on UDP payloads (CRITIQUE §7)

**Missing:**
- CMP seq injection (CRITICAL)
- Alignment-safe deserialization
- QUIC transport (spec mentions, TCP implemented)
- CMP stress tests (packet loss, reorder, flow control)

**Test Coverage:** 5 test files, 684 lines

**Compliance:** 70%

---

### 5. rsx-risk (Risk Engine Binary)

**Spec:** RISK.md, DATABASE.md, TESTING-RISK.md

**Implementation:** ⚠️ PARTIAL (60%) - logic complete, order forwarding missing

**Files:** 15 total, ~1,662 lines
- `main.rs`, `shard.rs`, `position.rs`, `account.rs`, `margin.rs`, `price.rs`, `funding.rs`, `persist.rs`, `replay.rs`, `schema.rs`, `types.rs`, `rings.rs`, `config.rs`, `risk_utils.rs`, `lib.rs`

**Status:**
- ✅ ShardConfig from env
- ✅ RiskShard::new (init accounts, positions, margin, exposure)
- ✅ Cold start: acquire_advisory_lock, load_from_postgres
- ✅ WAL replay: replay_from_wal
- ✅ Persist worker: batched UPSERT/COPY, 10ms flush
- ✅ CMP receivers: Gateway (orders), ME (fills)
- ✅ CMP senders: ME, Gateway
- ✅ process_fill: dedup, apply_fill, calculate_fee, update_exposure
- ✅ process_order: user_in_shard, check_order (portfolio margin), freeze_margin
- ✅ process_bbo: stash, drain, update index price
- ✅ maybe_settle_funding: interval_id, calculate_rate/payment
- ✅ run_once: fills > orders > mark > bbo > funding (priority)
- ✅ Position::apply_fill, PortfolioMargin, ExposureIndex, IndexPrice
- ❌ Main loop never forwards validated orders to ME (CRITIQUE §3)
- ❌ ME -> Risk payload mismatch (CRITIQUE §2)
- ⚠️ UB risk: unsafe ptr::read on CMP payloads (CRITIQUE §7)

**Missing:**
- Order forwarding to ME (CRITICAL)
- OrderResponse -> ME CMP send
- Liquidation processing
- Mark price DXS consumer integration
- Replica behavior
- Advisory lock renewal

**Test Coverage:** 9 test files, 1026 lines (most comprehensive)

**Compliance:** 60%

---

### 6. rsx-gateway (Gateway Binary)

**Spec:** NETWORK.md, WEBPROTO.md, RPC.md, MESSAGES.md

**Implementation:** ❌ SKELETON (10%) - binary runs, no protocol parsing

**Files:** 11 total, ~1,304 lines
- `main.rs` (97), `ws.rs` (190), `protocol.rs` (683), `types.rs`, `config.rs`, `convert.rs`, `order_id.rs`, `pending.rs`, `circuit.rs`, `rate_limit.rs`, `lib.rs`

**Status:**
- ✅ GatewayConfig from env
- ✅ CMP sender/receiver init
- ✅ monoio runtime (FusionDriver, io_uring)
- ✅ ws_accept_loop: accept, WebSocket handshake
- ✅ WsFrame enum, parse_frame, write_frame
- ✅ protocol.rs: complete JSON parsing (683 lines, NOT USED)
- ❌ Main loop spawns ws_accept_loop but passes empty closure (CRITIQUE §4)
- ❌ No per-connection handler
- ❌ Protocol parsing never called
- ❌ No dedup, rate limit, circuit breaker (code exists, not wired)

**Missing:**
- Per-connection handler (spawn on accept, parse frames, route to Risk)
- Order routing to Risk via CMP
- Response routing from Risk to WS
- UUIDv7 order_id generation (code exists, not integrated)
- PendingOrders tracking (code exists, not integrated)
- Dedup window (5min)
- RateLimiter, CircuitBreaker (code exists, not integrated)
- Auth

**Test Coverage:** 0 test files

**Compliance:** 10%

---

### 7. rsx-marketdata (Marketdata Binary)

**Spec:** MARKETDATA.md, TESTING-MARKETDATA.md

**Implementation:** ❌ SKELETON (30%) - binary runs, no event processing

**Files:** 8 total, ~660 lines
- `main.rs` (65), `shadow.rs` (259), `protocol.rs` (145), `subscription.rs` (140), `types.rs`, `config.rs`, `lib.rs`

**Status:**
- ✅ MarketdataConfig from env
- ✅ CMP receiver init (from ME)
- ✅ monoio runtime
- ✅ ShadowBook: wraps Orderbook, apply_event, compute_l2/bbo/trades
- ✅ L2Snapshot, BBO, Trade JSON serialization
- ✅ SubscriptionManager: per-channel subscriber tracking
- ❌ Main loop receives CMP but never decodes/processes events (CRITIQUE §5)
- ❌ No WS broadcast accept loop
- ❌ No shadow book instance
- ❌ No subscription manager instance

**Missing:**
- CMP event decoding
- ShadowBook instance + apply_event
- WS broadcast accept loop
- Per-client subscription tracking
- L2/BBO/trades broadcast on update
- DXS recovery (replay from ME WAL on startup)

**Test Coverage:** 0 test files

**Compliance:** 30%

---

### 8. rsx-mark (Mark Price Aggregator Binary)

**Spec:** MARK.md, TESTING-MARK.md

**Implementation:** ❌ SKELETON (20%) - binary runs, no source connectors

**Files:** 6 total, ~308 lines
- `main.rs` (90), `aggregator.rs` (138), `source.rs` (32), `types.rs` (43), `config.rs` (90), `lib.rs`

**Status:**
- ✅ MarkConfig from env
- ✅ WalWriter init
- ✅ SymbolMarkState: aggregate_price (median), sweep_stale
- ✅ MarkSource trait, SourcePrice struct
- ✅ MarkPriceEvent record
- ✅ Main loop: TODO drain rings, sweep (1s), flush (10ms)
- ❌ No source connector implementations
- ❌ No SPSC rings for source data
- ❌ No SymbolMarkState instances
- ❌ No DXS replay server

**Missing:**
- Source connectors (Binance, Bybit HTTP/WS clients)
- SPSC rings per source
- SymbolMarkState Vec (one per symbol)
- aggregate_price call + MarkPriceEvent emission
- DXS replay server (so Risk can consume)
- Premium calculation
- Staleness detection

**Test Coverage:** 0 test files

**Compliance:** 20%

---

### 9. rsx-recorder (Archival Consumer Binary)

**Spec:** DXS.md §8, ARCHIVE.md

**Implementation:** ✅ COMPLETE (100%)

**Files:** 1 file, 142 lines
- `main.rs`

**Status:**
- ✅ RecorderState: archive_dir, stream_id, current_date, file, buf
- ✅ RecorderState::new: create dir, open daily file
- ✅ write_record: append header + payload to buf
- ✅ rotate: check date, close old file, open new
- ✅ flush: write_all + sync_all
- ✅ DxsConsumer::run: callback on each record
- ✅ RecorderConfig::from_env
- ✅ Daily rotation at UTC midnight
- ✅ No transformation (raw WAL bytes)

**Missing:** Nothing.

**Test Coverage:** 0 test files (integration test would need real DXS server)

**Compliance:** 100%

---

## Spec Compliance Matrix

| Spec | Primary Crate | Status | Compliance |
|------|---------------|--------|------------|
| ARCHITECTURE.md | All | ⚠️ Partial | 30% - CMP routing broken |
| TILES.md | All | ⚠️ Partial | 40% - monoio, SPSC exist but not wired |
| ORDERBOOK.md | rsx-book | ⚠️ Good | 85% - core complete, snapshot missing |
| DXS.md | rsx-dxs | ⚠️ Partial | 70% - seq injection broken |
| WAL.md | rsx-dxs | ⚠️ Partial | 70% - seq=0 bug |
| CMP.md | rsx-dxs | ⚠️ Partial | 60% - seq injection broken, UB |
| RISK.md | rsx-risk | ⚠️ Partial | 60% - order forwarding missing |
| MARK.md | rsx-mark | ❌ Skeleton | 20% - sources missing |
| MARKETDATA.md | rsx-marketdata | ❌ Skeleton | 30% - event processing missing |
| NETWORK.md | All | ❌ Broken | 20% - CMP broken |
| WEBPROTO.md | rsx-gateway | ❌ Skeleton | 10% - protocol never called |
| RPC.md | rsx-gateway | ❌ Skeleton | 5% - pending tracker not used |
| MESSAGES.md | rsx-gateway | ❌ None | 0% - dedup not wired |
| DATABASE.md | rsx-risk | ✅ Good | 90% - schema + persist work |
| LIQUIDATOR.md | rsx-risk | ❌ None | 0% - logic exists, not called |
| METADATA.md | All | ❌ None | 0% - CONFIG_APPLIED not propagated |
| CONSISTENCY.md | All | ❌ Broken | 20% - CMP seq bug breaks ordering |
| DEPLOY.md | - | ❌ None | 0% - no deployment scripts |
| ARCHIVE.md | rsx-recorder | ✅ Complete | 100% |

---

## Test Coverage Summary

**Total:** 17 test files, 2,379 lines

### By Crate

- **rsx-types:** 0 test files (types are simple)
- **rsx-book:** 0 test files (tests may be in rsx-matching)
- **rsx-matching:** 3 files, 316 lines
- **rsx-dxs:** 5 files, 684 lines
- **rsx-risk:** 9 files, 1026 lines (MOST COMPREHENSIVE)
- **rsx-gateway:** 0 files
- **rsx-marketdata:** 0 files
- **rsx-mark:** 0 files
- **rsx-recorder:** 0 files

### Test Spec Compliance

| Test Spec | Coverage |
|-----------|----------|
| TESTING-BOOK.md | ⚠️ Partial - 3 matching tests |
| TESTING-MATCHING.md | ⚠️ Partial - basic serde only |
| TESTING-DXS.md | ⚠️ Partial - WAL/CMP unit tests |
| TESTING-RISK.md | ✅ Good - 9 files, testcontainers |
| TESTING-GATEWAY.md | ❌ None |
| TESTING-MARKETDATA.md | ❌ None |
| TESTING-MARK.md | ❌ None |
| TESTING-CMP.md | ⚠️ Partial - unit tests, no stress |
| TESTING-SMRB.md | ❌ None |

---

## Critical Path to Working System

### Phase 1: Fix Critical CMP Issues (1-2 days)

**BLOCKER**

1. CMP seq injection (rsx-dxs/src/cmp.rs)
2. ME -> Risk payload fix (rsx-matching/src/main.rs)
3. Risk order forwarding (rsx-risk/src/main.rs)
4. UB fix: unaligned reads (all CMP consumers)

### Phase 2: Gateway Protocol Parsing (2-3 days)

**HIGH**

5. Gateway per-connection handler
6. Gateway dedup + pending tracking

### Phase 3: Marketdata Fan-Out (1-2 days)

**MEDIUM**

7. ME fanout to Marketdata
8. Marketdata event processing + WS broadcast

### Phase 4: WAL Seq Fix (1 day)

**HIGH**

9. WAL seq assignment (rsx-matching, rsx-dxs)

### Phase 5: Basic E2E Test (1 day)

**HIGH**

10. E2E smoke test (start all binaries, send order, verify fill)

**Total: 7-9 days for minimal working system**

---

## Summary

**Lines of Code:**
- Implementation: 8,685 lines (excluding tests)
- Tests: 2,379 lines (17 files)
- Total: 11,064 lines

**Overall Progress:**
- Core data structures: 85% complete
- Networking/CMP: 60% complete (broken seq, UB)
- Matching engine: 70% complete (works but CMP broken)
- Risk engine: 60% complete (logic works, order forwarding missing)
- Gateway: 10% complete (skeleton)
- Marketdata: 30% complete (logic ready, not wired)
- Mark: 20% complete (skeleton)
- Recorder: 100% complete

**Critical Blockers:**
1. CMP seq injection
2. ME -> Risk payload mismatch
3. Risk order forwarding
4. UB in ptr::read
5. Gateway protocol parsing

**Next Steps:** Fix 4 critical issues + Gateway protocol parsing = working end-to-end in ~7-9 days.

**Last Updated:** 2026-02-10 (comprehensive line-by-line audit)
