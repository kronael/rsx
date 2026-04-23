# Backpressure or Death: No Silent Drops

When the buffer fills, the system stalls. Never drop data silently.

## The Problem

Every queue-based system has the same question: what happens when the
buffer is full?

Option A: Drop the message, log an error, keep going.
Option B: Stall the producer until space is available.

Most systems choose A. It's "more available." Producers don't block.
Throughput stays high.

**We chose B. Always.**

## The Philosophy

Fills are sacred. Orders are ephemeral. If we lose a fill, user balances
diverge forever. If we lose an order, user resubmits.

Silent drops are invisible data loss:
- Prometheus counter increments
- Log line scrolls by
- Monitoring alerts... maybe
- User files support ticket 3 days later

Backpressure is visible:
- Producer blocks immediately
- Latency spikes in p99 metrics
- Load balancer fails health check
- On-call gets paged

**Visible failures are fixable. Silent failures are lawsuits.**

## How It Works

WAL writer has a buffer. When full, `append()` returns `WouldBlock`.

```rust
// rsx-dxs/src/wal.rs
pub fn append<T: CmpRecord>(
    &mut self,
    record: &mut T,
) -> io::Result<u64> {
    let payload_len = std::mem::size_of::<T>();

    if self.flush_stalled {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "flush stalled, backpressure active",
        ));
    }

    // Max buffer = max(2 * file_size, 256KB)
    let max_buf_size = std::cmp::max(
        2 * self.max_file_size as usize,
        256 * 1024,
    );

    if self.buf.len() + 16 + payload_len > max_buf_size {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "buffer full",
        ));
    }

    // Assign seq, append to buffer
    record.set_seq(self.next_seq);
    let header = WalHeader {
        stream_id: self.stream_id,
        record_type: T::record_type(),
        seq: self.next_seq,
        payload_len: payload_len as u16,
        crc32: compute_crc32(as_bytes(record)),
    };

    self.buf.extend_from_slice(as_bytes(&header));
    self.buf.extend_from_slice(as_bytes(record));

    self.next_seq += 1;
    self.records_since_flush += 1;

    Ok(self.next_seq - 1)
}
```

Matching engine handles it:

```rust
// Pseudo-code from rsx-matching/src/main.rs
loop {
    // Poll for incoming orders
    if let Ok(order_msg) = cmp_rx.try_recv() {
        let mut order = parse_order(&order_msg);
        process_new_order(&mut book, &mut order);

        // Write events to WAL
        match write_events_to_wal(&mut wal, &book.events[..book.event_len]) {
            Ok(_) => {
                // Success: send fills to risk
                for event in &book.events[..book.event_len] {
                    if let Event::Fill { .. } = event {
                        cmp_tx.send(event).unwrap();
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                // Backpressure: stall until flush completes
                warn!("wal backpressure, stalling");
                wal.flush()?;  // Block until fsync
                // Retry append
                write_events_to_wal(&mut wal, &book.events[..book.event_len])?;
            }
            Err(e) => {
                // Fatal: can't WAL = can't process orders
                panic!("wal write failed: {}", e);
            }
        }
    }

    // Flush if 10ms elapsed or 1000 records buffered
    flush_if_due(&mut wal, &mut last_flush)?;
}
```

SPSC ring (intra-process) uses bounded queue:

```rust
// rsx-matching tile to marketdata tile
let (mut prod, mut cons) = rtrb::RingBuffer::<Event>::new(10_000);

// Producer (matching engine)
match prod.push(fill_event) {
    Ok(_) => { /* sent */ }
    Err(_) => {
        // Ring full: marketdata consumer is slow
        // Spin until space available
        loop {
            std::hint::spin_loop();
            if prod.push(fill_event).is_ok() {
                break;
            }
        }
    }
}
```

No `try_push` + drop. No timeout. **Producer spins until consumer
catches up.**

## Small Buffers Fail Fast

WAL buffer: `max(2 × file_size, 256KB)`. For 64MB files, that's 128MB.
For 1KB test files, that's 256KB.

Why so small? **Because backpressure should happen early.**

Large buffer: 10GB RAM. Producer writes 100M events before backpressure.
By the time you notice, 10s of latency have accumulated. P99 is 5s.
Users are rage-tweeting.

Small buffer: 256KB. Producer writes 3000 events before backpressure
(~3ms at 1M events/sec). P99 spikes to 15ms. Monitoring alerts. On-call
fixes before users notice.

**Small buffers surface problems faster.**

## Tests Prove It

```rust
// rsx-dxs/tests/wal_test.rs
#[test]
fn writer_backpressure_stalls() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), None, 4096, 600_000_000_000,
    ).unwrap();

    let mut hit_backpressure = false;
    for i in 0..5000 {
        let mut fill = make_fill(i);
        match writer.append(&mut fill) {
            Ok(_) => continue,
            Err(e) => {
                assert_eq!(e.kind(), io::ErrorKind::WouldBlock);
                hit_backpressure = true;
                break;
            }
        }
    }

    assert!(hit_backpressure, "should have hit backpressure");
}
```

This test fails if:
- Buffer size is unlimited (never returns WouldBlock)
- Append silently drops when full
- Error is wrong kind (InvalidData instead of WouldBlock)

Flush lag test:

```rust
#[test]
fn flush_lag_triggers_backpressure() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
    ).unwrap();

    // Simulate slow disk: append without flushing
    for i in 0..1000 {
        writer.append(&mut make_fill(i)).unwrap();
    }

    // Mark flush as stalled (simulates fsync taking >10ms)
    writer.flush_stalled = true;

    // Next append should fail with backpressure
    let result = writer.append(&mut make_fill(1001));
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WouldBlock);
}
```

## The Cost

Backpressure means the matching engine stalls when WAL is slow. If
fsync takes 50ms (slow SSD), matching pauses for 50ms.

During that 50ms:
- No new orders accepted
- Gateway queues fill
- Users see "service unavailable"

This is **correct behavior**.

If we kept accepting orders during slow fsync:
- Orders pile up in memory (10,000+ orders)
- Crash during backlog = lose all queued orders
- Users get "order accepted" but order never executed
- Support tickets: "I placed an order but it disappeared"

Better: reject new orders at the gateway. User retries. Order executes
or rejects cleanly. No phantom orders.

## Production Scenarios

**Scenario 1: Slow Disk**
- Fsync takes 50ms (expected: 1ms)
- WAL buffer fills in 10ms
- Backpressure after 10ms
- Matching stalls for 40ms
- Gateway returns 503 Service Unavailable
- Monitoring alerts: "p99 latency 50ms"
- On-call investigates disk I/O
- Fix: replace SSD

**Scenario 2: Consumer Lag**
- Marketdata consumer is slow (100ms per event, expected: 1μs)
- SPSC ring fills in 10ms (10,000 event buffer)
- Matching engine stalls on ring push
- No fills sent to users
- Gateway times out waiting for responses
- Monitoring alerts: "marketdata lag 5s"
- Fix: kill slow consumer, restart, replay from WAL

**Scenario 3: Network Partition**
- Risk engine unreachable (CMP/UDP packets dropped)
- Gateway buffers fills waiting for risk ACK
- Buffer full after 100ms
- Gateway stops accepting orders
- Load balancer marks gateway unhealthy
- Traffic routes to healthy gateway
- Monitoring alerts: "gateway offline"
- Fix: network team investigates partition

All three scenarios: **visible failures, immediate alerts, no data loss.**

## Why It Matters

Silent drops are a time bomb. System looks healthy (low latency, high
throughput), but data is leaking. Users discover the problem days later
when balances don't match.

Backpressure is honest failure:
- Latency spikes immediately
- Monitoring alerts immediately
- On-call investigates immediately
- Fix deployed before users notice

The SLA is: **we never lose your fill.** We don't promise we'll always
be fast. We promise if we accept your order, the fill is durable.

Backpressure enforces that promise.

## Key Takeaways

- **Never drop data silently**: WouldBlock > silent drop
- **Small buffers fail fast**: 256KB buffer surfaces problems in
  milliseconds, not minutes
- **Stall > corruption**: Better to pause than to lose fills
- **Visible failures**: Latency spike alerts immediately, dropped
  messages alert... eventually
- **Test backpressure**: Assert `WouldBlock` happens when buffer fills

Every queue in RSX has bounded capacity. WAL buffer, SPSC rings, CMP
send queue, gateway ingress queue. All bounded. All return WouldBlock
when full.

When the system can't keep up, it stalls. It doesn't lie.

## Target Audience

Distributed systems engineers tired of debugging silent data loss.
Anyone building financial systems where losing a message is
unacceptable. SREs who've been paged for "phantom orders" or "missing
fills."

## See Also

- `specs/2/48-wal.md` - WAL backpressure rules
- `specs/2/6-consistency.md` - Event ordering guarantees
- `rsx-dxs/src/wal.rs` - WAL writer with backpressure
- `rsx-dxs/tests/wal_test.rs` - Backpressure tests
- `blog/04-wal-and-recovery.md` - WAL durability guarantees
