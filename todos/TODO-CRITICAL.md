# TODO: Critical Bugs (10)

Must fix before any deployment. Security, crashes, data corruption.

## Rust (3)

### [ ] Circuit breaker HalfOpen blocks all requests forever
- **File:** rsx-gateway/src/circuit.rs:45
- **Bug:** `State::HalfOpen => false` rejects ALL requests. Circuit
  can never recover to Closed because no test request is allowed.
  Permanent degraded state after any failure burst.
- **Fix:** HalfOpen should return true for one test request:
  ```rust
  State::HalfOpen => {
      self.state = State::Open;  // fail-safe: reopen if test fails
      true
  }
  ```
- **Test:** Trigger threshold failures, wait cooldown, verify recovery.

### [ ] parse_price unchecked overflow corrupts mark prices
- **File:** rsx-mark/src/source.rs:221
- **Bug:** `whole_val * scale + frac_val` overflows i64 with large
  prices. Publishes corrupted mark prices to WAL, breaking all
  downstream consumers (risk, liquidation, funding).
- **Fix:** Use `checked_mul` and `checked_add`, return None on overflow.
- **Test:** Feed price > 9M with scale 1B, verify None returned.

### [ ] Snapshot load panics on out-of-bounds indices (3 locations)
- **File:** rsx-book/src/snapshot.rs:198, 224, 245
- **Bug:** No bounds validation before array/slab indexing. Malformed
  snapshot with bad indices causes panic (DoS).
- **Fix:** Validate `idx < len` before every indexed access. Return
  Err on out-of-bounds.
- **Test:** Load snapshot with idx=u32::MAX, verify error not panic.

## Python (7)

### [ ] XSS: 5 unescaped HTML outputs in pages.py
- **Files/Lines:**
  - pages.py:~1507 — render_logs() uses raw line, not escaped_line
  - pages.py:~1743 — render_risk_user() unescaped dict keys/values
  - pages.py:~1532 — render_error_agg() unescaped error patterns
  - pages.py:~1562 — render_verify() unescaped check details
  - server.py stress reports — FIXED (html.escape added)
- **Fix:** Add `import html` to pages.py, wrap all user data with
  `html.escape(str(value))` before rendering.
- **Test:** Submit data containing `<script>alert(1)</script>`,
  verify it renders as escaped text in browser.

### [ ] JSONResponse type mismatch crashes stress reports table
- **File:** server.py:1467-1509
- **Bug:** `api_stress_reports()` returns JSONResponse object.
  `x_stress_reports_list()` calls it directly and tries to access
  `.body` attribute which doesn't exist. AttributeError crash.
- **Fix:** Change `api_stress_reports()` to return raw list instead
  of JSONResponse. Or extract shared logic into helper function.
- **Test:** Visit /x/stress-reports-list after running a stress test.

### [ ] Missing subprocess import in conftest.py
- **File:** rsx-playground/tests/conftest.py:47
- **Bug:** `subprocess.TimeoutExpired` referenced but `subprocess`
  never imported. NameError crash during test cleanup.
- **Fix:** Add `import subprocess` at top of conftest.py.
- **Test:** Run pytest — cleanup fixture should work without NameError.

### [ ] No production environment guard
- **File:** server.py (missing)
- **Spec:** PLAYGROUND-DASHBOARD.md section 5.1
- **Bug:** Playground can run in production without any safeguard.
  All destructive operations (kill process, reset state, liquidate)
  accessible.
- **Fix:** Add PLAYGROUND_MODE env var check at startup. Refuse to
  start if mode is 'production'. See FIX.md for full implementation.

### [ ] Duplicate client fixture shadows conftest.py
- **File:** rsx-playground/tests/api_e2e_test.py:11-14
- **Bug:** `client` fixture redefined locally, shadows the one from
  conftest.py. Different initialization between test files.
- **Fix:** Delete the duplicate fixture in api_e2e_test.py.
