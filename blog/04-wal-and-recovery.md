# WAL-Based Persistence and Recovery

In RSX, fills are sacred. A fill represents an irreversible
transfer of risk between two parties. Losing a fill means
incorrect positions, incorrect margin, and potential exchange
insolvency. This post covers how we ensure fills survive any
single-component failure with 0ms data loss.

## The WAL Format

Every event produced by the matching engine is written to a
Write-Ahead Log (WAL). The format is deliberately minimal:

```
struct WalHeader {       // 16 bytes
    record_type: u16,    // what kind of event
    len: u16,            // payload length
    crc32: u32,          // integrity check
    _reserved: [u8; 8],  // future use
}
```

The payload follows the header immediately. All payloads are
`#[repr(C, align(64))]` structs with explicit padding fields
and little-endian byte order. The alignment matches cache line
size on x86-64.

The WalWriter assigns monotonically increasing sequence numbers:

```rust
pub struct WalWriter {
    pub stream_id: u32,
    pub next_seq: u64,
    buf: Vec<u8>,
    file: File,
    file_size: u64,
    first_seq: u64,
    last_seq: u64,
    wal_dir: PathBuf,
    archive_dir: Option<PathBuf>,
    max_file_size: u64,
    retention_ns: u64,
    listeners: Vec<Arc<Notify>>,
}
```

The `append` function writes a record to the buffer and assigns
the next sequence number:

```rust
pub fn append<T: CmpRecord>(
    &mut self,
    record: &mut T,
) -> io::Result<u64> {
    let payload_len = std::mem::size_of::<T>();
    // backpressure: stall if buf > 2x max_file_size
    let limit = (self.max_file_size as usize)
        .saturating_mul(2)
        .max(256 * 1024);
    // ... write header + payload to buf, assign seq
}
```

Backpressure is built into the append path. If the buffer exceeds
2x the max file size (default 64MB, so buffer limit is 128MB),
the writer forces a flush before accepting more records. This
prevents unbounded memory growth.

## Flush and Durability

The WAL flushes every 10ms or every 1000 records, whichever comes
first. Each flush calls `fsync` to ensure bytes reach durable
storage.

This is the core durability guarantee: **any fill emitted by the
matching engine is on disk within 10ms**. The 10ms window is the
maximum data loss for in-flight orders that were accepted but not
yet flushed. Fills that have been flushed are permanent.

If the disk is slow and flush lag exceeds 10ms, the matching
engine stalls order processing. This preserves the 10ms bound
under all conditions. The system trades latency for safety:
during a disk slowdown, all users experience higher latency,
but no data is lost.

## File Rotation and Retention

WAL files rotate at 64MB. Old files are retained for 10 minutes,
then garbage collected (based on file mtime). The 10-minute window
is the DXS replay buffer -- any consumer that falls behind by more
than 10 minutes must rebuild from a snapshot.

When an archive directory is configured, rotated files are moved
there before deletion. The Recorder process consumes these archived
files for long-term storage (daily rotation, append-only).

## DXS: Direct Exchange Streaming

DXS is how consumers read the WAL. Each producer (matching engine,
mark aggregator) runs a DxsReplayService that serves WAL records
over TCP.

The protocol:

1. Consumer connects, sends `from_seq` (the sequence number to
   start from).
2. Server seeks to that position in the WAL files.
3. Server streams records until the consumer catches up to live.
4. Server sends `CaughtUp` (live_seq is inclusive).
5. Server continues streaming live records as they are appended.

The consumer tracks its tip per stream and requests
`from_seq = tip + 1` on reconnect. This makes replay idempotent:
the consumer can reconnect at any time and resume from where it
left off.

### TLS Support

DXS supports optional TLS for WAL replication. This matters when
risk engines and matching engines run on different hosts. The TLS
handshake adds latency to the initial connection but not to
subsequent record streaming.

### Unknown Record Types

When a consumer encounters a record type it does not recognize,
it logs a warning and skips the record (advancing past `len`
bytes). This enables rolling upgrades: deploy consumers first
(they ignore new record types), then deploy producers that emit
them.

## CMP: The Hot Path Transport

WAL replication over TCP is the cold path -- suitable for replay
and archival, but too slow for live order flow. The hot path uses
CMP over UDP.

CMP (C Message Protocol) uses the same wire format as the WAL.
One WAL record per UDP datagram. No fragmentation -- all payloads
fit in a single datagram (max 64KB, typical payloads are <256
bytes).

```
[UDP datagram]
  [16B WalHeader][payload]
```

The CMP sender assigns sequence numbers just like the WalWriter.
The receiver tracks the expected sequence and sends NAK (negative
acknowledgment) for gaps. The sender retransmits from a ring
buffer.

This is inspired by Aeron's reliable UDP protocol: sequence-based
flow control without the overhead of TCP's congestion control.
TCP adds head-of-line blocking and Nagle's algorithm (even with
TCP_NODELAY, the kernel still batches ACKs). UDP with application-
level reliability gives us control over retransmission timing.

### CMP Configuration

CMP parameters are configurable via environment variables:

- Buffer sizes for send/receive rings
- Heartbeat interval
- NAK retransmission timeout
- Sequence gap threshold for triggering a full resync

## Recovery Scenarios

The system is designed around one recovery path: **load state,
replay from WAL**. There is no separate crash recovery code.
Normal startup and crash recovery use the same logic.

### Matching Engine Recovery

1. Load the latest orderbook snapshot (binary serialized).
2. Open the WAL and find `snapshot.last_seq + 1`.
3. Replay all WAL records from that point.
4. Resume live processing.

Recovery time: 5-10s typical (snapshot load + WAL replay).

### Risk Engine Recovery

1. Acquire Postgres advisory lock.
2. Load positions, accounts, tips from Postgres.
3. For each symbol, request DXS replay from the matching engine
   at `tips[symbol_id] + 1`.
4. Process replay fills through the same code path as live.
5. On `CaughtUp` for all streams: connect gateway, go live.

Recovery time: 2-5s typical (Postgres load + DXS replay).

### Market Data Recovery

1. Request DXS replay from each matching engine.
2. Rebuild shadow orderbook from fill/insert/cancel events.
3. Resume broadcasting to WebSocket clients.

Recovery time: <1s.

## The 0ms Fill Loss Guarantee

Here is the data loss matrix for every failure scenario we
analyzed:

| Scenario | Fill Loss | Position Loss |
|----------|-----------|---------------|
| Gateway crash | 0ms | 0ms |
| ME master crash | 0ms | 0ms |
| Risk master crash | 0ms | 10ms |
| ME + replica crash | 0ms | 0ms |
| Risk + replica crash | 0ms | 100ms |
| ME + Risk crash | 0ms | 10ms |
| ME + Postgres crash | 0ms | 100ms |

Fills are always 0ms because the matching engine WAL is the source
of truth. Even if Risk and Postgres both crash, Risk rebuilds its
positions by replaying fills from the matching engine's WAL.

Position loss is bounded by the Postgres write-behind interval
(10ms) or the worst case of multiple components crashing before
their respective flushes (100ms). But positions are always
reconstructable from fills, so "loss" here means "must replay
more records," not "data is gone."

## Tip Persistence

Tips are the mechanism that connects fills to positions. Each risk
shard maintains a vector of tips, one per symbol:

```rust
pub tips: Vec<u64>,
```

`tips[symbol_id]` is the last processed sequence number for that
symbol. Tips are persisted to Postgres atomically with position
updates. On recovery, risk loads tips and replays from
`tips[symbol_id] + 1`.

The invariant: **tips are monotonic**. They never decrease. After
recovery, in-memory tips must be >= persisted tips (the replay may
have advanced them further).

## What We Did Not Build

We did not build a distributed consensus system. There is no Raft,
no Paxos, no chain replication. Each matching engine is a single
writer to its own WAL. Risk is a single writer to its own Postgres
shard. The advisory lock provides mutual exclusion, not consensus.

This is a deliberate simplification. Distributed consensus adds
latency (an extra network round trip per commit) and complexity
(leader election, log compaction, membership changes). For an
exchange running in a single datacenter with dedicated hardware,
WAL replication plus advisory locks provides the guarantees we
need without the overhead.

The tradeoff: multi-datacenter deployments would require either
synchronous WAL replication (adding cross-DC latency) or accepting
a wider loss window (the time for a fill to replicate across DCs).
That is a v2 problem.

## The fsync you forgot

While writing about durability guarantees, we shipped a bug that
violated them. The rotate function in `rsx-dxs/src/wal.rs` had
this sequence:

```rust
// BEFORE: drop, then rename
drop(self.file);
fs::rename(&active_path, &rotated_path)?;
```

The race: `drop(file)` closes the file descriptor. The OS may
flush dirty pages at this point, or it may not -- depends on the
kernel version, filesystem, and mount options. Between the close
and the rename, a crash leaves the data written but the file still
named `*_active.wal`. On restart, the recovery logic sees an
active file, assumes it is the current segment, and overwrites it.
Silent data loss.

The fix is two steps, not one:

```rust
// AFTER: sync before close, fsync parent after rename
self.file.sync_all()?;          // flush + fsync data
drop(std::mem::replace(         // close the fd
    &mut self.file,
    File::create("/dev/null")?,
));
fs::rename(&active_path, &rotated_path)?;
// optionally: fsync(parent_dir) to make rename durable
```

`sync_all()` calls `fsync(2)` before the file is closed. This
ensures the data bytes reach durable storage while we still hold
the file descriptor. The rename that follows is now safe: if it
crashes after rename but before the parent directory fsync, the
file is at least recoverable with the correct name on most
filesystems (ext4 with `data=ordered`). For full POSIX durability
you also fsync the parent directory after rename -- otherwise a
crash can leave the directory entry update in a limbo state.

The rule: **dropping a file descriptor is not an fsync**. The
kernel may buffer the close. `close(2)` does not guarantee data is
on disk. Filesystem close semantics vary:

- `ext4 data=ordered`: data before metadata, but no fsync
- `ext4 data=writeback`: metadata can precede data
- `tmpfs`: in-memory, survives nothing
- NFS: network adds another layer of buffering

Every durable state transition needs an explicit sync point. Close
is cleanup, not commitment. If you are rotating a WAL segment and
care about the records inside it, call `fsync` before you rename,
and consider fsyncing the directory after.
