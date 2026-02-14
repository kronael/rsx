# Parallel Agent Audits: Finding 90 Bugs in 3 Hours

Manual code review is slow. One developer auditing 960 tests would take days. We used four parallel AI agents instead and found 90 bugs in 3 hours.

This is how we structured the audit, what we learned about agent-based code review, and why it works better than you'd expect.

## The Problem: Too Many Tests, Too Little Time

Our test suite had grown to 960 tests across 90 files. We knew there were issues (sporadic CI failures, local/CI discrepancies) but didn't know the scope.

Options:

1. **Manual audit**: One developer, 90 files, read every test. Estimate: 3-5 days.
2. **Wait for failures**: Let CI find bugs over time. Estimate: Weeks of sporadic failures.
3. **Ignore it**: Ship with flaky tests. Cost: Developer frustration, lost trust in CI.

We chose option 4: **Parallel agent audit**.

## The Setup: Four Agents, Four Categories

We divided the work by bug category, not by file:

**Agent A: Race Conditions**
- Port binding TOCTOU bugs
- Shared state in parallel tests
- Process cleanup races
- Files: All test files (broad scan)

**Agent B: Resource Leaks**
- Hardcoded paths (`./tmp`)
- Unclosed file handles
- Process orphans
- Files: WAL, archive, gateway tests

**Agent C: Timing Issues**
- `time.sleep()` in async waits
- Fixed timeouts
- Missing polling
- Files: Python integration tests

**Agent D: Incorrect Assertions**
- Testing wrong behavior
- Assertions that can't fail
- Missing edge cases
- Files: Core logic tests (book, matching, risk)

Each agent got a list of files and a checklist of patterns to find. They ran in parallel, independent of each other.

## Why Categories Beat File Division

We could have split 90 files across 4 agents (22 files each). That's simple division. But it's inefficient.

**File division problems:**

1. **Load imbalance**: Agent 1 gets small files (done in 20 min), Agent 4 gets complex files (2 hours).
2. **Context switching**: Each agent needs to understand all bug types for their files.
3. **Duplicate effort**: All agents learn the same patterns.

**Category division benefits:**

1. **Load balancing**: Each category has work across many files.
2. **Specialization**: Each agent becomes expert in one bug type.
3. **Cross-validation**: Multiple agents review same files from different angles.

Agent A looking for race conditions has different mental model than Agent D looking for assertion bugs. Both review `wal_test.rs`, both find different issues.

## The Agent Instructions

Clear instructions are critical. Vague tasks get vague results.

**Bad instruction:**
```
"Check tests for bugs"
```

**Good instruction:**
```
Find port binding races in test setup code. Pattern:
1. `UdpSocket::bind("127.0.0.1:<fixed_port>")`
2. `drop(sock)` or sock goes out of scope
3. Another component binds same port

For each instance:
- File and line number
- Current code snippet
- Proposed fix (ephemeral port via :0)
- Risk level (high/medium/low)
```

We gave each agent:
- Specific patterns to find (with code examples)
- Output format (structured list)
- Success criteria (number + severity)

## Real Agent Output (Race Conditions)

Agent A found 3 port binding races:

```
=== PORT BINDING RACES ===

1. rsx-dxs/tests/cmp_test.rs:45
   Risk: HIGH

   Current code:
   ```rust
   let sock = UdpSocket::bind("127.0.0.1:8080")?;
   let addr = sock.local_addr()?;
   drop(sock);
   let receiver = CmpReceiver::new(addr, ...)?;
   ```

   Issue: TOCTOU race between drop() and CmpReceiver::new()

   Fix: Use ephemeral port
   ```rust
   let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
   let addr = sock.local_addr().unwrap();
   drop(sock);
   let receiver = CmpReceiver::new(addr, ...)?;
   ```

2. [similar entries for other 2 instances]

Summary: 3 instances, all HIGH risk, parallel test failures likely
```

Structured, actionable, ready to fix.

## The Aggregate Results

After 3 hours (wall clock time, not agent time):

**Agent A (Race Conditions):** 15 bugs
- 3 port binding TOCTOU
- 8 process cleanup races
- 4 shared state issues

**Agent B (Resource Leaks):** 23 bugs
- 15 hardcoded `./tmp` paths
- 5 unclosed file handles
- 3 process orphans

**Agent C (Timing Issues):** 30 bugs
- 5 critical `time.sleep()` in waits
- 12 suboptimal timeouts
- 13 missing polling patterns

**Agent D (Incorrect Assertions):** 22 bugs
- 8 wrong behavior tests
- 14 documentation drift

**Total: 90 bugs**

Many files had multiple issues. One test file (`api_processes_test.py`) had race condition + timing issue + resource leak. The cross-category approach caught all three.

## What Agents Found That Humans Miss

**1. Consistency Across Codebase**

Humans get fatigued. First 10 files: thorough review. Files 30-40: starting to skim. Files 80-90: "looks fine."

Agents apply same rigor to file 90 as file 1.

**2. Pattern Recognition Across Distance**

Human sees port binding bug in `wal_test.rs`, might remember to check `cmp_test.rs`. Probably forgets to check `tls_test.rs` (different directory).

Agent A searches entire codebase for pattern, finds all 3 instances regardless of location.

**3. Cross-File Context**

`conftest.py` defines test fixtures. Tests in `test_api_processes.py` use them. Human reviewer might not connect process cleanup bug in fixture to sporadic failure in test.

Agent B sees both files, traces process lifecycle from fixture to test to teardown, identifies the gap.

## What Agents Miss That Humans Catch

**1. Domain-Specific Correctness**

Agent D flagged this assertion as "might be wrong":
```rust
assert_eq!(book.best_bid(), Some(Price(99_000)));
```

Human knows: "99_000 is correct for BTC-PERP at $99k." Agent doesn't have domain context.

**2. Intent vs Implementation**

Agent C said: "This sleep looks unnecessary."
```python
time.sleep(0.1)  # Let WebSocket establish
```

Human knows: WebSocket handshake isn't instant. 100ms sleep is fine, not a bug.

**3. Test Purpose**

Agent D: "This test asserts that error message contains 'Invalid'. Is this correct?"

Human reads test name `test_reject_invalid_price()`: "Yes, that's exactly what we're testing."

Agents are literal pattern matchers. Humans understand intent.

## The Human-Agent Review Loop

Best results came from hybrid approach:

1. **Agents scan** (3 hours, find patterns)
2. **Human triages** (1 hour, filter false positives)
3. **Agents fix** (2 hours, apply fixes)
4. **Human validates** (1 hour, test that fixes work)

Total: 7 hours to find and fix 90 bugs. Manual approach would take days.

**False positive rate:** ~15%

Agent flagged 106 potential bugs. Human review eliminated 16 as false positives (intentional behavior, domain-specific correctness).

That's acceptable. Would you rather review 106 flagged items or manually audit 960 tests hoping to find issues?

## The Fix Execution

Once bugs were identified, we used agents for fixes too:

**Agent A fixes:** Port binding races
```bash
# Task: Replace fixed ports with ephemeral in these 3 files
# Pattern: 127.0.0.1:<number> → 127.0.0.1:0
```

Agent made changes, we reviewed diffs, merged.

**Agent B fixes:** TempDir migration
```bash
# Task: Replace Path::new("./tmp") with TempDir::new()
# Files: [15 test files]
# Pattern: [code example]
```

**Result:** All fixes applied in 2 hours, compared to manual fix time of 6-8 hours.

## Cost Analysis

**Human-only approach:**
- Audit: 3-5 days (1 developer)
- Fix: 2-3 days
- Total: 5-8 days
- Cost: $4000-6400 (at $800/day)

**Agent-assisted approach:**
- Agent audit: 3 hours wall clock
- Human triage: 1 hour
- Agent fixes: 2 hours
- Human validation: 1 hour
- Total: 7 hours (1 day)
- Cost: ~$800 + agent compute ($20)

**ROI:** 5-8x time savings, similar cost savings.

But the real win isn't cost. It's that we actually did the audit. Without agents, we would have delayed it ("too time-consuming") until CI failures forced us.

## Reusable Patterns for Your Codebase

**1. The Race Condition Scanner**

```
Search for:
- Fixed ports in tests (UdpSocket::bind, TcpListener::bind)
- Shared state without locks (static mut, global DashMap)
- Process spawn without wait() in cleanup
- File operations in ./tmp without unique dirs
```

**2. The Resource Leak Scanner**

```
Search for:
- File::open without explicit close
- Process::spawn without process.wait()
- Hardcoded paths (./tmp, /tmp, ./data)
- Port allocations without ephemeral (0)
```

**3. The Timing Bug Scanner**

```
Search for:
- time.sleep() followed by assertion
- Fixed timeouts in async operations
- Missing retry/polling loops
- Hardcoded delays (> 1 second)
```

**4. The Assertion Bug Scanner**

```
Search for:
- Assertions that always pass (assert!(true))
- Assertions on wrong value
- Missing edge case tests
- Documentation mismatches (docstring vs code)
```

Give these patterns to an agent with your test directory. Let it run for an hour. Review results.

## Lessons for Agent-Based Code Review

**1. Specificity beats generality**

"Find bugs" is too vague. "Find port binding races where sock is dropped before component creation" is specific enough to produce results.

**2. Examples drive accuracy**

Show agent what good code looks like and what bad code looks like. Examples beat descriptions.

**3. Structured output enables automation**

Ask for "file, line, issue, fix, risk" format. You can parse this, sort by risk, auto-apply low-risk fixes.

**4. Multiple passes beat single pass**

Four agents with specific tasks found more bugs than one agent looking for "everything." Specialization works.

**5. Human validation is mandatory**

Agents find patterns. Humans understand intent. Never auto-apply agent suggestions without review.

## The Unexpected Benefit: Documentation

Agents produced structured lists of bugs. These became documentation:

```markdown
# Test Quality Checklist

Before PR merge, verify:
- [ ] No fixed ports (use ephemeral :0)
- [ ] No hardcoded ./tmp (use TempDir)
- [ ] No time.sleep() in async waits (use polling)
- [ ] Process cleanup uses wait()
- [ ] All assertions test actual behavior

See: [Agent A findings](./audit-results/race-conditions.md)
```

This checklist now lives in our `tests/README.md`. New tests get reviewed against it. Bugs don't recur.

## Would We Do It Again?

Yes. The audit found 90 bugs that would have caused CI pain for months. We fixed them in one day.

More importantly: it changed our test development culture. We now know what patterns cause problems. We avoid them proactively.

The agent audit wasn't just bug finding. It was team learning, compressed into 7 hours.

## Practical Starting Point

Don't audit everything at once. Start small:

**Week 1: Port binding audit**
- Pattern: Fixed ports in tests
- Agent: 1 hour to scan
- Human: 30 min to triage
- Fix: 1 hour
- Result: No more parallel test port conflicts

**Week 2: Resource leak audit**
- Pattern: Hardcoded ./tmp
- Agent: 1 hour to scan
- Fix: Replace with TempDir
- Result: No more test pollution

**Week 3: Timing audit**
- Pattern: time.sleep() in waits
- Agent: 1 hour to scan
- Fix: Convert to polling
- Result: Tests 4x faster

By week 4, you've eliminated the three most common test bugs and learned the agent review workflow.

## Summary

We used four parallel agents to audit 960 tests in 3 hours. Found 90 bugs across race conditions, resource leaks, timing issues, and incorrect assertions.

The approach:
- Divide by bug category, not by file
- Give specific patterns with examples
- Let agents scan, humans triage
- Use agents for fixes too
- Validate everything

Result: 5-8x faster than manual audit, 15% false positive rate, all real bugs fixed in one day.

Agents don't replace human code review. They scale pattern recognition, letting humans focus on intent and correctness.

The test suite is now CI-ready, parallel-safe, and non-flaky. That's worth 7 hours.

---

**The Rule:** Agent-based audits work when you know what patterns to find. Define the pattern, provide examples, let agents scan, humans validate. Specificity and structure beat generality and freestyle.

---

Related:
- [Test Suite Archaeology](06-test-suite-archaeology.md) for detailed bug categories
- [Port Binding TOCTOU](07-port-binding-toctou.md) for race condition details
- [TempDir Over ./tmp](08-tempdir-over-tmp.md) for resource cleanup patterns