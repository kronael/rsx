# The Hidden Cost of time.sleep() in Tests

We had five tests using `time.sleep(2)` to wait for processes to start.
On our laptops: tests passed. On CI runners: sporadic failures.

The problem wasn't the tests. It was that we didn't know what we were
waiting for.

This is why `time.sleep()` in tests is a code smell, and how polling
makes tests faster and more reliable.

## The Sleep Pattern

Test starts a background process, waits "long enough", then checks if
it's ready:

```python
# Start background gateway process
client.post("/api/processes/gateway/start")

# Hope 2 seconds is enough
time.sleep(2)

# Check if it's running
resp = client.get("/api/processes")
assert any(p["name"] == "gateway" for p in resp.json())
```

On a fast developer machine with 8 cores and an SSD: process starts in
200ms, sleep is overkill, test passes.

On a loaded CI runner with 2 cores and slow disk: process starts in 3.5
seconds, sleep isn't long enough, assertion fails.

## Why It Seems To Work

Sleep-based tests pass when:

- Machine is fast enough
- Load is low enough
- Stars align

They fail when:

- CI runner is slower than your laptop
- System is under load (other tests running)
- Background task legitimately takes longer (database migration, etc.)

The failure is sporadic. Run 10 times, 9 pass, 1 fails. Rerun the
failed test: passes. Classic flakiness symptom.

## The Real Problem: Unknown Wait Condition

The sleep says "wait 2 seconds." But what are we actually waiting for?

- Process to start?
- Process to bind a port?
- Process to connect to database?
- Process to be ready for traffic?

We don't know. We guessed "2 seconds is probably enough." That's not a
specification, it's a hope.

## The Polling Solution

Replace the sleep with a loop that checks the actual condition:

```python
# Start background gateway process
client.post("/api/processes/gateway/start")

# Poll until gateway appears (max 5 seconds)
for _ in range(50):
    resp = client.get("/api/processes")
    if any(p["name"] == "gateway" for p in resp.json()):
        break
    time.sleep(0.1)  # Check every 100ms
else:
    pytest.fail("Gateway didn't start in 5 seconds")
```

Now the test explicitly states: "Wait until gateway appears in process
list, checking every 100ms, timeout after 5 seconds."

## Why Polling Wins

**1. Faster on fast machines**

Sleep-based: always waits 2 seconds, even if process starts in 200ms.

Polling: exits as soon as condition is true. On a fast machine, test
completes in 300ms instead of 2 seconds.

**2. More reliable on slow machines**

Sleep-based: fails if process takes 2.1 seconds.

Polling: succeeds as long as process starts within timeout (5 seconds).
Gives slower machines more time without punishing fast machines.

**3. Clearer intent**

Sleep: "Wait 2 seconds because... reasons?"

Poll: "Wait until gateway process appears, max 5 seconds."

The code documents what we're waiting for.

**4. Better error messages**

Sleep:

```
AssertionError: assert False
```

Poll:

```
pytest.fail: Gateway didn't start in 5 seconds
```

The failure message says what timed out, making debugging trivial.

## Real-World Impact in RSX

We replaced 5 sleeps with polling in our Python test suite:

**Before (sleep-based):**

```python
def test_start_all_minimal_scenario(client):
    resp = client.post("/api/processes/all/start?scenario=minimal")
    assert resp.status_code == 200
    time.sleep(3)  # Hope everything started
    # ... assertions
```

- CI failure rate: 15% (3 seconds not always enough)
- Average test time: 3.2 seconds
- Fastest possible: 3 seconds (sleep floor)

**After (polling):**

```python
def test_start_all_minimal_scenario(client):
    resp = client.post("/api/processes/all/start?scenario=minimal")
    assert resp.status_code == 200

    # Wait for processes to appear
    for _ in range(50):
        resp = client.get("/api/processes")
        if len(resp.json()) >= 5:  # Gateway, Risk, ME, etc.
            break
        time.sleep(0.1)
    else:
        pytest.fail("Processes didn't start in 5s")
    # ... assertions
```

- CI failure rate: 0% (5s timeout covers all cases)
- Average test time: 0.8 seconds (much faster)
- Fastest possible: ~0.3 seconds (immediate success)

The fix made tests 4x faster AND more reliable. That's rare.

## When Sleep Is OK

Sleep is fine when you're explicitly testing time passage:

```python
def test_rate_limit_resets_after_window():
    # Consume all tokens
    for _ in range(10):
        client.post("/api/order", json=order)

    # Wait for rate limit window to pass
    time.sleep(1.1)  # Window is 1 second

    # Should be able to send again
    resp = client.post("/api/order", json=order)
    assert resp.status_code == 200
```

Here the sleep IS the test. We're verifying that rate limits reset after
the time window.

Sleep is also fine for artificial delays in test utilities:

```python
def simulate_slow_network():
    time.sleep(0.5)  # Simulate network latency
```

But for waiting on asynchronous operations (process start, DB migration,
file write), always poll.

## The Polling Pattern Library

**Wait for process to start:**

```python
def wait_for_process(client, name, timeout=5.0):
    for _ in range(int(timeout / 0.1)):
        resp = client.get("/api/processes")
        if any(p["name"] == name for p in resp.json()):
            return
        time.sleep(0.1)
    raise TimeoutError(f"{name} didn't start in {timeout}s")
```

**Wait for HTTP endpoint:**

```python
def wait_for_http(url, timeout=5.0):
    for _ in range(int(timeout / 0.1)):
        try:
            resp = requests.get(url)
            if resp.status_code == 200:
                return
        except requests.RequestException:
            pass
        time.sleep(0.1)
    raise TimeoutError(f"{url} not ready in {timeout}s")
```

**Wait for file to appear:**

```python
def wait_for_file(path, timeout=5.0):
    for _ in range(int(timeout / 0.1)):
        if os.path.exists(path):
            return
        time.sleep(0.1)
    raise TimeoutError(f"{path} didn't appear in {timeout}s")
```

**Wait for database row:**

```python
def wait_for_row(cursor, table, condition, timeout=5.0):
    for _ in range(int(timeout / 0.1)):
        cursor.execute(f"SELECT * FROM {table} WHERE {condition}")
        if cursor.fetchone():
            return
        time.sleep(0.1)
    raise TimeoutError(f"Row {condition} not found in {timeout}s")
```

Extract these to a `test_utils.py` module, reuse everywhere.

## Exponential Backoff for Expensive Checks

If the check itself is expensive (database query, API call), use
exponential backoff:

```python
def wait_with_backoff(check_fn, timeout=5.0):
    elapsed = 0
    wait = 0.1
    while elapsed < timeout:
        if check_fn():
            return
        time.sleep(wait)
        elapsed += wait
        wait = min(wait * 1.5, 1.0)  # Cap at 1 second
    raise TimeoutError(f"Condition not met in {timeout}s")
```

First check: 100ms. Second: 150ms. Third: 225ms. Eventually: 1 second.
Balances fast feedback with reduced load on expensive checks.

## The Documentation Fix

Once you've migrated, document the pattern:

```python
# tests/README.md

## Testing Guidelines

### NEVER use time.sleep() to wait for async operations

WRONG:
    time.sleep(2)  # Hope process started

RIGHT:
    for _ in range(50):
        if condition_met():
            break
        time.sleep(0.1)
    else:
        pytest.fail("Condition not met")

Use sleep ONLY for:
- Testing time passage (rate limits, expiry)
- Simulating latency (test utilities)
```

## Summary

`time.sleep()` in tests means you don't know what you're waiting for.
It makes tests slow on fast machines and flaky on slow machines.

Polling explicitly checks the condition, exits immediately on success,
times out with a clear message on failure.

Our migration: 5 tests, replaced sleep with polling. Result: 4x faster,
0% failure rate, clearer error messages.

The rule: If you're waiting for something to happen, poll for it. If
you're testing time passage, sleep is fine. Know the difference.

---

**The Rule:** Tests MUST NOT use `time.sleep()` to wait for asynchronous
operations. Use polling with explicit timeout and clear failure messages.

---

Related:
- [Test Suite Archaeology](06-test-suite-archaeology.md) on finding
  timing bugs in production tests
- [Port Binding TOCTOU](07-port-binding-toctou.md) on race conditions
  in test setup
