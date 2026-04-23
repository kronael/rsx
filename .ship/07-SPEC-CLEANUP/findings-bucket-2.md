# Bucket 2 findings (specs 13-24)

## 13-liquidator.md

- §1 State — **bloat** (full Rust struct defs `LiquidationState`/`LiqStatus` verbatim; match the code in `rsx-risk/src/liquidation.rs:1-18` but belong there, not here)
- §1 Implementation note — **match** (`active: Vec<LiquidationState>` per (user_id,symbol_id) confirmed in code)
- §2 Backoff Schedule — **drift** (spec says table+`max_slip_bps` cap; code has no `max_slip_bps` field in `LiquidationEngine` or `LiquidationConfig`; cap is absent from implementation)
- §3 Order Generation — **match** (fallback chain, reduce_only, is_liquidation flags confirmed in `shard.rs:442`)
- §4 Lifecycle — **bloat** (pseudocode duplicates actual `shard.rs` logic; readable in code)
- §5 Margin Recovery — **match** (equity check on fill / round escalation logic present in `shard.rs:309-417`)
- §6 Frozen Margin Interaction — **match**
- §7 Main Loop Integration — **drift** (spec says liquidation goes between funding check and lease renewal; code runs liquidation at step 1b — ordering differs from spec)
- §8 Persistence — **match** (`liquidation_events` table in `rsx-risk/migrations/001_base_schema.sql:73-86`)
- §9 Config — **drift** (spec lists `RSX_LIQUIDATION_MAX_SLIP_BPS=500`; unimplemented in `LiquidationConfig`)
- §10 Edge Cases — **bloat** (extensive per-edge-case analysis; belongs in TESTING-LIQUIDATOR)
- §11 Performance Targets — **bloat** (micro-benchmark table without gating)
- §12 File Organization — **match**
- §13 Tests — **match**

**Status recommendation**: partial
**Notable action items**:
- Add `max_slip_bps` to `LiquidationConfig` and enforce cap; the §9 config env var is advertised but not wired
- Strip §10 edge case wall-of-text and §11 perf table
- Reconcile §7 main-loop ordering with actual shard tick sequence

## 14-management-dashboard.md

- §Intro — **match** (playground + webui exist)
- §Shared requirements — **drift** (playground API routes are `/api/*` not `/v1/api/play/*`; `audit_log` is local stdout, not DB; no feature flags wired)
- §Platform Decisions — **partial match** (Vite confirmed; shadcn/ui unverified)

**Status recommendation**: partial
**Notable action items**:
- Align shared-requirements claims to code: `audit_log` is stdout-only; API base path is `/api/` not `/v1/api/play`
- Verify shadcn/ui vs plain Tailwind in rsx-webui

## 15-mark.md

- §1 Architecture — **match**
- §2 Data Structures — **bloat** (`MarkPriceRecord`/`SymbolMarkState` struct defs verbatim; code at `rsx-mark/src/types.rs:3-35`)
- §3 Source Connectors — **match**
- §4 Aggregation Logic — **bloat** (pseudocode duplicates `rsx-mark/src/aggregator.rs`)
- §5 Serving Subscribers — **match**
- §6 Main Loop — **bloat** (pseudocode duplicates `main.rs`)
- §7 Config — **match** (all env vars confirmed in `config.rs`)
- §8 RISK.md Changes — **match** (`binance.rs` absent from `rsx-risk/src/`; risk receives mark via CMP)
- §9 Performance Targets — **bloat** (no criterion gating)
- §10 File Organization — **match**

**Status recommendation**: shipped
**Notable action items**:
- Strip §2, §4, §6 struct/pseudocode bloat
- §9 perf targets should be backed by bench gates

## 16-marketdata.md

- §Inputs/Outputs — **match**
- §Subscribe Channels — **match** (`CHANNEL_BBO=1`, `CHANNEL_DEPTH=2`, `CHANNEL_TRADES=4` in `rsx-marketdata/src/subscription.rs:4-6`)
- §Notes — **match**

**Status recommendation**: shipped
**Notable action items**: None; spec is lean and accurate.

## 17-matching.md

- §Responsibilities — **match**
- §Inputs/Outputs — **match**
- §Determinism — **match**
- §Config — **match** (env-only symbol_id/tick/lot; 10-min Postgres poll)

**Status recommendation**: shipped
**Notable action items**: None; spec is terse and correct.

## 18-messages.md

- §Order States — **match**
- §Message Schema — **match** (`#[repr(C)]` fixed records over CMP/UDP; types in `rsx-dxs`)
- §Message Flow Sequences — **bloat** (step-by-step worked examples; these are integration test scenarios)
- §Fill Streaming Details — **match**
- §Completion Signals — **match**
- §Idempotency & Deduplication — **bloat** (full Rust pseudocode; actual in `rsx-matching/src/dedup.rs`). Drift: dedup key is `(user_id, order_id_hi, order_id_lo)` not just `order_id`
- §Idempotency — **match** (`DEDUP_WINDOW=300s`, VecDeque pruning)
- §Risk Integration — **bloat** (reimplements RISK.md margin logic inline)
- §Alignment with Existing Architecture — **bloat** (repeats ORDERBOOK.md)
- §Cross-References — **match**

**Status recommendation**: partial
**Notable action items**:
- Remove §Message Flow Sequences worked examples, §Risk Integration pseudocode, §Alignment section
- Fix dedup key description: code keys on `(user_id, order_id_hi, order_id_lo)`

## 19-metadata.md

- §Goals — **match**
- §Data Model §1 `symbol_static` — **match** (`rsx-matching/migrations/001_symbol_config.sql:13-17`)
- §Data Model §2 `symbol_config_schedule` — **match**
- §Application Semantics (10-min poll, effective_at_ms, CONFIG_APPLIED event) — **match** (`rsx-matching/src/config.rs:40-45`, `main.rs:336-343`)
- §Propagation / cold start (`symbol_config_applied`) — **match**
- §Notes — **match**

**Status recommendation**: shipped
**Notable action items**: None; spec is accurate and lean.

## 20-network.md

- §Overview/topology — **match**
- §Component Architecture / Gateway (monoio, rate limit, 10k cap) — **match** (monoio in `rsx-gateway/src/main.rs:126`; 10k cap in `pending.rs:20`)
- §Component Architecture / Risk Engine — **match**
- §Component Architecture / Matching Engine — **drift** (spec says "UUIDv7 tracking" but code uses `(user_id, order_id_hi, order_id_lo)` triple; also "Stateless regarding users" but ME does track `UserState` for reduce-only)
- §Scaling Strategy — **match**
- §Communication Topology — **match**
- §Data Flow / Order Submission Flow — **bloat** (duplicates MESSAGES.md)
- §Data Flow / Fill Notification — **drift** (diagram shows fills going to Gateway directly; actual path is ME→Risk→Gateway)
- §Data Flow / Risk Update Flow — **match**
- §Network Boundaries — **match**
- §Performance Characteristics — **bloat** (latency numbers without measurement backing)
- §Deployment Topologies — **match**
- §Service Discovery (env vars) — **partial** (listed env vars like `RSX_ME_BTC_ADDR`, `RSX_GATEWAY_ADDR` not found in code)
- §Startup Ordering — **match**
- §Failure Modes — **match**
- §MARKETDATA section — **match**

**Status recommendation**: partial
**Notable action items**:
- Fix §Fill Notification Flow: fills go ME→Risk→Gateway
- Fix §Matching Engine stateless claim: ME tracks `UserState`
- Verify/fix service discovery env var names against actual crate configs
- Strip §Order Submission (duplication) and perf numbers

## 21-orderbook.md

- §Design Goals — **match**
- §1 Price & Quantity — **bloat** (struct defs; code in `rsx-types/src/lib.rs:21-30`)
- §2 Tick/Lot — **bloat** (`SymbolConfig` struct + `validate_order` pseudocode)
- §2.5 Compressed Indexing — **bloat** (CompressionMap + `price_to_index` Rust verbatim)
- §2.6 Smooshed Ticks — **bloat** (matching pseudocode)
- §2.7 Copy-on-Write Recentering — **bloat** (migration structs and algorithms)
- §2.8 Durability — **match**
- §2.9 Symbol Config Distribution — **match**
- §3 Orderbook Data Structure — **drift** (spec shows `sequence: u16`; code has `u32`; spec also shows `_pad4: [u8; 40]` but code has `[u8; 24]` plus `order_id_hi/lo` in slot; spec says those "live at gateway layer" — wrong)
- §4 Operation Complexity — **match**
- §5 Matching Algorithm — **bloat** (pseudocode with FOK/IOC)
- §6 Event Types — **bloat** (Event enum verbatim). Drift: Fill has `maker_order_id_hi/lo` and `taker_order_id_hi/lo` not handles
- §6.5 User Position Tracking — **bloat** (`UserState` + `get_or_assign_user` pseudocode)
- §7 Memory Layout & Performance — **bloat** (sizing tables)
- §8 Why This Design — **match**

**Status recommendation**: partial
**Notable action items**:
- Fix §3 OrderSlot drift: `sequence` is `u32`; `order_id_hi/lo` ARE in slot
- Strip §1, §2, §2.5-2.7, §5, §6, §6.5, §7 (all duplicated in code)

## 22-perf-verification.md

- §Status quo — **match**
- §Deliverable 1: Criterion CI gate — **match** (`scripts/bench-gate.sh` + `make bench-gate`/`bench-save`)
- §Deliverable 2: Playground latency pipeline — **partial** (endpoints exist; `play_latency.spec.ts` still has `test.skip()` at lines 245, 298, 335 — vacuous-assertion fix partially unshipped)
- §Deliverable 3: Gateway mode endpoint — **match** (`/api/gateway-mode` + `/x/gateway-mode` + test in `api_e2e_test.py:350-356`)
- §Files — **match**
- §Verification — **match**

**Status recommendation**: partial
**Notable action items**:
- Remove remaining `test.skip()` blocks from `play_latency.spec.ts` lines 245, 298, 335

## 23-playground-dashboard.md

- §1 Purpose — **match**
- §2 Scope / Mode flags — **drift** (`PLAYGROUND_WRITES_ENABLED` doesn't exist; `PLAYGROUND_MODE` only checks `== "production"` to block)
- §3 Capability Model (Observe/Act/Verify) — **match**
- §4 API — **drift** (actual routes are `/api/*` not `/v1/api/play/*`; listed endpoints like `/faults/{kind}/inject` use different paths)
- §5 Safety Rules — **drift** (`audit_log` is stdout print, not DB write with module field)
- §6 Data Sources — **match**
- §7 UI Surfaces — **partial match** (most exist; no dedicated "CMP" screen)
- §8 Auth Model — **match**
- §9 Acceptance — **match**

**Status recommendation**: partial
**Notable action items**:
- Fix §4: update spec API base path to `/api/*` or align code to `/v1/api/play/*`
- Implement or remove `PLAYGROUND_WRITES_ENABLED`; add `module` field to audit_log or retract
- Add CMP flows screen or remove from §7

## 24-position-edge-cases.md

- §1 Position State Transitions — **match** (all four patterns in `rsx-risk/src/position.rs:33-101`)
- §2 Arithmetic Edge Cases — **match** (i128 intermediates in `position.rs`; `unrealized_pnl()` in `margin.rs:49`)
- §2.2 Division by Zero / Index price — **drift** (spec shows formula `(bid * ask_qty + ask * bid_qty) / (bid_qty + ask_qty)`; code has this in `rsx-risk/src/price.rs` but described as risk's BBO-derived index)
- §3 Multi-User Interactions — **match**
- §4 Crash and Recovery — **match** (`rsx-risk/src/replay.rs`)
- §5 Liquidation Edge Cases — **match**
- §6 Price Feed Edge Cases — **match**
- §7 Fee and Collateral — **match**
- §8 Concurrency/Ordering — **match**
- §9 Symbol Config — **match**
- §10 Replay and Reconciliation — **bloat** (reconciliation SQL query and fill-gap scenarios)
- §11 Network and Partition — **match**
- §12 Summary of Critical Invariants — **match**
- §13 References — **match**

**Status recommendation**: reference
**Notable action items**:
- Move §10 SQL queries into test fixtures or GUARANTEES.md
- Consider demoting to `tests/TESTING-RISK.md` or reference appendix
- No code gaps; all edge cases implemented
