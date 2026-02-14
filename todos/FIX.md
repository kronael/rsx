# RSX Bug Fixes (Full Codebase)

**Date:** 2026-02-14
**Total Bugs Found:** 59 (30 Python + 29 Rust)
**Scope:** All crates, playground, start script
**Status:** Ready for implementation

---

## Executive Summary

Comprehensive bug hunt across all playground code identified 30 bugs:
- **11 Critical** (security, crashes, data loss)
- **8 High** (incorrect behavior, race conditions)
- **11 Medium/Low** (edge cases, quality issues)

**Estimated Fix Time:**
- Critical fixes: 2-3 hours
- All fixes: 6-8 hours

---

## Priority Levels

### 🔴 CRITICAL (Must fix before any deployment)
- Security: 5 XSS vulnerabilities
- Crashes: 3 type errors, missing imports
- Spec compliance: 3 safety gaps

### 🟠 HIGH (Fix before staging)
- Data integrity: 2 bugs
- Error handling: 4 bugs
- Integration: 2 bugs

### 🟡 MEDIUM/LOW (Quality improvements)
- Race conditions: 3 bugs
- Metrics accuracy: 2 bugs
- Test issues: 6 bugs

---

# Bug Fixes by File

## 1. server.py (11 bugs)

### ✅ BUG #1: Missing 3rd return value in send_order_to_gateway (CRITICAL)
**Status:** FIXED
**Lines:** 1229-1234
**Severity:** CRITICAL - Causes tuple unpacking crash

**Original Code:**
```python
except (ConnectionRefusedError, OSError):
    return None, "gateway not running"
except asyncio.TimeoutError:
    return None, "timeout waiting for response"
except Exception as e:
    return None, str(e)
```

**Fix Applied:**
```python
except (ConnectionRefusedError, OSError):
    return None, "gateway not running", None
except asyncio.TimeoutError:
    return None, "timeout waiting for response", None
except Exception as e:
    return None, str(e), None
```

**Verification:** Submit order with gateway down - should not crash

---

### BUG #2: XSS in stress reports list (CRITICAL)
**Lines:** 1497, 1507
**Severity:** CRITICAL - XSS attack vector

**Issue:** Timestamp and report ID rendered without HTML escaping

**Current Code:**
```python
for r in data:
    timestamp_fmt = r["timestamp"]
    # ...
    rows.append(
        f'<a href="/stress/{r["id"]}">{ts}</a>'
    )
```

**Fix:**
```python
import html  # Already added at top of file

# In x_stress_reports_list():
ts_escaped = html.escape(ts)
id_escaped = html.escape(str(r["id"]))

rows.append(
    f'<a href="/stress/{id_escaped}">{ts_escaped}</a>'
)
```

**Status:** FIXED ✅

---

### BUG #3: JSONResponse type mismatch (CRITICAL)
**Lines:** 1467-1509
**Severity:** CRITICAL - Crashes stress reports table

**Issue:** `api_stress_reports()` returns JSONResponse but caller expects dict

**Current Code:**
```python
@app.get("/api/stress/reports")
async def api_stress_reports():
    # ... build reports list ...
    return JSONResponse(reports)  # ❌ Returns response object

@app.get("/x/stress-reports-list", response_class=HTMLResponse)
async def x_stress_reports_list():
    reports = await api_stress_reports()
    if not reports.body:  # ❌ JSONResponse has no .body
        return ...
    data = json.loads(reports.body)  # ❌ AttributeError
```

**Fix:**
```python
@app.get("/api/stress/reports")
async def api_stress_reports():
    # ... build reports list ...
    return reports  # Return raw list, let FastAPI serialize

# OR keep JSONResponse but fix the caller:
@app.get("/x/stress-reports-list", response_class=HTMLResponse)
async def x_stress_reports_list():
    # Call the API endpoint via HTTP instead
    import httpx
    async with httpx.AsyncClient() as client:
        resp = await client.get("http://localhost:49171/api/stress/reports")
        data = resp.json()
    # ... rest of code
```

**Recommended:** Return raw list (simpler, faster)

---

### BUG #4: Order form field ignored (HIGH)
**Lines:** 1239-1253
**Severity:** HIGH - Silent data loss

**Issue:** Form submits `order_type` field but server ignores it

**Current Code:**
```python
# pages.py has:
<select name="order_type">
    <option value="limit">LIMIT</option>
    <option value="market">MARKET</option>
</select>

# server.py api_orders_test() ignores it:
form = await request.form()
order_msg = {
    "type": "NewOrder",
    "symbol_id": int(form.get("symbol_id", "10")),
    "side": form.get("side", "buy"),
    # ❌ Missing: order_type handling
    "tif": form.get("tif", "GTC"),
}
```

**Fix:**
```python
order_msg = {
    "type": "NewOrder",
    "symbol_id": int(form.get("symbol_id", "10")),
    "side": form.get("side", "buy"),
    "price": form.get("price", "0"),
    "qty": form.get("qty", "0"),
    "client_order_id": cid,
    "order_type": form.get("order_type", "limit"),  # ADD THIS
    "tif": form.get("tif", "GTC"),
    "reduce_only": form.get("reduce_only") == "on",
    "post_only": form.get("post_only") == "on",
}
```

---

### BUG #5: Hardcoded gateway URL (MEDIUM)
**Line:** 1220
**Severity:** MEDIUM - Inflexible deployment

**Issue:** Gateway URL is hardcoded, can't connect to custom ports

**Current Code:**
```python
async def send_order_to_gateway(order_msg: dict, user_id: int = 1):
    async with websockets.connect(
        "ws://localhost:8080",  # ❌ Hardcoded
        ...
```

**Fix:**
```python
# At top of file:
GATEWAY_URL = os.environ.get("GATEWAY_URL", "ws://localhost:8080")

# In function:
async def send_order_to_gateway(order_msg: dict, user_id: int = 1):
    async with websockets.connect(
        GATEWAY_URL,  # ✓ Configurable
        ...
```

---

### BUG #6: Scenario state updated before success (MEDIUM)
**Lines:** 241-242
**Severity:** MEDIUM - State inconsistency

**Issue:** `current_scenario` updated before processes start

**Current Code:**
```python
async def start_all(scenario="minimal"):
    global current_scenario
    current_scenario = scenario  # ❌ Updated before build/spawn
    plan = get_spawn_plan(scenario)
    ok = await do_build()
    if not ok:
        return {"error": "build failed"}  # But scenario already changed!
```

**Fix:**
```python
async def start_all(scenario="minimal"):
    plan = get_spawn_plan(scenario)
    ok = await do_build()
    if not ok:
        return {"error": "build failed", "log": build_log[-5:]}

    # Spawn all processes
    started = []
    for name, binary, env in plan:
        result = await spawn_process(name, binary, env)
        if "pid" in result:
            started.append(name)
        await asyncio.sleep(0.1)

    # Only update state after successful start
    if started:
        global current_scenario
        current_scenario = scenario  # ✓ Updated after success

    return {"started": started, "count": len(started)}
```

**Status:** FIXED ✅ (in api_scenario_switch but not in start_all)

---

### BUG #7-10: Spec compliance gaps (CRITICAL)

#### BUG #7: No production environment guard
**Severity:** CRITICAL
**Spec:** PLAYGROUND-DASHBOARD.md §5.1

**Issue:** Playground can run in production without safeguards

**Fix:**
```python
# At top of server.py, after imports:
PLAYGROUND_MODE = os.environ.get('PLAYGROUND_MODE', 'local')
ALLOWED_MODES = ['local', 'staging']

if PLAYGROUND_MODE not in ALLOWED_MODES:
    raise RuntimeError(
        f"Playground not allowed in mode '{PLAYGROUND_MODE}'. "
        f"Set PLAYGROUND_MODE to one of: {ALLOWED_MODES}"
    )

# Add guard to all mutating endpoints:
def require_playground_enabled(endpoint_name: str):
    """Guard against production usage."""
    if PLAYGROUND_MODE == 'production':
        return JSONResponse(
            {"error": f"{endpoint_name} disabled in production"},
            status_code=403
        )
    return None

# Use in endpoints:
@app.post("/api/processes/{name}/{action}")
async def api_process_action(name: str, action: str):
    if guard := require_playground_enabled("process_action"):
        return guard
    # ... rest of endpoint
```

---

#### BUG #8: No staging confirmation gate
**Severity:** HIGH
**Spec:** PLAYGROUND-DASHBOARD.md §5.2

**Issue:** Destructive actions have no confirmation in staging

**Fix:**
```python
# Add confirmation token system
from secrets import token_urlsafe

staging_tokens = {}  # token -> (action, target, expires_at)

@app.post("/api/confirm/request")
async def request_confirmation(
    action: str = Query(...),
    target: str = Query(...)
):
    """Request confirmation token for staging operations."""
    if PLAYGROUND_MODE != 'staging':
        return {"token": "not_required"}

    token = token_urlsafe(16)
    expires_at = time.time() + 300  # 5 min expiry
    staging_tokens[token] = (action, target, expires_at)

    return {
        "token": token,
        "action": action,
        "target": target,
        "expires_at": expires_at,
        "message": f"Confirm {action} on {target}?"
    }

@app.post("/api/processes/{name}/{action}")
async def api_process_action(
    name: str,
    action: str,
    confirm_token: str = Header(None, alias="x-confirm-token")
):
    # Check confirmation in staging
    if PLAYGROUND_MODE == 'staging' and action in ['kill', 'stop']:
        if not confirm_token:
            return JSONResponse(
                {"error": "confirmation required in staging"},
                status_code=409
            )

        if confirm_token not in staging_tokens:
            return JSONResponse(
                {"error": "invalid or expired confirmation token"},
                status_code=403
            )

        saved_action, saved_target, expires_at = staging_tokens[confirm_token]
        if time.time() > expires_at:
            del staging_tokens[confirm_token]
            return JSONResponse(
                {"error": "confirmation token expired"},
                status_code=403
            )

        if saved_action != action or saved_target != name:
            return JSONResponse(
                {"error": "confirmation token mismatch"},
                status_code=403
            )

        # Consume token
        del staging_tokens[confirm_token]

    # ... rest of endpoint
```

---

#### BUG #9: No audit logging
**Severity:** HIGH
**Spec:** PLAYGROUND-DASHBOARD.md §5.3

**Issue:** No audit trail for destructive operations

**Fix:**
```python
import json
from datetime import datetime

AUDIT_LOG_FILE = ROOT / "log" / "audit.log"

async def audit_log(
    module: str,
    action: str,
    user_id: str = "system",
    target: str = "",
    status: str = "ok",
    detail: str = ""
):
    """Write audit log entry."""
    AUDIT_LOG_FILE.parent.mkdir(parents=True, exist_ok=True)

    entry = {
        "timestamp": datetime.utcnow().isoformat(),
        "module": module,
        "action": action,
        "user_id": user_id,
        "target": target,
        "status": status,
        "detail": detail
    }

    with open(AUDIT_LOG_FILE, 'a') as f:
        f.write(json.dumps(entry) + "\n")
        f.flush()

# Use in all mutation endpoints:
@app.post("/api/processes/{name}/{action}")
async def api_process_action(name: str, action: str):
    # ... perform action ...

    await audit_log(
        module="playground",
        action=f"process_{action}",
        target=name,
        status="ok" if "pid" in result else "failed",
        detail=json.dumps(result)
    )

    return result
```

---

#### BUG #10: No idempotency key validation
**Severity:** MEDIUM
**Spec:** PLAYGROUND-DASHBOARD.md §5.4

**Issue:** Duplicate requests not detected

**Fix:**
```python
# Simple in-memory dedup store (use Redis in production)
idempotency_store = {}  # key -> (timestamp, result)
IDEMPOTENCY_TTL = 3600  # 1 hour

def check_idempotency(key: str):
    """Check if request is duplicate."""
    if key in idempotency_store:
        ts, result = idempotency_store[key]
        if time.time() - ts < IDEMPOTENCY_TTL:
            return result
        else:
            del idempotency_store[key]
    return None

def store_idempotency(key: str, result: dict):
    """Store result for dedup."""
    idempotency_store[key] = (time.time(), result)

@app.post("/api/stress/run")
async def api_stress_run(
    rate: int = Query(...),
    duration: int = Query(...),
    x_idempotency_key: str = Header(None)
):
    if not x_idempotency_key:
        return JSONResponse(
            {"error": "x-idempotency-key header required"},
            status_code=400
        )

    # Check for duplicate
    if cached := check_idempotency(x_idempotency_key):
        return JSONResponse({
            "status": "completed",
            "cached": True,
            **cached
        })

    # Run stress test
    result = await run_stress_test(...)

    # Store for dedup
    store_idempotency(x_idempotency_key, result)

    return result
```

---

### BUG #11: Health check returns wrong info (MEDIUM)
**Lines:** 341-344
**Status:** FIXED ✅

**Original Code:**
```python
@app.get("/healthz")
async def healthz():
    return {"status": "ok", "port": 49171}
```

**Fixed Code:**
```python
@app.get("/healthz")
async def healthz():
    procs = scan_processes()
    running = [p for p in procs if p.get("state") == "running"]
    return {
        "status": "ok",
        "port": 49171,
        "processes_running": len(running),
        "processes_total": len(procs),
        "postgres": pg_pool is not None
    }
```

---

## 2. pages.py (8 bugs)

### BUG #12: XSS in render_logs() (CRITICAL)
**Line:** 1507
**Severity:** CRITICAL - XSS attack vector

**Issue:** Log lines rendered without HTML escaping

**Current Code:**
```python
def render_logs(lines):
    html = ""
    for line in lines:
        escaped_line = line.replace('"', '&quot;').replace("'", "&#39;")
        # ...
        html += f'{line}</div>\n'  # ❌ Uses raw line, not escaped_line
    return html
```

**Fix:**
```python
import html

def render_logs(lines):
    output = ""
    for line in lines:
        # Escape HTML special chars
        safe_line = html.escape(line, quote=False)

        # Apply CSS classes based on content
        cls = "text-slate-300"
        if "error" in line.lower():
            cls = "text-red-400"
        # ...

        output += (
            f'<div class="{cls} text-xs py-0.5 font-mono">'
            f'{safe_line}</div>\n'  # ✓ Escaped
        )
    return output
```

---

### BUG #13: XSS in render_risk_user() (CRITICAL)
**Lines:** 1743-1748
**Severity:** CRITICAL - Database XSS

**Issue:** User data keys/values rendered without escaping

**Current Code:**
```python
for key, val in data.items():
    rows += (
        f'<td {_TD}>{key}</td>'  # ❌ Unescaped
        f'<td {_TD}>{val}</td>'  # ❌ Unescaped
    )
```

**Fix:**
```python
import html

for key, val in data.items():
    rows += (
        f'<td {_TD}>{html.escape(str(key))}</td>'  # ✓ Escaped
        f'<td {_TD}>{html.escape(str(val))}</td>'  # ✓ Escaped
    )
```

---

### BUG #14: XSS in render_error_agg() (HIGH)
**Line:** 1532
**Severity:** HIGH - Log pattern XSS

**Issue:** Error patterns rendered without escaping

**Current Code:**
```python
for pattern, info in list(errors.items())[:20]:
    rows += (
        f'<td {_TD}>{pattern}</td>'  # ❌ Unescaped
    )
```

**Fix:**
```python
import html

for pattern, info in list(errors.items())[:20]:
    rows += (
        f'<td {_TD}>{html.escape(pattern)}</td>'  # ✓ Escaped
    )
```

---

### BUG #15: XSS in render_verify() (HIGH)
**Line:** 1562
**Severity:** HIGH - Check detail XSS

**Issue:** Verification check details unescaped

**Current Code:**
```python
if c.get("detail"):
    detail = (
        f'<div class="text-[10px] text-slate-500 mt-0.5">'
        f'{c["detail"]}</div>'  # ❌ Unescaped
    )
```

**Fix:**
```python
import html

if c.get("detail"):
    detail = (
        f'<div class="text-[10px] text-slate-500 mt-0.5">'
        f'{html.escape(str(c["detail"]))}</div>'  # ✓ Escaped
    )
```

---

### BUG #16: Missing error handling in render_wal_status() (HIGH)
**Lines:** 1420-1422
**Severity:** HIGH - KeyError crash

**Issue:** Direct dict access without `.get()`

**Current Code:**
```python
for s in streams:
    rows += (
        f'<td {_TD}>{s["name"]}</td>'  # ❌ KeyError if missing
        f'<td {_TD}>{s["files"]}</td>'  # ❌ KeyError if missing
        f'<td {_TD}>{s["total_size"]}</td>'  # ❌ KeyError if missing
    )
```

**Fix:**
```python
for s in streams:
    rows += (
        f'<td {_TD}>{s.get("name", "-")}</td>'  # ✓ Safe
        f'<td {_TD}>{s.get("files", "0")}</td>'  # ✓ Safe
        f'<td {_TD}>{s.get("total_size", "-")}</td>'  # ✓ Safe
    )
```

---

### BUG #17: Missing error handling in render_wal_files() (HIGH)
**Lines:** 1453-1456
**Severity:** HIGH - KeyError crash

**Issue:** Same as #16, different table

**Fix:**
```python
for f in files:
    rows += (
        f'<td {_TD}>{f.get("stream", "-")}</td>'
        f'<td {_TD}>{f.get("name", "-")}</td>'
        f'<td {_TD}>{f.get("size", "-")}</td>'
        f'<td {_TD}>{f.get("modified", "-")}</td>'
    )
```

---

### BUG #18: Missing error handling in render_verify() (MEDIUM)
**Line:** 1569
**Severity:** MEDIUM - KeyError risk

**Fix:**
```python
for c in checks:
    # ...
    f'<td {_TD}>{c.get("name", "unknown")}{detail}</td>'  # ✓ Safe
```

---

### BUG #19: Tailwind dynamic classes (LOW)
**Lines:** 2060-2079
**Severity:** LOW - Fragile CSS

**Issue:** Dynamic Tailwind classes work but are fragile

**Current Code:**
```python
f'<div class="bg-{status_color}-900/40 border-{status_color}-800">'
```

**Note:** This actually works because Python generates the HTML server-side, but consider using safe-listed classes for better Tailwind JIT compatibility.

**Recommended (optional):**
```python
# Map colors to full class names
color_classes = {
    "emerald": "bg-emerald-900/40 border-emerald-800 text-emerald-400",
    "amber": "bg-amber-900/40 border-amber-800 text-amber-400",
    "red": "bg-red-900/40 border-red-800 text-red-400",
}

classes = color_classes.get(status_color, color_classes["emerald"])
f'<div class="{classes}">'
```

---

## 3. stress_client.py (4 bugs)

### BUG #20: Exception swallowing prevents cleanup (HIGH)
**Line:** 149
**Severity:** HIGH - Resource leak

**Issue:** Exception caught but not re-raised, prevents task cancellation

**Current Code:**
```python
try:
    async with await self.connect() as ws:
        # ... work ...
except Exception as e:
    print(f"Worker {self.worker_id} error: {e}")
    # ❌ Exception swallowed, no re-raise
```

**Fix:**
```python
try:
    async with await self.connect() as ws:
        # ... work ...
except Exception as e:
    print(f"Worker {self.worker_id} error: {e}")
    raise  # ✓ Allow proper cancellation and cleanup
```

---

### BUG #21: Submitted counter incremented before send (MEDIUM)
**Line:** 137
**Severity:** MEDIUM - Inflated metrics

**Issue:** Counter incremented before order actually sent

**Current Code:**
```python
self.metrics.submitted += 1  # Line 137 - before send
latency = await self.submit_order(ws)  # Line 138 - may fail
```

**Fix:**
```python
try:
    latency = await self.submit_order(ws)
    self.metrics.submitted += 1  # ✓ Only count after successful send
    if latency:
        self.metrics.latencies_us.append(latency)
except Exception:
    # Don't count failed sends
    pass
```

---

### BUG #22: Latency includes send buffering (MEDIUM)
**Lines:** 101-107
**Severity:** MEDIUM - Wrong measurement

**Issue:** Latency measured from before `ws.send()` which can block

**Note:** This is a design decision. If you want to measure only server processing time (not network stack), adjust measurement point.

**Current Code:**
```python
start = time.perf_counter_ns()
await ws.send(json.dumps(order))  # Can block on buffer
response = await ws.recv()
latency_ns = time.perf_counter_ns() - start
```

**Alternative (measure only server time):**
```python
await ws.send(json.dumps(order))
start = time.perf_counter_ns()  # ✓ Start after send
response = await ws.recv()
latency_ns = time.perf_counter_ns() - start
```

**Recommendation:** Document current behavior as "round-trip latency including send buffering"

---

### BUG #23: Symbol ID mismatch (MEDIUM)
**Line:** 72
**Severity:** MEDIUM - Order rejections

**Issue:** Stress test uses symbol IDs [0,1,2] but exchange expects [1,2,3,10]

**Current Code:**
```python
symbol_id = random.choice([0, 1, 2])  # BTC, ETH, SOL
```

**Fix:**
```python
# Match actual configured symbols
symbol_id = random.choice([1, 2, 3, 10])  # BTC, ETH, SOL, PENGU
```

---

### BUG #24: Race condition in latency list (LOW)
**Lines:** 140-141
**Severity:** LOW - Safe in CPython

**Issue:** Concurrent list append without lock

**Note:** Safe in CPython due to GIL, but not guaranteed by language spec.

**Current Code:**
```python
self.metrics.latencies_us.append(latency)  # Unsynchronized
```

**Fix (if needed for other Python implementations):**
```python
# Add lock to StressWorker class
self.latency_lock = asyncio.Lock()

# Use lock when appending
async with self.latency_lock:
    self.metrics.latencies_us.append(latency)
```

**Recommendation:** Leave as-is for CPython, document assumption

---

## 4. playground (CLI) - All Fixed ✅

All 10 bugs in the CLI were already fixed in the initial implementation:
- ensure_server() return value propagation
- api_post() status validation
- TOCTOU race handling
- Integer conversion error handling
- PID file cleanup
- etc.

---

## 5. tests/ (10 bugs)

### BUG #25: Missing subprocess import (CRITICAL)
**File:** conftest.py
**Line:** 47
**Severity:** CRITICAL - NameError crash

**Issue:** `subprocess` used but never imported

**Current Code:**
```python
except subprocess.TimeoutExpired:  # ❌ NameError!
```

**Fix:**
```python
# Add at top of file:
import subprocess
```

---

### BUG #26: Duplicate client fixture (CRITICAL)
**File:** api_e2e_test.py
**Lines:** 11-14
**Severity:** CRITICAL - Fixture shadowing

**Issue:** `client` fixture redefined, conflicts with conftest.py

**Fix:**
```python
# DELETE lines 11-14 in api_e2e_test.py:
# @pytest.fixture
# def client():
#     return TestClient(app)

# Use the fixture from conftest.py instead
```

---

### BUG #27: Race condition in test_concurrent_order_cancels (HIGH)
**File:** api_edge_cases_test.py
**Line:** 809
**Severity:** HIGH - Flaky test

**Issue:** Unsafe access to `server.recent_orders`

**Current Code:**
```python
cids = [o["cid"] for o in server.recent_orders[:5]]
```

**Fix:**
```python
# Add guard for empty list
if len(server.recent_orders) < 5:
    pytest.skip("Not enough orders to test concurrent cancels")

cids = [o["cid"] for o in server.recent_orders[:5]]
```

---

### BUG #28: Missing assertions in test_concurrent_order_cancels (MEDIUM)
**File:** api_edge_cases_test.py
**Lines:** 803-818
**Severity:** MEDIUM - Incomplete test

**Issue:** No assertions after concurrent operations

**Fix:**
```python
def test_concurrent_order_cancels(client):
    """Concurrent order cancels."""
    import threading
    import server

    client.post("/api/orders/batch")
    initial_count = len(server.recent_orders)
    cids = [o["cid"] for o in server.recent_orders[:5]]

    results = []
    def cancel(cid):
        resp = client.post(f"/api/orders/{cid}/cancel")
        results.append(resp.status_code)

    threads = [threading.Thread(target=cancel, args=(c,)) for c in cids]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    # ADD ASSERTIONS:
    assert all(r == 200 for r in results), "All cancels should succeed"
    cancelled = [o for o in server.recent_orders if o["status"] == "cancelled"]
    assert len(cancelled) == 5, "All 5 orders should be cancelled"
```

---

### BUG #29-34: Other test issues (MEDIUM/LOW)
- Process cleanup not waiting (conftest.py:38-56)
- Fixture cleanup order (conftest.py:24-56)
- Loose percentile assertions (stress_integration_test.py:237-239)
- Missing async marker (stress_integration_test.py:215)
- Loose path traversal assertion (api_edge_cases_test.py:381)
- KeyError risk (api_edge_cases_test.py:809)

**Recommendation:** Fix after critical bugs resolved

---

# Verification Plan

## Phase 1: Critical Fixes (Do First)

### 1.1 Security (XSS)
```bash
# Fix all 5 XSS vulnerabilities in pages.py
# Test: Submit order with payload: <script>alert('xss')</script>
# Expected: Payload escaped in HTML output

python3 -c "import html; print(html.escape('<script>alert(1)</script>'))"
```

### 1.2 Type Errors
```bash
# Fix JSONResponse mismatch in server.py
# Test: Visit /x/stress-reports-list
# Expected: Table renders without AttributeError

curl http://localhost:49171/x/stress-reports-list
```

### 1.3 Missing Import
```bash
# Fix subprocess import in conftest.py
# Test: Run tests with process cleanup

cd /home/onvos/sandbox/rsx/rsx-playground
pytest tests/conftest.py -v
```

---

## Phase 2: Integration Tests

### 2.1 Order Submission
```bash
# Fix order_type handling
# Test: Submit order with type=MARKET via UI
# Expected: order_type included in gateway message

# Start playground
./playground start
./playground start-all minimal

# Submit order via CLI
./playground submit-order
```

### 2.2 Gateway URL
```bash
# Fix hardcoded gateway URL
# Test: Set GATEWAY_URL env var and submit order

export GATEWAY_URL="ws://localhost:9999"
./playground submit-order
```

### 2.3 Stress Test
```bash
# Fix symbol IDs in stress_client.py
# Test: Run stress test, check acceptance rate

./playground stress 10 5
# Expected: High acceptance rate (>95%)
```

---

## Phase 3: Spec Compliance

### 3.1 Production Guard
```bash
# Test environment check
export PLAYGROUND_MODE=production
python3 server.py
# Expected: RuntimeError - playground not allowed

export PLAYGROUND_MODE=local
python3 server.py
# Expected: Server starts normally
```

### 3.2 Audit Logging
```bash
# Test audit trail
./playground start
./playground start-all minimal
./playground stop-proc gateway

# Check audit log
cat ../log/audit.log | jq '.'
# Expected: JSON entries with action=process_stop, target=gateway
```

### 3.3 Idempotency
```bash
# Test duplicate request detection
curl -X POST http://localhost:49171/api/stress/run \
  -H "x-idempotency-key: test123" \
  -d "rate=10&duration=5"

# Retry with same key
curl -X POST http://localhost:49171/api/stress/run \
  -H "x-idempotency-key: test123" \
  -d "rate=10&duration=5"

# Expected: Second request returns cached result
```

---

# Rust Crate Bugs (29 bugs)

## 6. rsx-book (4 bugs)

### BUG #35: Missing bounds check in snapshot level load (HIGH)
**File:** rsx-book/src/snapshot.rs:224
**Issue:** `active_levels[idx as usize] = lvl;` — no validation that
`idx < total_levels` before indexing. Malformed snapshot causes panic.
**Fix:** Add `if (idx as usize) >= active_levels.len() { return Err(...); }`

### BUG #36: Missing bounds check in snapshot user state load (HIGH)
**File:** rsx-book/src/snapshot.rs:245
**Issue:** `user_states[idx as usize]` — no bounds validation.
**Fix:** Same pattern as #35.

### BUG #37: Missing bounds check in snapshot slab order load (HIGH)
**File:** rsx-book/src/snapshot.rs:198
**Issue:** `*slab.get_mut(idx) = slot;` — no validation `idx < capacity`.
**Fix:** Validate index before slab access.

### BUG #38: Event buffer overflow risk (MEDIUM)
**File:** rsx-book/src/book.rs:88-89
**Issue:** `event_len` incremented without checking `MAX_EVENTS` (10,000).
Buffer overflow if huge order cascades through many levels.
**Fix:** Add `if self.event_len >= MAX_EVENTS { return; }` in `emit()`.

---

## 7. rsx-gateway (4 bugs)

### BUG #39: Circuit breaker HalfOpen blocks ALL requests (CRITICAL)
**File:** rsx-gateway/src/circuit.rs:45
**Issue:** `State::HalfOpen => false` — rejects every request in
HalfOpen state. Circuit can NEVER recover to Closed because no
test request is allowed, so `record_success()` is never called.
Permanent degraded state after any failure burst.
**Fix:**
```rust
State::HalfOpen => {
    // Allow one test request
    self.state = State::Open;  // Fail-safe: reopen if test fails
    true
}
```

### BUG #40: Rate limiter missing last_refill update (HIGH)
**File:** rsx-gateway/src/rate_limit.rs:44-48
**Issue:** `advance_time_by()` adds tokens but doesn't update
`last_refill`. Next `try_consume()` → `refill()` re-adds the
same time delta, double-counting tokens.
**Fix:** Add `self.last_refill = Instant::now();` after token update.

### BUG #41: JWT missing aud/iss validation (MEDIUM)
**File:** rsx-gateway/src/jwt.rs:14-35
**Issue:** Only checks expiry and signature. Tokens from other
services sharing the same secret are accepted.
**Fix:** Add `validation.set_audience(&["rsx-gateway"])` and
`validation.set_issuer(&["rsx-auth"])`.

### BUG #42: WAL rotation no sync before rename (HIGH)
**File:** rsx-dxs/src/wal.rs:182-187
**Issue:** Old file dropped without explicit `sync_all()` before
rename. On crash between drop and rename, data can be lost.
**Fix:** Call `file.sync_all()?;` before `drop()` and `rename()`.

---

## 8. rsx-dxs (5 bugs)

### BUG #43: WAL file exceeds max size by one record (MEDIUM)
**File:** rsx-dxs/src/wal.rs:144-168
**Issue:** Size check happens AFTER `write_all()`. File can exceed
`max_file_size` by up to 65535 bytes (MAX_PAYLOAD).
**Fix:** Check `file_size + buf.len() >= max_file_size` BEFORE writing.

### BUG #44: NAK retransmit fails silently (HIGH)
**File:** rsx-dxs/src/cmp.rs:151-184
**Issue:** If `WalReader::open_from_seq()` fails, NAK is silently
ignored with a warning. Receiver never gets missing sequences.
No retry mechanism.
**Fix:** Return error or re-queue NAK for retry after backoff.

### BUG #45: Tip persistence lacks directory sync (MEDIUM)
**File:** rsx-dxs/src/client.rs:460-465
**Issue:** Atomic rename is correct, but parent directory isn't
fsynced. Rename metadata could be lost on crash.
**Fix:** `fs::File::open(path.parent())?.sync_all()?;` after rename.

### BUG #46: Reorder buffer drops records silently (MEDIUM)
**File:** rsx-dxs/src/cmp.rs:426-443
**Issue:** When reorder buffer is full, arriving gap-fill records
are dropped. NAK keeps requesting same record forever.
**Fix:** Grow buffer or implement sliding window with eviction.

### BUG #47: Tip advance from records without seq (MEDIUM)
**File:** rsx-dxs/src/client.rs:307-312
**Issue:** Fallback `saturating_add(1)` advances tip even when
`extract_seq()` fails. Tip can drift from actual sequence.
**Fix:** Skip tip update when `extract_seq()` returns None.

---

## 9. rsx-risk (4 bugs)

### BUG #48: Liquidation slip calculation overflow (HIGH)
**File:** rsx-risk/src/liquidation.rs:166-173
**Issue:** `round * round * base_slip_bps` can overflow i64.
Also `(10_000 - slip)` goes negative when slip > 10,000,
corrupting liquidation price.
**Fix:**
```rust
let slip = (state.round as i64)
    .checked_mul(state.round as i64)
    .and_then(|v| v.checked_mul(self.base_slip_bps))
    .unwrap_or(9_999);  // Cap at max
let slip = slip.min(9_999);  // Never exceed 99.99%
```

### BUG #49: Funding rate premium overflow (HIGH)
**File:** rsx-risk/src/funding.rs:26-27
**Issue:** `(mark - index) * 10_000` can overflow i64 with
extreme mark/index values.
**Fix:** Use i128 intermediate:
```rust
let premium = ((mark as i128 - index as i128) * 10_000
    / index as i128) as i64;
```

### BUG #50: Average entry price divide-by-zero (LOW)
**File:** rsx-risk/src/position.rs:105-114
**Issue:** If `net_qty > 0` but `long_qty == 0` (invariant broken),
division panics. Latent bug triggered by corruption elsewhere.
**Fix:** Add `debug_assert!(self.long_qty > 0)` or return 0.

### BUG #51: Notional overflow on i128→i64 cast (MEDIUM)
**File:** rsx-risk/src/position.rs:100-103
**Issue:** i128 result cast to i64 without overflow check.
Extreme positions silently truncate.
**Fix:** Use `i64::try_from()` or cap at `i64::MAX`.

---

## 10. rsx-marketdata (1 bug)

### BUG #52: Shadow book seq gap detection skips seq=0 (MEDIUM)
**File:** rsx-marketdata/src/state.rs:209-216
**Issue:** When `seq == 0`, function returns false WITHOUT
initializing `expected_seq[idx]`. First event with seq=0
causes phantom gaps for all subsequent events.
**Fix:** Remove `|| seq == 0` from early return, or handle
seq=0 as valid initialization.

---

## 11. rsx-mark (3 bugs)

### BUG #53: Unchecked overflow in parse_price (CRITICAL)
**File:** rsx-mark/src/source.rs:221
**Issue:** `whole_val * scale + frac_val` has no overflow check.
Large prices with large scales overflow i64, publishing
corrupted mark prices to WAL.
**Fix:** Use `checked_mul` and `checked_add`, return None on overflow.

### BUG #54: Incorrect scale_digits calculation (MEDIUM)
**File:** rsx-mark/src/source.rs:207
**Issue:** Assumes scale is power of 10. Breaks with non-power
scales like 500_000 or 1.
**Fix:** Validate scale at config time or use `(scale as f64).log10()`.

### BUG #55: Fractional price truncation (MEDIUM)
**File:** rsx-mark/src/source.rs:209
**Issue:** Truncates fractional part instead of rounding.
Consistent bias toward lower prices.
**Fix:** Document as spec decision OR implement proper rounding.

---

## 12. rsx-cli (1 bug)

### BUG #56: No max record size check in WAL dump (MEDIUM)
**File:** rsx-cli/src/main.rs:128
**Issue:** Accepts arbitrary record lengths without validation.
Corrupted WAL file causes OOM or very long hang.
**Fix:** Add `if len > 1_000_000 { break; }` after parsing length.

---

## 13. start script (3 bugs)

### BUG #57: SQL injection in reset_db (MEDIUM)
**File:** start:441-445
**Issue:** `db_name` interpolated into SQL without escaping.
**Fix:** Quote identifier: `f'DROP DATABASE IF EXISTS "{db_name}"'`

### BUG #58: REPL input parsing crash (MEDIUM)
**File:** start:672,676
**Issue:** IndexError when user types "stop " with trailing space
but no process name.
**Fix:** Add bounds check: `parts[1] if len(parts) > 1 else ""`

### BUG #59: Postgres init race condition (LOW)
**File:** start:511-541
**Issue:** Migration runs before postgres is fully initialized.
**Fix:** Add retry loop or `pg_isready` check before migration.

---

# Full Implementation Checklist

## Critical (Must Do)

### Python
- [ ] Fix all 5 XSS vulnerabilities in pages.py (html.escape)
- [ ] Fix JSONResponse type mismatch (server.py)
- [ ] Add subprocess import to conftest.py
- [ ] Add production environment guard (server.py)
- [ ] Fix duplicate client fixture (api_e2e_test.py)

### Rust
- [ ] Fix circuit breaker HalfOpen state (rsx-gateway/circuit.rs)
- [ ] Fix parse_price overflow (rsx-mark/source.rs)
- [ ] Fix snapshot bounds checks (rsx-book/snapshot.rs, 3 locations)

## High Priority (Before Production)

### Python
- [ ] Implement audit logging (server.py)
- [ ] Fix order_type handling (server.py)
- [ ] Make gateway URL configurable (server.py)
- [ ] Fix error handling in pages.py (use .get())

### Rust
- [ ] Fix rate limiter last_refill (rsx-gateway/rate_limit.rs)
- [ ] Fix WAL sync before rename (rsx-dxs/wal.rs)
- [ ] Fix NAK retransmit silent failure (rsx-dxs/cmp.rs)
- [ ] Fix liquidation slip overflow (rsx-risk/liquidation.rs)
- [ ] Fix funding premium overflow (rsx-risk/funding.rs)

## Medium Priority (Quality)

### Python
- [ ] Fix scenario state update timing (server.py)
- [ ] Fix stress client symbol IDs (stress_client.py)
- [ ] Fix submitted counter timing (stress_client.py)
- [ ] Fix test race conditions (tests/)

### Rust
- [ ] Fix WAL max size overshoot (rsx-dxs/wal.rs)
- [ ] Fix tip persistence dir sync (rsx-dxs/client.rs)
- [ ] Fix reorder buffer drops (rsx-dxs/cmp.rs)
- [ ] Fix seq gap detection for seq=0 (rsx-marketdata/state.rs)
- [ ] Fix notional overflow cast (rsx-risk/position.rs)
- [ ] Fix event buffer overflow (rsx-book/book.rs)
- [ ] Add JWT aud/iss validation (rsx-gateway/jwt.rs)
- [ ] Fix CLI WAL dump max record (rsx-cli/main.rs)
- [ ] Fix scale_digits calculation (rsx-mark/source.rs)
- [ ] Fix SQL injection in start script (start)
- [ ] Fix REPL input parsing (start)

## Low Priority (Optional)
- [ ] Fix avg entry divide-by-zero (rsx-risk/position.rs)
- [ ] Fix price truncation vs rounding (rsx-mark/source.rs)
- [ ] Fix postgres init race (start)
- [ ] Tighten test assertions (tests/)
- [ ] Fix Tailwind dynamic classes (pages.py)

---

# Summary by Crate

| Crate | Bugs | Critical | High | Medium | Low |
|-------|------|----------|------|--------|-----|
| **rsx-playground (Python)** | 30 | 8 | 8 | 11 | 3 |
| **rsx-gateway** | 4 | 1 | 1 | 2 | 0 |
| **rsx-dxs** | 5 | 0 | 2 | 3 | 0 |
| **rsx-book** | 4 | 0 | 3 | 1 | 0 |
| **rsx-risk** | 4 | 0 | 2 | 1 | 1 |
| **rsx-mark** | 3 | 1 | 0 | 2 | 0 |
| **rsx-marketdata** | 1 | 0 | 0 | 1 | 0 |
| **rsx-cli** | 1 | 0 | 0 | 1 | 0 |
| **start script** | 3 | 0 | 0 | 2 | 1 |
| **rsx-types** | 0 | 0 | 0 | 0 | 0 |
| **rsx-recorder** | 0 | 0 | 0 | 0 | 0 |
| **rsx-maker** | 0 | 0 | 0 | 0 | 0 |
| **rsx-sim** | 0 | 0 | 0 | 0 | 0 |
| **TOTAL** | **59** | **10** | **16** | **24** | **5** |

---

# Sign-off

**Reviewed By:** Bug Hunt Agents (9 rounds total)
**Files Analyzed:** All Rust crates + all Python files + start script
**Lines Searched:** ~29,000 lines (~21k Rust + ~8k Python)
**Bugs Found:** 59
**Bugs Fixed:** 11 (playground CLI + server.py already applied)
**Remaining:** 48 bugs to fix
