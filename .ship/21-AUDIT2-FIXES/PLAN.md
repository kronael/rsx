# AUDIT2-FIXES — minimal-correct fixes for CTO+CEO round-2 critical findings

Source reports: `../20-CTO-CEO-REVIEW-2/CTO-REPORT.md`,
`../20-CTO-CEO-REVIEW-2/CEO-REPORT.md`. Branch: detached HEAD.

## Forced rank (correctness × effort)

| # | Finding | LOC | Status |
|---|---|---:|---|
| 1 | R-N3 — `make lint` broken | 12 | ✅ 6e4c6e2 |
| 2 | R-N6 — Gateway advances CMP seq even after `send_raw` fails | ~5 | pending |
| 3 | R-N5 — JtiTracker burns token before 101 response | ~30 | pending |
| 4 | R-N2 — Order ring overflow silently dropped | ~25 | pending |
| 5 | R-N1 — ME has no WAL replay on startup | ~80 | pending |
| 6 | R-N4 — FillRecord wire-format silently extended | ~10 | pending |
| 7 | R-N7 — `Event::OrderFailed` not persisted to WAL | ~25 | pending |
| 8 | CEO #1 — `/trade/` schema mismatch (bundle vs server) | ~20 | pending |
| 9 | CEO #2 — `/x/order` returns 200 OK with engines dead | ~30 | pending |
| 10 | CEO #3 — `/x/*` wildcard 200 sinkhole | ~10 | pending |
| 11 | CEO #4 — `/verify` PASS when 4/7 processes running | ~15 | pending |
| 12 | CEO #5 — `/faults` Restart silent no-op | ~10 | pending |
| 13 | R-N9 — heap alloc per CMP frame | def | deferred (perf, not correctness) |
| 14 | R-N10 — `bench-reference.json` re-seal | def | deferred (CI policy, not code) |
| 15 | R-N8 — 48h WAL retention disk burn | def | deferred (ops, not code) |

## Per-fix plan (minimal-correct, with acceptance test)

### 2. R-N6 — Gate `advance_seq` on `send_raw` success
`rsx-gateway/src/handler.rs:497-504`. Move `sender.advance_seq()`
inside the success branch. If `send_raw` fails, seq stays put;
NAK ring stays consistent with what's actually on the wire.
Acceptance: `cargo test -p rsx-gateway` (add
`tests/handler_send_fail_test.rs`).

### 3. R-N5 — Only record `jti` after 101 is written
`rsx-gateway/src/ws.rs:127-164`. Refactor: validate JWT, parse jti
into a deferred-insert handle, write 101 response, *then* commit
the jti. On write failure, drop the handle without committing.
Acceptance: existing
`rsx-gateway/tests/jti_*` plus a new test that forces a write
failure and reconnects with same JWT.

### 4. R-N2 — Stall on `order_prod` full (match `fill_prod` pattern)
`rsx-risk/src/main.rs:539-544`. Replace `is_err()` warn-and-drop
with a bounded retry loop that drains `accepted_cons` between
attempts, matching the pattern at lines 690-714 for `fill_prod`.
Acceptance: shrink ring to 1, send 2 orders, both complete.

### 5. R-N1 — Replay WAL after snapshot load on ME startup
`rsx-matching/src/main.rs` after `load_snapshot`. Open
`WalReader::open_from_seq(symbol_id, book.sequence + 1, &wal_dir)`
and apply each record to the in-memory book until EOF. Reuse
existing `Event` apply paths. Acceptance: integration test in
`rsx-matching/tests/replay_after_snapshot_test.rs`.

### 6. R-N4 — Reject `taker_ts_ns=0` instead of guessing
`rsx-gateway/src/route.rs:52-58` + `rsx-risk/src/main.rs:659-666`.
Replace the `> 1.7e18` plausibility check with a strict requirement:
if `taker_ts_ns == 0`, fall back to `ts_ns` deterministically; if
nonzero, treat as authoritative — no numeric magic. Also add an
explicit comment + doc-line in the FillRecord definition that
offset 88 is `taker_ts_ns: u64` (V1 wire format).
Bumping `WAL_HEADER_VERSION_LATEST` to V2 is out of scope for this
fix (it requires a coordinated reader-rejection rollout); the strict
check is the minimal-correct version.
Acceptance: receiver-side fuzz that injects arbitrary 8 bytes at
offset 88 and asserts the latency math is either correct (matches
ts_ns) or rejected — never a 100-year-anchor anomaly.

### 7. R-N7 — Persist `Event::OrderFailed` to WAL
`rsx-matching/src/wal_integration.rs:170-172`. Add a
`RECORD_ORDER_FAILED` constant (already exists in messages? verify),
serialise the Event::OrderFailed payload, append. Mirrors the
existing OrderInserted / OrderDone paths.
Acceptance: regression test in
`rsx-matching/tests/order_failed_wal_test.rs`.

### 8. CEO #1 — `/trade/` schema reconcile
Server returns `{"symbols":[{...}]}` but bundle expects
`{M:[[id,tickSize,lotSize,name]]}`. Decide direction: server-side
add a `M:` tuple field alongside `symbols:` (additive, backwards-
compatible). Acceptance: open `/trade/` headless, pair selector
populates within 3 s.

### 9. CEO #2 — `/x/order` propagates ME/GW health
Playground `/x/order` handler must check process health for the
target symbol and return 503 if ME or GW is down. Acceptance:
kill `me-pengu`, POST `/x/order`, response is 503 with a reason.

### 10. CEO #3 — `/x/*` 404 unknown paths
Replace catch-all 200 with explicit 404 default. Acceptance:
`POST /x/literally_random_text` returns 404.

### 11. CEO #4 — `/verify` "processes running 4/7" must FAIL when N < total
The check currently labels itself PASS by string-match instead of
counting. Acceptance: kill one process, click Run All Checks, that
row reports FAIL.

### 12. CEO #5 — `/faults` Restart actually restarts
Either route Restart through the same code path as `/control`
Start, or remove Restart from /faults (CEO's audit phrased
this as "two functionally different verbs"). Acceptance: kill
mark, click Restart on /faults, mark is running within 5 s.

## Discipline

- One commit per fix, prefix `[fix-RNx]` or `[fix-CEOx]`.
- `cargo test --workspace --lib --tests` after each commit must
  pass.
- `make lint` after each commit must exit 0.
- Each fix file ≤ 50 LOC unless absolutely forced.
- No refactor passes alongside the fix.
- Diary entry at the end summarising what shipped.
