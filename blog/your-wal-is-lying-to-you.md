# Your WAL Is Lying To You

You need durable event streaming for a matching engine. Every fill, every
cancel, every order insert—persisted, sequenced, replayable. Miss one and
positions diverge. Replay wrong and you double-fill.

The standard answer: Kafka. Or Postgres. Or some distributed log with
consensus.

That answer is wrong.

## The Problem With "Durable"

Most systems promise durability. Here's what that actually means:

```
Kafka:     write → broker → acks=all → 3 replicas fsync → ack
Latency:   2-10ms (network + disk + consensus)

Postgres:  write → WAL → fsync → ack
Latency:   500µs-2ms (disk + syscall overhead)

Our WAL:   write → buffer → 10ms tick → fsync → ack
Latency:   0ns (append is memcpy)
```

Wait. Zero nanoseconds for the write? Where's the durability?

It's in the flush. And the flush is on a timer.

## Bounded Data Loss Is Not Data Loss

Here's the shocking part: we accept up to 10ms of data loss.

```rust
pub fn append(
    &mut self,
    record_type: u16,
    payload: &[u8],
) -> io::Result<u64> {
    // No I/O. No syscall. Just memcpy.
    self.buf.extend_from_slice(&header.to_bytes());
    self.buf.extend_from_slice(payload);

    let seq = self.next_seq;
    self.next_seq += 1;
    Ok(seq)
}
```

Append is O(1). No disk. No network. No allocation. The matching engine
calls this in its hot loop. If append took 2ms (Kafka), you'd process 500
orders/sec. With memcpy, you process millions.

Every 10ms, a background flush writes the buffer to disk with `fsync`:

```rust
pub fn flush(&mut self) -> io::Result<()> {
    self.file.write_all(&self.buf)?;
    self.file.sync_all()?;  // this is the expensive part
    self.buf.clear();
    Ok(())
}
```

If the process crashes between flushes, you lose at most 10ms of events.
That's not a bug. That's the design.

Why is this okay? Because the matching engine is deterministic. Feed it the
same inputs, get the same outputs. On crash recovery, you replay from the
last fsynced position. Any orders that were in the 10ms gap? The client
retries. Dedup catches duplicates. The world continues.

## fsync Is The Only Thing That Matters

Everyone obsesses over serialization format. "Use protobuf!" "No,
FlatBuffers!" "No, Cap'n Proto!"

Serialization is 50-200ns. fsync is 200µs-2ms. That's 1000-10000x
difference.

The dominant cost is always the disk. Everything else is noise.

So we made two decisions:

1. **Serialize as cheaply as possible** — raw C structs, `memcpy` to buffer
2. **Batch fsync on a timer** — 10ms, amortize disk latency across hundreds
   of records

```rust
#[repr(C, align(64))]
struct FillRecord {
    seq: u64,
    ts_ns: u64,
    symbol_id: u32,
    maker_oid: u128,
    taker_oid: u128,
    px: i64,
    qty: i64,
    maker_side: u8,
    _pad1: [u8; 7],
}
```

64-byte aligned. Cache-line sized. No serialization step. The struct IS the
wire format IS the disk format. Write it to the buffer, write the buffer to
disk, stream it to consumers. Same bytes everywhere.

This is what the previous blog posts were building toward. Raw structs
aren't just fast for IPC—they're fast for persistence too.

## The WAL Is The Stream

Most architectures have separate systems: WAL for persistence, message
queue for streaming, maybe a replay service for recovery. Three systems,
three formats, three failure modes.

We have one:

```
Matching Engine → WalWriter → disk (fsync)
                           ↘ DxsReplay → consumers (gRPC stream)
```

The WAL file IS the stream. Consumers connect to a gRPC service that reads
the same WAL files and streams them. Historical replay? Read old files.
Live tail? Wait for flush notification, read new records.

```proto
service DxsReplay {
  rpc Stream(ReplayRequest) returns (stream WalBytes);
}
```

Consumer connects, says "give me everything from seq 42." Server opens the
WAL file containing seq 42, reads forward, streams records. When it catches
up to the writer's position, it sends a `CaughtUp` marker and switches to
live mode—waiting for flush notifications and streaming new records as
they arrive.

No Kafka. No ZooKeeper. No broker. Producer writes to disk. Consumer reads
from disk (or from the network, via gRPC). That's it.

## Backpressure Means Stalling The Matching Engine

This is the part that makes people uncomfortable.

If the in-memory buffer gets too full (consumer can't keep up, disk is
slow), the producer stalls:

```rust
if self.buf.len() > limit {
    return Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "wal buffer full, backpressure",
    ));
}
```

The matching engine stops processing orders. Dead stop. No new fills until
the buffer drains.

Why? Because the alternative is worse. If you drop events, positions
diverge. If you buffer without limit, you OOM. If you slow down gracefully,
you get priority inversion—slow consumer controls fast producer's latency.

Binary backpressure is honest. The system either works or it doesn't. No
degraded mode. No silent data loss. No "eventually consistent" hand-waving.

In practice, this never triggers. A 64MB buffer at 100K records/sec gives
you ~8 seconds of runway. If your disk can't keep up for 8 seconds, you
have bigger problems.

## File Rotation Is Garbage Collection

WAL files grow. You can't keep them forever (well, the recorder does, but
that's archival). The writer rotates at 64MB:

```
wal/1/1_active.wal           ← currently writing
wal/1/1_1_1000.wal           ← rotated, seqs 1-1000
wal/1/1_1001_2000.wal        ← rotated, seqs 1001-2000
```

Old files get garbage collected after the retention window (10 minutes).
The filename encodes the seq range, so finding the right file for replay is
a binary search on filenames. No index. No metadata database.

```rust
fn rotate(&mut self) -> io::Result<()> {
    // rename active → {first_seq}_{last_seq}.wal
    fs::rename(&active_path, &rotated_path)?;
    // open new active
    self.file = File::create(&active_path)?;
    self.gc()?;
    Ok(())
}
```

## CRC32: Trust But Verify

Every record has a CRC32 in the 16-byte header:

```
[version:2][type:2][len:4][stream_id:4][crc32:4] [payload...]
```

On read, if the CRC doesn't match, the reader truncates the stream at that
point. This handles crash recovery: a partial write from a crash leaves a
torn record at the end of the file. CRC detects it. Reader stops there.
Recovery replays from the last good record.

Unknown version? Fail fast, don't skip. A version mismatch means something
is deeply wrong. Silently skipping records is how you get phantom positions
and mystery P&L.

## The Recorder: Infinite Retention Without Complexity

The WAL writer retains 10 minutes. That's enough for crash recovery. But
you want to keep everything forever—audits, compliance, replay, debugging.

The recorder is a DxsConsumer that writes to daily archive files:

```
archive/1/1_2024-01-15.wal
archive/1/1_2024-01-16.wal
```

Same format. No transformation. Same 16-byte header + payload. You can
read archive files with the same WalReader. The recorder persists its tip
(last seen seq) to disk every 10ms. On crash, it resumes from the last
tip. Simple, boring, correct.

## Why Tokio Is Temporary

We use tonic gRPC (tokio) for the DxsReplay cold-path service.

Tokio is epoll-based. epoll means syscalls per I/O operation. For a
streaming service pushing 100K+ records/sec to multiple consumers, that's a
lot of syscalls.

The plan:

1. **Now:** tonic/tokio. It works. Ship it.
2. **Next:** monoio with io_uring. Zero-copy I/O, batched submissions,
   kernel does the work.
3. **Later:** userspace networking. NIC writes directly to ring buffers.
   No kernel involvement at all.

The architecture supports this because the WAL is the interface, not the
transport. Swap the tokio runtime for monoio, the WAL files don't change,
the consumers don't change (they just reconnect), and the matching engine
doesn't know the difference.

## What We Didn't Build

- **Consensus.** Single writer per stream. No Raft, no Paxos, no split brain.
- **Replication.** The recorder IS the replica. It's a consumer that writes
  to a different disk.
- **Compression.** 64-byte aligned structs don't compress well, and
  decompression adds latency. If disk space matters, the recorder can
  compress archives offline.
- **Encryption.** Internal network. If someone's on your internal network
  reading your WAL stream, encryption won't save you.
- **Schema registry.** Version field in the header. All producers and
  consumers deploy together. If they don't, the version check fails fast.

Every feature you don't build is a feature that can't break.

## Performance

```
Operation                    Target        Actual
--------------------------------------------------
WAL append (in-memory)       <200ns        ~50ns
WAL flush (fsync, 64KB)      <1ms          ~300µs
WAL read (sequential)        >500 MB/s     ~800 MB/s
Replay 100K records          <1s           ~200ms
Recorder sustained write     >100K rec/s   ~500K rec/s
```

The bottleneck is fsync. Everything else is memcpy.

## The Uncomfortable Truth

Most "event streaming" systems are distributed databases pretending to be
message queues. They solve problems you don't have (multi-datacenter
replication, exactly-once across partitions, consumer group rebalancing)
and add latency you can't afford.

A WAL is a file you append to and fsync. A stream is a reader that tails
that file. A replay service is a reader that starts from the beginning.

That's it. That's the whole system.

The hard part isn't the architecture. The hard part is resisting the urge
to add complexity.
