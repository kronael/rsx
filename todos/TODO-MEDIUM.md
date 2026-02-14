# TODO: Medium Priority Bugs (24)

Quality improvements. Edge cases, metrics accuracy, hardening.

## Rust (13)

### [ ] WAL file exceeds max size by one record
- **File:** rsx-dxs/src/wal.rs:144-168
- **Fix:** Check `file_size + buf.len() >= max_file_size` BEFORE write.

### [ ] Tip persistence lacks directory sync
- **File:** rsx-dxs/src/client.rs:460-465
- **Fix:** `fs::File::open(path.parent())?.sync_all()?;` after rename.

### [ ] Reorder buffer silently drops records
- **File:** rsx-dxs/src/cmp.rs:426-443
- **Fix:** Grow buffer or implement sliding window with eviction.

### [ ] Tip advance from records without seq
- **File:** rsx-dxs/src/client.rs:307-312
- **Fix:** Skip tip update when extract_seq() returns None.

### [ ] JWT missing aud/iss validation
- **File:** rsx-gateway/src/jwt.rs:14-35
- **Fix:** Add set_audience and set_issuer to validation.

### [ ] Shadow book seq gap detection skips seq=0
- **File:** rsx-marketdata/src/state.rs:209-216
- **Fix:** Remove `|| seq == 0` or handle seq=0 as valid init.

### [ ] Notional overflow on i128 to i64 cast
- **File:** rsx-risk/src/position.rs:100-103
- **Fix:** Use i64::try_from() or cap at i64::MAX.

### [ ] scale_digits assumes power-of-10 scale
- **File:** rsx-mark/src/source.rs:207
- **Fix:** Validate scale at config time or use log10().

### [ ] Fractional price truncation (not rounding)
- **File:** rsx-mark/src/source.rs:209
- **Fix:** Document as spec decision OR implement rounding.

### [ ] CLI WAL dump no max record size check
- **File:** rsx-cli/src/main.rs:128
- **Fix:** Add `if len > 1_000_000 { break; }` after length parse.

### [ ] SQL injection in start script reset_db
- **File:** start:441-445
- **Fix:** Quote identifier: `f'DROP DATABASE IF EXISTS "{db_name}"'`

### [ ] REPL input parsing crash on empty args
- **File:** start:672,676
- **Fix:** Bounds check: `parts[1] if len(parts) > 1 else ""`

### [ ] Idempotency key validation (spec gap)
- **File:** server.py (new feature)
- **Spec:** PLAYGROUND-DASHBOARD.md section 5.4
- **Fix:** Add x-idempotency-key header check on stress endpoints.

## Python (11)

### [ ] Stress client symbol ID mismatch
- **File:** stress_client.py:72
- **Bug:** Uses [0,1,2] but exchange expects [1,2,3,10].
- **Fix:** `random.choice([1, 2, 3, 10])`

### [ ] Submitted counter incremented before send
- **File:** stress_client.py:137
- **Fix:** Move increment inside submit_order after successful send.

### [ ] Latency includes send buffering time
- **File:** stress_client.py:101-107
- **Fix:** Document as round-trip OR move start after ws.send().

### [ ] Test: process cleanup not waiting (conftest)
- **File:** tests/conftest.py:38-56
- **Fix:** Add proper wait with timeout after kill.

### [ ] Test: fixture cleanup order
- **File:** tests/conftest.py:24-56
- **Fix:** Make scope explicit with scope="function".

### [ ] Test: loose percentile assertions
- **File:** tests/stress_integration_test.py:237-239
- **Fix:** Tighten ranges or use exact values with epsilon.

### [ ] Test: missing async marker
- **File:** tests/stress_integration_test.py:215
- **Fix:** Add @pytest.mark.asyncio if function uses async.

### [ ] Test: loose path traversal assertion
- **File:** tests/api_edge_cases_test.py:381
- **Fix:** Check specific status code (400/404/403).

### [ ] Test: KeyError risk in concurrent test
- **File:** tests/api_edge_cases_test.py:809
- **Fix:** Use .get("cid", "") instead of ["cid"].

### [ ] Tailwind dynamic classes (fragile)
- **File:** pages.py:2060-2079
- **Fix:** Map colors to full class name strings.

### [ ] Race condition in latency list (CPython safe)
- **File:** stress_client.py:140-141
- **Fix:** Document CPython GIL assumption or add asyncio.Lock.
