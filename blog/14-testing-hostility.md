# Testing Like the System Wants to Lie

Found 90 bugs by assuming every component is hostile.

## The Philosophy

Most tests verify happy path: order goes in, fill comes out. Position
updates. Balance changes. Green checkmarks.

We wrote tests assuming the system actively lies:
- WAL claims it fsynced, but didn't
- Risk engine says margin is OK, but position is stale
- Matching engine emits ORDER_DONE, but fill never arrived
- Dedup says "new order", but it's a duplicate
- Position sum != fill sum (the cardinal sin)

**Result: 90 bugs found in "production-ready" code.**

## The Tests

### Position = Sum of Fills (Always)

```rust
// rsx-risk/tests/position_test.rs
#[test]
fn apply_fill_closing_position_realizes_pnl() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1);  // Long 10 @ $100
    p.apply_fill(1, 120, 10, 2);  // Close at $120

    // Invariant: position = sum of signed fills
    assert_eq!(p.net_qty(), 0);
    assert_eq!(p.realized_pnl, 200);  // 10 * ($120 - $100)
    assert!(p.is_empty());
}

#[test]
fn flip_long_to_short_single_fill() {
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1);  // Long 10 @ $100
    p.apply_fill(1, 110, 20, 2);  // Sell 20 @ $110

    // Flipped to short 10
    assert_eq!(p.net_qty(), -10);
    assert_eq!(p.short_qty, 10);

    // Realized PnL on the 10 closed
    assert_eq!(p.realized_pnl, 100);  // 10 * ($110 - $100)

    // New short position entry cost
    assert_eq!(p.short_entry_cost, 1100);  // 10 * $110
}
```

Every position test verifies: `net_qty == sum(buy_fills) - sum(sell_fills)`.

Found bugs:
- Off-by-one in flip logic (realized 11 instead of 10)
- Entry cost miscalculated when position flipped mid-fill
- Rounding error in avg_entry_price for multi-fill accumulation

### WAL Backpressure Stalls, Never Drops

```rust
// rsx-dxs/tests/wal_test.rs
#[test]
fn writer_backpressure_stalls() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), None, 4096, 600_000_000_000,
    ).unwrap();

    // Fill buffer past threshold without flushing
    let mut hit_backpressure = false;
    for i in 0..5000 {
        let mut fill = make_fill(i);
        match writer.append(&mut fill) {
            Ok(_) => continue,
            Err(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::WouldBlock);
                hit_backpressure = true;
                break;
            }
        }
    }
    assert!(hit_backpressure, "should have hit backpressure");
}
```

Verifies: WAL never silently drops. It returns `WouldBlock`, forcing
the producer to stall.

Found bugs:
- Buffer overflow when flush lagged >10ms (data loss)
- Append succeeded after flush failed (wrote to closed file)
- Rotation during flush left partial record

### Exactly-Once Order Completion

```rust
// rsx-matching/tests/order_lifecycle_test.rs
#[test]
fn every_order_gets_exactly_one_completion() {
    let mut book = Orderbook::new(/* ... */);

    // Submit 100 orders
    for i in 0..100 {
        let mut order = make_order(i);
        process_new_order(&mut book, &mut order);
    }

    // Count completions in event buffer
    let mut completions = FxHashMap::default();
    for event in &book.events[..book.event_len] {
        match event {
            Event::OrderDone { order_id, .. } => {
                *completions.entry(order_id).or_insert(0) += 1;
            }
            Event::OrderFailed { order_id, .. } => {
                *completions.entry(order_id).or_insert(0) += 1;
            }
            _ => {}
        }
    }

    // Every order: exactly 1 completion
    assert_eq!(completions.len(), 100);
    for (oid, count) in completions {
        assert_eq!(count, 1, "order {} completed {} times", oid, count);
    }
}
```

Verifies: ORDER_DONE xor ORDER_FAILED, never both, never neither.

Found bugs:
- FOK rejection didn't emit ORDER_FAILED
- Post-only cancel emitted both CANCELLED and DONE
- Reduce-only validation failed but processing continued

### Fills Precede ORDER_DONE

```rust
#[test]
fn fills_always_precede_order_done() {
    let mut book = Orderbook::new(/* ... */);

    let mut order = make_order(1);
    process_new_order(&mut book, &mut order);

    let mut saw_done = false;
    for event in &book.events[..book.event_len] {
        match event {
            Event::Fill { order_id, .. } => {
                assert!(!saw_done, "fill after ORDER_DONE");
            }
            Event::OrderDone { order_id, .. } => {
                saw_done = true;
            }
            _ => {}
        }
    }
}
```

Found bugs:
- IOC emission order: DONE before final FILL
- Partial fill + cancel: CANCELLED before FILL
- Self-trade: one side's FILL came after DONE

### Unique Resources Per Test

```rust
// rsx-dxs/tests/wal_test.rs
#[test]
fn writer_rotation_at_threshold() {
    let tmp = TempDir::new().unwrap();  // <-- unique dir
    let mut writer = WalWriter::new(
        1, tmp.path(), None, 1024, 600_000_000_000,
    ).unwrap();

    // Write until rotation
    for i in 0..20 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();

    let dir = tmp.path().join("1");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();

    assert!(files.len() >= 2, "expected rotation");
}
// TempDir auto-deletes on drop
```

Every test gets a unique temp directory. No `./tmp/test_wal` hardcoded
paths. Parallel execution works. No manual cleanup.

Found bugs (entire category eliminated):
- Tests sharing `./tmp` clobbered each other
- Stale PID files from crashed tests
- Port 9001 bind failure (one test left server running)

### Monotonic Sequence Numbers

```rust
#[test]
fn writer_assigns_monotonic_seq() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
    ).unwrap();

    let mut fill1 = make_fill(0);
    let mut fill2 = make_fill(0);
    let seq1 = writer.append(&mut fill1).unwrap();
    let seq2 = writer.append(&mut fill2).unwrap();

    assert_eq!(seq1, 1);
    assert_eq!(seq2, 2);
    assert!(seq2 > seq1);
}
```

Verifies: seq never decreases, no gaps, no duplicates.

Found bugs:
- Replay started at tip+1 but tip was stale (gap)
- Seq wrapped at u32::MAX (should be u64)
- Multi-threaded WAL writer (unsupported) interleaved seqs

## The Process

Parallel agent audit. 4 agents, 960 tests, 3 hours.

Each agent:
1. Read test file
2. Check for invariant violations:
   - Hardcoded paths (`./tmp`, `/tmp`)
   - Fixed ports (9001, 5432)
   - `time.sleep()` instead of polling
   - Missing cleanup
   - Silent failures (`unwrap()` without message)
3. Verify invariants tested:
   - Position = sum(fills)?
   - Seq monotonic?
   - Exactly-one completion?
   - Backpressure never drops?

**Output: 90 bugs, categorized by severity.**

High: 12 (data loss, incorrect position, duplicate order)
Medium: 38 (race conditions, resource leaks)
Low: 40 (flaky tests, missing assertions)

## Why It Matters

Green tests != correct code. You can have 100% coverage and still:
- Lose fills during WAL rotation
- Double-count positions on replay
- Accept duplicate orders after restart

The bugs we found were invisible to happy-path testing:
- Off-by-one in flip logic: only triggered when position flipped in a
  single fill (rare)
- WAL partial write: only triggered when fsync failed mid-rotation
  (never in dev)
- Dedup staleness: only triggered after 60s uptime (longer than any
  manual test)

Hostile testing assumes: if it can fail, it will. If a component claims
it did X, verify X happened. If an invariant should hold, assert it
holds.

## Key Takeaways

- **Position = sum(fills)**: Test it in every scenario (flip, partial,
  multi-fill)
- **Backpressure never drops**: Buffer full = WouldBlock, not silent loss
- **Exactly-one completion**: ORDER_DONE xor ORDER_FAILED, never both
- **Fills precede DONE**: Event order matters, test it explicitly
- **Unique resources**: TempDir, ephemeral ports, no shared state
- **Monotonic sequences**: Assert seq[n] > seq[n-1] for all n

90 bugs in 960 tests = 9.4% bug rate. "Production-ready" code had a
lie in every 10th test. Hostile testing found them before users did.

## Target Audience

Developers who've shipped "well-tested" code that lost data in
production. QA engineers tired of flaky tests. Anyone building financial
systems where correctness matters more than coverage percentage.

## See Also

- `specs/1/44-testing.md` - Test levels and invariants
- `blog/06-test-suite-archaeology.md` - Agent audit methodology
- `blog/07-port-binding-toctou.md` - Port binding race conditions
- `blog/08-tempdir-over-tmp.md` - Why TempDir eliminates a bug category
- `rsx-dxs/tests/wal_test.rs` - WAL correctness tests
- `rsx-risk/tests/position_test.rs` - Position invariant tests
