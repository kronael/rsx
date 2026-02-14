# TODO: High Priority Bugs (16)

Fix before production. Incorrect behavior, data integrity, race conditions.

## Rust (7)

### [ ] Rate limiter double-counts tokens
- **File:** rsx-gateway/src/rate_limit.rs:44-48
- **Bug:** `advance_time_by()` adds tokens but doesn't update
  `last_refill`. Next `try_consume()` re-adds same time delta.
- **Fix:** Add `self.last_refill = Instant::now();` in advance_time_by.

### [ ] WAL rotation: no sync before rename
- **File:** rsx-dxs/src/wal.rs:182-187
- **Bug:** Old file dropped without explicit sync_all() before
  rename. Crash between drop and rename loses data.
- **Fix:** Call `file.sync_all()?;` before drop and rename.

### [ ] NAK retransmit fails silently
- **File:** rsx-dxs/src/cmp.rs:151-184
- **Bug:** If WalReader::open_from_seq() fails (file missing,
  corrupt), NAK is silently ignored. Receiver never gets missing
  sequences. No retry.
- **Fix:** Return error or re-queue NAK for retry after backoff.

### [ ] Liquidation slip calculation overflow
- **File:** rsx-risk/src/liquidation.rs:166-173
- **Bug:** `round * round * base_slip_bps` overflows i64.
  `(10_000 - slip)` goes negative when slip > 10,000.
  Corrupted liquidation prices.
- **Fix:** Use checked_mul, cap slip at 9,999 (99.99%).

### [ ] Funding premium overflow
- **File:** rsx-risk/src/funding.rs:26-27
- **Bug:** `(mark - index) * 10_000` overflows i64 with extreme
  mark/index values. Funding invariant broken.
- **Fix:** Use i128 intermediate for multiplication.

### [ ] Snapshot level load: 3 missing bounds checks
- **File:** rsx-book/src/snapshot.rs:198, 224, 245
- **Bug:** (Same as critical — listed here for tracking if split)

### [ ] Event buffer overflow in book
- **File:** rsx-book/src/book.rs:88-89
- **Bug:** event_len incremented without checking MAX_EVENTS (10k).
  Buffer overflow if huge order cascades through many levels.
- **Fix:** Guard emit() with `if self.event_len >= MAX_EVENTS`.

## Python (9)

### [ ] Implement audit logging
- **File:** server.py (new feature)
- **Spec:** PLAYGROUND-DASHBOARD.md section 5.3
- **Bug:** No audit trail for destructive operations.
- **Fix:** Add audit_log() function writing JSONL to log/audit.log.
  Call from all POST endpoints. See FIX.md for implementation.

### [ ] Add staging confirmation gate
- **File:** server.py (new feature)
- **Spec:** PLAYGROUND-DASHBOARD.md section 5.2
- **Bug:** Destructive actions need no confirmation in staging.
- **Fix:** Add confirmation token system. See FIX.md for details.

### [ ] Order form field ignored
- **File:** server.py:1239-1253
- **Bug:** Form submits `order_type` (LIMIT/MARKET/POST_ONLY) but
  server ignores it. All orders default to LIMIT.
- **Fix:** Add `"order_type": form.get("order_type", "limit")` to
  order_msg dict.

### [ ] Make gateway URL configurable
- **File:** server.py:1220
- **Bug:** Gateway URL hardcoded to ws://localhost:8080.
- **Fix:** `GATEWAY_URL = os.environ.get("GATEWAY_URL", "ws://localhost:8080")`

### [ ] Fix error handling in pages.py (4 locations)
- **Files:**
  - pages.py:~1420 — render_wal_status() direct dict[] access
  - pages.py:~1453 — render_wal_files() direct dict[] access
  - pages.py:~1569 — render_verify() direct dict[] access
- **Fix:** Replace all `s["key"]` with `s.get("key", "-")`.

### [ ] Exception swallowing in stress_client.py
- **File:** stress_client.py:149
- **Bug:** Exception caught but not re-raised, prevents task
  cancellation and cleanup.
- **Fix:** Add `raise` after the print statement.

### [ ] Race condition in test_concurrent_order_cancels
- **File:** tests/api_edge_cases_test.py:809
- **Bug:** Unsafe access to server.recent_orders without checking
  length. KeyError/IndexError if list empty or missing "cid".
- **Fix:** Add length guard and .get() for key access.

### [ ] Missing assertions in concurrent cancel test
- **File:** tests/api_edge_cases_test.py:803-818
- **Bug:** No assertions after concurrent operations complete.
  Test only checks no crash, not correctness.
- **Fix:** Assert all responses are 200, verify cancelled status.

### [ ] Scenario state updated before success in start_all
- **File:** server.py:241-242
- **Bug:** `current_scenario` set before build/spawn. If build fails,
  state is wrong. Already fixed in api_scenario_switch but not in
  start_all().
- **Fix:** Move `current_scenario = scenario` after successful spawn.
