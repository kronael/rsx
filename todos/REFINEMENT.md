# Refinement Backlog
date: 2026-02-19

Items consolidated from TODO-CRITICAL, TODO-HIGH, TODO-MEDIUM, TODO-LOW,
TODO-SPEC-TESTS, TODO-FUTURE, DEFICIENCIES, FIX, LEFTOSPEC, and TASKS.
Skips: TODO-DONE items, items completed in PROGRESS.md.
One line per item, max 120 chars.

---

## Arithmetic overflow audit — DONE 2026-02-22

All i128→i64 casts audited and fixed. Saturating clamp applied to:
- `funding.rs`: premium and payment casts
- `position.rs`: 8 entry-cost and PnL casts in `apply_fill`
- `price.rs`: weighted mid-price cast
Already safe: `liquidation.rs` (checked_mul + min(9999)), `book.rs`
(bounds-checked emit), `margin.rs`/`risk_utils.rs` (try_from/unwrap_or),
`source.rs` parse_price (checked_mul/add).

---

## P0 — Correctness Bugs (must fix before deployment)

### [gateway] Circuit breaker HalfOpen blocks all requests forever; circuit can never recover
- File: rsx-gateway/src/circuit.rs:45
- Fix: `State::HalfOpen` must allow one test request through, not reject all

### [mark] parse_price unchecked overflow corrupts mark prices published to WAL
- File: rsx-mark/src/source.rs:221
- Fix: use checked_mul/checked_add, return None on overflow

### [book] Snapshot load panics on out-of-bounds slab index (no bounds check before indexing)
- File: rsx-book/src/snapshot.rs:198
- Fix: validate idx < capacity before slab access, return Err on bad index

### [book] Snapshot load panics on out-of-bounds level index
- File: rsx-book/src/snapshot.rs:224
- Fix: validate idx < active_levels.len() before indexing

### [book] Snapshot load panics on out-of-bounds user state index
- File: rsx-book/src/snapshot.rs:245
- Fix: validate idx < user_states.len() before indexing

### [playground] XSS: render_logs() uses raw log line instead of escaped_line variable
- File: rsx-playground/pages.py:~1507
- Fix: replace `{line}` with `{html.escape(line)}`

### [playground] XSS: render_risk_user() renders unescaped dict keys/values from DB
- File: rsx-playground/pages.py:~1743
- Fix: wrap key and val with html.escape(str(...))

### [playground] XSS: render_error_agg() renders unescaped error pattern strings
- File: rsx-playground/pages.py:~1532
- Fix: html.escape(pattern) before rendering

### [playground] XSS: render_verify() renders unescaped check detail strings
- File: rsx-playground/pages.py:~1562
- Fix: html.escape(str(c["detail"])) before rendering

### [playground] JSONResponse type mismatch crashes stress reports table (AttributeError on .body)
- File: rsx-playground/server.py:1467-1509
- Fix: return raw list from api_stress_reports(), not JSONResponse

### [playground] Missing subprocess import in conftest.py causes NameError on test cleanup
- File: rsx-playground/tests/conftest.py:47
- Fix: add `import subprocess` at top of file

### [playground] No production environment guard; destructive ops accessible without safeguard
- File: rsx-playground/server.py (missing)
- Fix: check PLAYGROUND_MODE env var at startup, refuse if mode is 'production'

### [playground] Duplicate client fixture in api_e2e_test.py shadows conftest.py fixture
- File: rsx-playground/tests/api_e2e_test.py:11-14
- Fix: delete the duplicate fixture definition, use conftest.py version

---

## P0 — Correctness Bugs (high severity, fix before production)

### [gateway] Rate limiter double-counts tokens: advance_time_by() never updates last_refill
- File: rsx-gateway/src/rate_limit.rs:44-48
- Fix: add `self.last_refill = Instant::now()` in advance_time_by

### [dxs] WAL rotation: no sync_all() before file rename; crash loses data
- File: rsx-dxs/src/wal.rs:182-187
- Fix: call file.sync_all()? before drop and rename

### [dxs] NAK retransmit fails silently when WalReader::open_from_seq() returns error
- File: rsx-dxs/src/cmp.rs:151-184
- Fix: return error or re-queue NAK for retry with backoff

### [risk] Liquidation slip calculation overflows i64; (10_000 - slip) goes negative
- File: rsx-risk/src/liquidation.rs:166-173
- Fix: use checked_mul chain, cap slip at 9_999

### [risk] Funding premium overflow: (mark - index) * 10_000 overflows i64
- File: rsx-risk/src/funding.rs:26-27
- Fix: use i128 intermediate for multiplication

### [book] Event buffer overflow: event_len incremented without checking MAX_EVENTS
- File: rsx-book/src/book.rs:88-89
- Fix: guard emit() with `if self.event_len >= MAX_EVENTS { return; }`

### [playground] Order form order_type field silently ignored; all orders default to LIMIT
- File: rsx-playground/server.py:1239-1253
- Fix: add `"order_type": form.get("order_type", "limit")` to order_msg dict

### [playground] stress_client.py exception swallowed; prevents task cancellation and cleanup
- File: rsx-playground/stress_client.py:149
- Fix: add `raise` after the print statement in except block

### [playground] Concurrent cancel test: unsafe access to server.recent_orders (no length guard)
- File: rsx-playground/tests/api_edge_cases_test.py:809
- Fix: add length guard and .get() for key access

### [playground] Concurrent cancel test: no assertions after operations; only checks no-crash
- File: rsx-playground/tests/api_edge_cases_test.py:803-818
- Fix: assert all responses are 200, verify cancelled status

### [playground] Scenario state set before build/spawn in start_all(); wrong state on build fail
- File: rsx-playground/server.py:241-242
- Fix: move `current_scenario = scenario` after successful spawn

### [playground] Audit logging missing: no trail for destructive ops (kill, reset, liquidate)
- File: rsx-playground/server.py (missing)
- Fix: add audit_log() writing JSONL to log/audit.log, call from all POST endpoints

---

## P1 — Test Gaps (spec-specified tests not implemented)

### Gateway tests (TESTING-GATEWAY.md)

### [gateway] [test] heartbeat_sent_every_5s — no timer test exists
### [gateway] [test] heartbeat_timeout_closes_at_10s — no timer test exists
### [gateway] [test] heartbeat_client_response_resets_timer — no handler test
### [gateway] [test] symbol_not_found_rejects_early — needs config cache
### [gateway] [test] config_cache_updated_on_config_applied — needs CONFIG_APPLIED event
### [gateway] [test] ws_new_order_accepted_and_filled — E2E test missing
### [gateway] [test] concurrent_sessions_isolated — E2E test missing
### [gateway] [test] fills_precede_order_done_on_wire — invariant 1 not verified in test
### [gateway] [test] liquidation_order_routed_correctly — E2E test missing
### [gateway] [test] circuit_breaker_opens_on_gateway_overload — E2E test missing
### [gateway] [test] rate_limit_per_user_enforced_e2e — E2E test missing

### Risk Engine tests (TESTING-RISK.md)

### [risk] [test] order_while_user_liquidated_rejected — integration test missing
### [risk] [test] config_applied_event_updates_params — integration test missing
### [risk] [test] config_applied_forwarded_to_gateway — integration test missing
### [risk] [test] main_lease_acquired_at_startup — replication test missing
### [risk] [test] replica_promoted_on_main_failure — replication test missing
### [risk] [test] fill_buffering_during_promotion — replication test missing
### [risk] [test] crash_recovery_replays_from_tip — replication test missing
### [risk] [test] full_lifecycle_order_to_settlement — full system test missing
### [risk] [test] liquidation_cascade_multiple_users — full system test missing
### [risk] [test] me_failover_dedup_preserved — full system test missing
### [risk] [test] funding_settlement_all_intervals — full system test missing

### Liquidator tests (TESTING-LIQUIDATOR.md)

### [liquidator] [test] liquidate_largest_position_first — multi-position test missing
### [liquidator] [test] partial_liquidation_reduces_to_target — test missing
### [liquidator] [test] multiple_symbols_liquidated_independently — test missing
### [liquidator] [test] new_orders_rejected_during_liquidation — integration test missing
### [liquidator] [test] price_drops_triggers_liquidation — E2E test missing
### [liquidator] [test] cascade_liquidation_across_users — E2E test missing
### [liquidator] [test] liquidation_persisted_to_postgres — E2E test missing
### [liquidator] [test] recovery_resumes_pending_liquidations — E2E test missing
### [liquidator] [test] order_failed_retries_with_slip — E2E test missing
### [liquidator] [test] insurance_fund_absorbs_deficit — E2E test missing
### [liquidator] [test] symbol_halt_on_repeated_failure — E2E test missing

### Playground test gaps

### [playground] [test] Process cleanup fixture does not wait with timeout after kill
- File: rsx-playground/tests/conftest.py:38-56

### [playground] [test] Fixture cleanup scope not explicit (missing scope="function")
- File: rsx-playground/tests/conftest.py:24-56

### [playground] [test] Percentile assertions too loose in stress integration test
- File: rsx-playground/tests/stress_integration_test.py:237-239

### [playground] [test] Missing @pytest.mark.asyncio on async test function
- File: rsx-playground/tests/stress_integration_test.py:215

### [playground] [test] Path traversal assertion checks wrong status code (too loose)
- File: rsx-playground/tests/api_edge_cases_test.py:381

---

## P2 — Hardening and Quality

### [gateway] JWT missing aud/iss validation; tokens from other services sharing secret accepted
- File: rsx-gateway/src/jwt.rs:14-35
- Fix: add set_audience(["rsx-gateway"]) and set_issuer(["rsx-auth"]) to validation

### [dxs] WAL file exceeds max_file_size by one record: size check happens after write
- File: rsx-dxs/src/wal.rs:144-168
- Fix: check `file_size + buf.len() >= max_file_size` BEFORE write

### [dxs] Tip persistence lacks directory fsync; rename metadata can be lost on crash
- File: rsx-dxs/src/client.rs:460-465
- Fix: open parent dir and sync_all() after rename

### [dxs] Reorder buffer silently drops gap-fill records when full; NAK loops forever
- File: rsx-dxs/src/cmp.rs:426-443
- Fix: grow buffer or implement sliding window with eviction

### [dxs] Tip advances from records with no seq via saturating_add(1); tip drifts
- File: rsx-dxs/src/client.rs:307-312
- Fix: skip tip update when extract_seq() returns None

### [risk] Notional overflow: i128 result cast to i64 without check; extreme positions truncate
- File: rsx-risk/src/position.rs:100-103
- Fix: use i64::try_from() or cap at i64::MAX

### [risk] Average entry price divide-by-zero if long_qty == 0 (latent invariant break)
- File: rsx-risk/src/position.rs:105-114
- Fix: add debug_assert!(self.long_qty > 0) or return 0

### [mark] scale_digits assumes power-of-10 scale; breaks with non-power scales (e.g. 500_000)
- File: rsx-mark/src/source.rs:207
- Fix: validate scale at config time or use (scale as f64).log10()

### [mark] Fractional price truncation (not rounding); consistent downward bias in mark prices
- File: rsx-mark/src/source.rs:209
- Fix: document as spec decision OR implement rounding

### [mark] Double stale update in mark aggregator: stale flag not checked before aggregate phase
- File: rsx-mark/src/main.rs:~180
- Fix: check stale flag before aggregate phase

### [marketdata] Shadow book seq gap detection skips seq=0; causes phantom gaps for all subsequent events
- File: rsx-marketdata/src/state.rs:209-216
- Fix: remove `|| seq == 0` from early return or handle seq=0 as valid init

### [cli] WAL dump reads arbitrary record lengths; corrupted WAL causes OOM or hang
- File: rsx-cli/src/main.rs:128
- Fix: add `if len > 1_000_000 { break; }` after length parse

### [types] System time panics before 1970; all ts_ns() callers affected
- File: rsx-types/src/time.rs:8,17,26,35
- Fix: use .unwrap_or_default() instead of .unwrap()

### [start] SQL injection in reset_db: db_name interpolated into SQL without quoting
- File: start:441-445
- Fix: `f'DROP DATABASE IF EXISTS "{db_name}"'`

### [start] REPL input parsing crashes on "stop " with trailing space and no process name
- File: start:672,676
- Fix: bounds check `parts[1] if len(parts) > 1 else ""`

### [start] Postgres init race: migration runs before postgres is fully initialized
- File: start:511-541
- Fix: add retry loop or pg_isready check before migration

### [start] Hardcoded port 5432 in start script; no DATABASE_URL or PORT config
- File: start:125-506
- Fix: read from DATABASE_URL or add PORT config var

### [playground] Gateway URL hardcoded to ws://localhost:8080; not configurable
- File: rsx-playground/server.py:1220
- Fix: `GATEWAY_URL = os.environ.get("GATEWAY_URL", "ws://localhost:8080")`

### [playground] render_wal_status() uses direct dict[] access; KeyError on missing keys
- File: rsx-playground/pages.py:~1420
- Fix: replace all s["key"] with s.get("key", "-")

### [playground] render_wal_files() uses direct dict[] access; KeyError on missing keys
- File: rsx-playground/pages.py:~1453
- Fix: same pattern, use f.get("key", "-")

### [playground] render_verify() uses direct dict[] access; KeyError on missing keys
- File: rsx-playground/pages.py:~1569
- Fix: use c.get("name", "unknown")

### [playground] Stress client symbol IDs [0,1,2] mismatch exchange expected [1,2,3,10]
- File: rsx-playground/stress_client.py:72
- Fix: `random.choice([1, 2, 3, 10])`

### [playground] Submitted counter incremented before send; inflated metrics on failed sends
- File: rsx-playground/stress_client.py:137
- Fix: move increment inside try block after successful ws.send()

### [playground] Staging confirmation gate missing: destructive ops need no confirmation
- File: rsx-playground/server.py (missing)
- Fix: add x-confirm-token header check for kill/stop in staging mode

### [playground] Idempotency key validation missing on stress endpoints
- File: rsx-playground/server.py (missing)
- Fix: require x-idempotency-key header, cache result for 1h

### [playground] Tailwind dynamic class strings fragile under JIT (f-string color interpolation)
- File: rsx-playground/pages.py:2060-2079
- Fix: map status names to full class name strings

### [playground] Latency measurement includes ws.send() buffering time; mislabeled as round-trip
- File: rsx-playground/stress_client.py:101-107
- Fix: document as "round-trip including send buffer" OR move start after ws.send()

### [playground] Race condition in latency list append (CPython GIL safe, not language-safe)
- File: rsx-playground/stress_client.py:140-141
- Fix: document CPython GIL assumption or add asyncio.Lock

---

## P3 — Future / Spec Gaps

### [mark] Multi-source mark price aggregation unspecified: only Binance ingestion exists in RISK.md
- Spec gap: need median-of-sources method, staleness threshold, fallback to index price
- Touches: RISK.md, CONSISTENCY.md, MESSAGES.md (telemetry)

### [mark] Binance feed reconnect details missing from RISK.md (~10 lines)
- Missing: backoff (1s,2s,4s,8s,max 30s), staleness 10s, stale behavior

### [dxs] CMP protocol: symbol_id sent per-message; should be per-stream (handshake/setup frame)
- Saves 4 bytes per record; simplifies wire format

### [dxs] CMP pipeline type layers: Book Event → 4 transform steps; should be single WAL record
- Fix: emit WAL record once from book, flow unchanged to CMP/WAL/consumers

### [gateway] monoio single-threaded per core; needs work-stealing runtime for many concurrent WS
- Evaluate tokio-uring or glommio; keep io_uring, add work stealing for connection distribution

### [liquidator] Symbol halt on repeated liquidation failure not implemented
- Source: specs/v1/LIQUIDATOR.md:347
- When liquidation fails repeatedly, halt symbol trading (spec TODO)

### [future] Quantified stress test targets not validated (1M fills/sec ME, 100K fills/sec DXS replay)
- Source: GUARANTEES.md:1073; run actual benchmarks to confirm or revise guarantees

### [future] Multi-datacenter replication guarantees unspecified (cross-DC lag, partition tolerance)
- Source: GUARANTEES.md:1081

### [future] Snapshot frequency vs replay time tradeoff not analyzed
- Source: GUARANTEES.md:1091; more frequent = faster recovery, higher I/O

### [future] WAL retention vs disk usage worst-case not analyzed (10min retention at peak load)
- Source: GUARANTEES.md:1100

### [future] smrb crate (shared memory ring buffer): shm_open/mmap backed SPSC, no_std core
- Source: TASKS.md; needed for cross-process IPC beyond same-process rtrb

### [future] Modify order: v1 deferred (cancel + re-insert); v2 atomic modify-in-place
- Source: DEFICIENCIES.md, LEFTOSPEC.md

---

## Counts by Theme

| Theme | P0 | P1 | P2 | P3 | Total |
|---|---|---|---|---|---|
| Correctness bugs (Rust) | 10 | - | 10 | - | 20 |
| Correctness bugs (Python/playground) | 15 | - | 11 | - | 26 |
| Spec test gaps (gateway) | - | 11 | - | - | 11 |
| Spec test gaps (risk) | - | 11 | - | - | 11 |
| Spec test gaps (liquidator) | - | 11 | - | - | 11 |
| Spec test gaps (playground) | - | 5 | - | - | 5 |
| Future / spec gaps | - | - | - | 12 | 12 |
| **Total** | **25** | **38** | **21** | **12** | **96** |
