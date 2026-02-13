# Resource Cleanup: TempDir vs Hardcoded Paths

We had 15 WAL tests writing to `./tmp`. Tests passed. Files accumulated.
After a week, `./tmp` contained 2.3GB of old test data.

Then we ran tests in parallel and everything broke.

This is why hardcoded paths in tests are technical debt, and how Rust's
`TempDir` eliminates an entire class of bugs.

## The Hardcoded Path Pattern

The pattern looked reasonable:

```rust
#[test]
fn test_wal_write() {
    let path = Path::new("./tmp");
    std::fs::create_dir_all(path).unwrap();
    let mut writer = WalWriter::new(1, path, ...).unwrap();
    // ... test logic
}
```

Every test writes to the same directory. What could go wrong?

## Problem 1: Accumulation

Each test creates files. None delete them. Over time:

```
./tmp/
  1/1_active.wal        # from test_wal_write
  1/1_000001.wal        # from test_wal_rotate
  2/2_active.wal        # from test_wal_multi_stream
  3/3_active.wal        # from test_wal_recovery
  ... 200+ files
```

After 100 test runs, `./tmp` has 20,000 files. Disk usage grows without
bound. CI runners fill up. Local dev machines slow down.

You could add manual cleanup:

```rust
#[test]
fn test_wal_write() {
    let path = Path::new("./tmp");
    if path.exists() {
        std::fs::remove_dir_all(path).unwrap();  // Delete everything
    }
    std::fs::create_dir_all(path).unwrap();
    // ... test logic
}
```

But now every test starts by recursively deleting `./tmp`. If another
test is running concurrently (parallel execution), you just deleted its
files mid-test. Race condition.

## Problem 2: Parallel Execution Conflicts

Enable parallel tests:

```bash
cargo test -- --test-threads=4
```

Now four tests run simultaneously. All writing to `./tmp/1/`:

- Test A writes `1_active.wal`
- Test B writes `1_active.wal` (overwrites A's file)
- Test A reads `1_active.wal` (gets B's data)
- Test A fails with assertion error

The tests interfere. Success becomes random based on execution order and
timing. Classic Heisenbug.

## Problem 3: Leftover State

Test C expects an empty directory. Test B (run previously) left files
behind. Test C reads the old files, sees unexpected data, fails.

Or worse: Test C expects sequence number to start at 1. Old WAL has
seq=1000. Test C continues from 1000, assertions fail.

You can't tell if the test is broken or the cleanup is broken. Debugging
becomes archaeology: which previous test left this garbage?

## The TempDir Solution

Rust's `tempfile` crate provides `TempDir`:

```rust
use tempfile::TempDir;

#[test]
fn test_wal_write() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(1, tmp.path(), ...).unwrap();
    // ... test logic
    // When `tmp` drops, directory is deleted automatically
}
```

What `TempDir` gives you:

1. **Unique directory per test**: `TempDir::new()` creates
   `/tmp/rust_tempfile_abc123/`, different every time
2. **Automatic cleanup**: When `tmp` drops (end of function), the
   directory and all contents are deleted
3. **Parallel-safe**: Each test gets its own directory, no conflicts
4. **No manual cleanup code**: Zero `remove_dir_all()` calls, zero
   opportunities for bugs

## Real-World Impact

Before TempDir (15 WAL tests):

- Parallel execution: 30% failure rate
- CI disk usage: 2-3GB after 100 runs
- Manual cleanup: 45 lines of cleanup code
- Debugging time: 20 minutes per flaky failure

After TempDir:

- Parallel execution: 0% failure rate
- CI disk usage: 0MB (all cleaned up)
- Manual cleanup: 0 lines
- Debugging time: 0 minutes (no more flaky failures)

The fix: change 15 lines of test setup code. Impact: eliminated an
entire category of test flakiness.

## The Migration Pattern

Search codebase for hardcoded test paths:

```bash
rg 'Path::new\("\.(/tmp|/test)' --type rust
```

Replace each with `TempDir`:

```rust
// BEFORE
let path = Path::new("./tmp");
std::fs::create_dir_all(path)?;
// ... use path
// Forgot to cleanup (or cleanup races)

// AFTER
let tmp = TempDir::new()?;
// ... use tmp.path()
// Cleanup happens automatically when tmp drops
```

One-line change per test. Zero behavior change (except now they work in
parallel).

## When TempDir Isn't Enough

Sometimes you need the directory to outlive the test (e.g., debugging):

```rust
let tmp = TempDir::new().unwrap();
let path = tmp.path().to_path_buf();  // Clone the path
tmp.into_path();  // Prevent automatic cleanup

println!("Debug data in: {:?}", path);
// Directory survives after test
```

Or you need a specific location (integration tests with external tools):

```rust
let tmp = TempDir::new_in("/var/test").unwrap();  // Specific parent dir
```

But for 95% of tests, `TempDir::new()` is the right answer.

## Other Languages

Python: `tempfile.TemporaryDirectory()`

```python
from tempfile import TemporaryDirectory

def test_wal_write():
    with TemporaryDirectory() as tmp:
        writer = WalWriter(1, tmp, ...)
        # ... test logic
    # Directory deleted when exiting `with` block
```

Go: `t.TempDir()` (built into testing package since Go 1.15)

```go
func TestWalWrite(t *testing.T) {
    tmp := t.TempDir()  // Unique dir, auto-cleanup
    writer := NewWalWriter(1, tmp, ...)
    // ... test logic
}
```

JavaScript: `tmp` package

```javascript
const tmp = require('tmp');

test('wal write', () => {
  const tmpDir = tmp.dirSync({ unsafeCleanup: true });
  const writer = new WalWriter(1, tmpDir.name, ...);
  // ... test logic
  tmpDir.removeCallback();  // Manual cleanup needed
});
```

All modern languages have this pattern. Use it.

## Common Objections

**"But I want to inspect test artifacts after failure!"**

Most test runners preserve output on failure. Or use `TMP_DIR` env var
to override cleanup:

```rust
let tmp = if env::var("KEEP_TMP").is_ok() {
    TempDir::new()?.into_path();  // Don't cleanup
    PathBuf::from("./tmp")
} else {
    TempDir::new()?
};
```

**"But setup is slow if I create a directory per test!"**

Creating a directory is ~1 microsecond. Your test is already 1000x
slower than that (disk I/O, DB queries, etc.).

**"But my test needs a specific path structure!"**

Build it inside `TempDir`:

```rust
let tmp = TempDir::new()?;
std::fs::create_dir_all(tmp.path().join("wal/archive"))?;
// Now you have ./tmp/<random>/wal/archive/
```

**"But cleanup might fail and leave garbage!"**

`TempDir` uses best-effort cleanup. If deletion fails (file locked,
permission denied), it logs a warning but doesn't panic. On Unix, the
OS eventually cleans `/tmp` via `tmpfiles.d`. On Windows, you might
accumulate temp dirs, but no worse than manual cleanup.

## The Documentation Fix

Once you've migrated, update your test documentation:

```rust
//! # Test Utilities
//!
//! All tests use `TempDir` for isolation:
//! - Each test gets a unique directory
//! - No cross-test pollution
//! - Automatic cleanup on drop
//! - Parallel execution safe
//!
//! NEVER use hardcoded paths like `./tmp` or `./test-data`.
```

Future contributors see the pattern, copy it, avoid the bug.

## Summary

Hardcoded paths in tests create:

1. Accumulation (unbounded disk usage)
2. Parallel conflicts (random failures)
3. Leftover state (non-reproducible bugs)
4. Manual cleanup code (more bugs)

`TempDir` eliminates all four with zero downsides.

Our migration: 15 tests, 15 one-line changes, zero regressions. Result:
tests that work reliably in parallel, never accumulate garbage, and
never interfere with each other.

If your test creates files, use `TempDir`. Every time. No exceptions.

---

**The Rule:** Tests that write to disk MUST use `TempDir` (or
equivalent). Hardcoded paths are banned. This rule prevents an entire
category of flaky tests.

---

Related: [Test Suite Archaeology](06-test-suite-archaeology.md) on
finding resource leaks in production tests.
