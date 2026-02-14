# Test Suite Archaeology: Finding 90 Bugs in Production-Ready Code

We thought our test suite was solid. 960 tests passing, 100% spec
coverage, all features implemented. Then we ran a comprehensive audit
and found 90 bugs lurking in the tests themselves.

This is the story of what we found and why test quality matters as
much as code quality.

## The Audit: Parallel Subagents

We split the test suite across four parallel agents, each auditing
20-30 test files for common failure patterns:

1. Race conditions (timing assumptions, shared state)
2. Resource leaks (ports, files, processes)
3. Incorrect assertions (testing wrong behavior)
4. Flakiness sources (sleep-based timing)

Three hours later: **90 bugs across all categories**.

Most shocking: the code worked. The tests passed. But they were
fragile, slow, and would fail under CI pressure or parallel execution.

## Category 1: The Port Binding Race (TOCTOU)

Found in three separate test files, same pattern:

```rust
// BEFORE: Bind, drop, hope no one steals the port
let sock = UdpSocket::bind("127.0.0.1:8080")?;
let addr = sock.local_addr()?;
drop(sock);  // Release port

// Now create real component
let receiver = CmpReceiver::new(addr, ...)?;  // RACE HERE
```

The bug: Between `drop(sock)` and `CmpReceiver::new()`, another test
(running in parallel) can steal port 8080. Classic Time-Of-Check-Time-
Of-Use (TOCTOU).

The fix was counterintuitive. Instead of fixed ports, use ephemeral:

```rust
// AFTER: OS assigns unique port per test
let recv_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
let recv_addr = recv_sock.local_addr().unwrap();
drop(recv_sock);
```

Port 0 means "give me any available port." Every test gets a unique
port. Parallel execution works. No coordination needed.

**Lesson:** Fixed ports in tests create false dependencies. Ephemeral
ports eliminate an entire class of races.

## Category 2: Resource Leaks (Hardcoded Paths)

Found in 15 WAL and archive tests:

```rust
// BEFORE: Hardcoded path accumulates garbage
let mut writer = WalWriter::new(1, Path::new("./tmp"), ...)?;
```

Problem: Each test writes to `./tmp`. Files accumulate. Tests pollute
each other. Parallel execution conflicts.

Fix: Use Rust's `TempDir` from the `tempfile` crate:

```rust
// AFTER: Unique directory per test, auto-cleanup
let tmp = TempDir::new().unwrap();
let mut writer = WalWriter::new(1, tmp.path(), ...)?;
// TempDir drops at end of scope, deletes everything
```

`TempDir` creates a unique directory, returns a path, and deletes
everything when dropped. Zero manual cleanup. Zero cross-test
pollution.

**Lesson:** Hardcoded paths like `./tmp` are test debt. Use `TempDir`
everywhere.

## Category 3: Process Cleanup Races

Found in Python test fixtures (`conftest.py`):

```python
# BEFORE: Kill process, move on
os.kill(proc.pid, signal.SIGTERM)
# Next test starts immediately

# AFTER: Wait for death
os.kill(proc.pid, signal.SIGTERM)
try:
    proc.wait(timeout=5)
except subprocess.TimeoutExpired:
    os.kill(proc.pid, signal.SIGKILL)
```

The bug: SIGTERM is async. The process gets the signal but hasn't
exited yet. The next test starts, tries to bind the same port, fails
with "address already in use."

Adding `proc.wait()` makes the cleanup synchronous. Test doesn't
proceed until the process is actually dead.

**Lesson:** Process cleanup isn't done when you send the signal.
It's done when the process exits.

## Category 4: The Hidden Cost of time.sleep()

Found in 5 locations in Python tests:

```python
# BEFORE: Hope 2 seconds is enough
client.post("/api/processes/all/start")
time.sleep(2)
resp = client.get("/api/processes")
assert len(resp.json()) > 0

# AFTER: Poll until ready
client.post("/api/processes/all/start")
for _ in range(50):  # 5 seconds max
    resp = client.get("/api/processes")
    if len(resp.json()) > 0:
        break
    time.sleep(0.1)
else:
    pytest.fail("Processes didn't start")
```

Why polling wins:

- **Faster on fast machines**: If startup takes 200ms, polling exits
  in 200ms. Sleep always waits 2 seconds.
- **More reliable on slow machines**: If startup takes 3 seconds,
  polling eventually succeeds. Sleep fails.
- **Clearer intent**: "Wait until condition X" vs "hope X happens in
  2 seconds."

**Lesson:** `time.sleep()` in tests means you don't know how long
something takes. Polling means you know what you're waiting for.

## Category 5: Incorrect Test Assertions

Found in migration and dedup tests:

```rust
// BEFORE: Assert no-op migration succeeded
assert_eq!(book.active_levels(), 0);  // WRONG: levels exist!

// AFTER: Verify migration actually migrated
assert!(book.active_levels() > 0);
assert_eq!(book.best_bid(), Some(expected_price));
```

The test was checking that a migration with orders in the book
resulted in zero levels. That's backwards - migration should preserve
orders, not delete them.

How did this pass? The test name was vague ("test_migration"), the
assertion was wrong, and no one questioned it because it was green.

**Lesson:** Green tests aren't correct tests. Read assertions
carefully. Does "success" mean what you think it means?

## The Build System Limit Discovery

Mid-audit, we hit a practical limit: parallel cargo builds consumed
90GB of disk space. The CI environment has 100GB total. We adapted:

1. Tried parallel workers (failed - disk pressure)
2. Switched to direct fixes + background agents
3. Used `cargo clean` as emergency relief

**Lesson:** Build parallelism has resource limits. Know your
constraints before scaling.

## Audit Results Summary

**Total bugs found:** 90

- Port binding races: 3
- Hardcoded paths: 15
- Process cleanup races: 2
- Timing-based flakiness: 5
- Incorrect assertions: 8
- Missing error checks: 12
- Resource leaks: 23
- Documentation drift: 22

**Time to fix:** 6 hours (automation + 4 parallel agents)

**Result:** All 960 tests now non-flaky, CI-ready, parallel-safe.

## Actionable Takeaways

1. **Audit your tests like production code.** Tests have bugs too.
   They're just harder to see because they're green.

2. **Use TempDir everywhere.** Hardcoded paths accumulate technical
   debt faster than you realize.

3. **Ephemeral ports eliminate races.** Port 0 is your friend in tests.

4. **Poll, don't sleep.** If you don't know the condition, you don't
   understand the test.

5. **Process cleanup is async.** Wait for death before proceeding.

6. **Green doesn't mean correct.** Read your assertions. Would they
   catch the bug you're testing for?

7. **Parallel agents scale audit work.** Four agents reviewing 90
   files in 3 hours beats one human reviewing for a week.

The test suite now runs in CI without flakiness. Parallel execution
works. Local and CI results match. That's the real test of test
quality.

---

Related: [Your WAL Is Lying To You](your-wal-is-lying-to-you.md) on
production invariant testing.
