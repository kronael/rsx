# Chronicle Queue

Open-source durable IPC queue by Peter Lawrey / OpenHFT
(now Chronicle Software). Memory-mapped append-only log with
write-lock-serialised multi-writer + lock-free multi-reader
semantics. Production-deployed in Java HFT and trading
platforms for 10+ years.

Repo: https://github.com/OpenHFT/Chronicle-Queue (Apache-2.0)

## Why we include it

The other entries in this survey (Aeron, KCP, Quinn, raw UDP,
TCP, LBM) compete with rsx-cast on the **UDP transport** axis:
how do you move bytes reliably from one host to another?

Chronicle Queue does not compete on that axis. It is a useful
comparator on a different axis: **persistence-as-protocol** —
the design choice that the on-disk log *is* the wire format
and *is* the reader cursor space.

- Chronicle: WAL on disk, mmap shared with reader processes.
  Persistence and IPC are the same artefact.
- rsx-cast: WAL on disk, identical bytes broadcast over CMP/UDP,
  identical bytes streamed back via TCP for cold replay.
  Persistence, live transport, and replay are the same artefact.

Both refuse the more common pattern of "encode for the wire,
re-encode for the log, re-decode for the replay path." That
design choice has the same payoff in both systems: zero
transformation cost, the log is provably what was sent, replay
is bit-identical to the original delivery.

Aeron is the closest peer on the UDP-NAK axis. Chronicle is a
useful comparator on the WAL-is-the-protocol axis. They sit on
different sides of rsx-cast's design.

## Design

### Storage

- **`.cq4` files** in a directory representing one logical queue.
- **Cycle-rotated**: one file per cycle. Default `RollCycles.DAILY`
  (filename `yyyyMMdd.cq4`); hourly and weekly options exist. The
  roll-cycle is fixed at queue creation and cannot change.
- **Multi-level index**: every `.cq4` file embeds an `index2index`
  primary index + secondary indices that point at message offsets.
  This makes random-seek-by-sequence O(log n) within a cycle file
  rather than a linear scan.
- **`metadata.cq4t`** (v5+) is a per-queue table store that holds
  the write-lock state and the cycle range. In v4 the
  cycle-range cache lived in a separate `directory-listing.cq4t`;
  v5 folded that into the same table store. Either way, tailers
  use the cached cycle range to avoid `readdir()` on every poll.

### Wire format

Chronicle commonly uses self-describing **Chronicle Wire** —
each message is a "DocumentContext" framed with a 4-byte length
prefix + tag bits + body. Fields can be name-tagged (binary
YAML), which lets readers in different schema versions read the
same file. Trade-off: every field carries a small tag overhead.

Chronicle also exposes lower-level bytes-oriented layouts:
`BytesMarshallable`, `writeBytes`/`readBytes`, and a
`FIELDLESS_BINARY` wire mode let callers write fixed-layout
payloads inside a DocumentContext when self-description isn't
needed. The format is a spectrum, not one fixed encoding.

### IPC mechanism

A writer appends to the mmapped tail of the current `.cq4` file.
A reader holds its own `ExcerptTailer` cursor (an `(index, file
offset)` pair) and reads the same mmapped pages directly. On
Linux the appender and tailer share the same page cache pages.
In the steady state — mappings established, pages resident, no
rollover — a tailer can read the next message by spinning on
the "next message length" header in shared memory, with no
kernel transition on the read.

That steady-state path is the source of the sub-µs IPC claim.
File setup, page faults, cycle rollover, and the periodic
directory-cache refresh all still go through the kernel.

### Durability

This is where the design diverges most from rsx-cast.

OSS Chronicle Queue **defaults** to relying on the OS page
cache. The FAQ:
> "The operating system will work in the background ensuring
> that entries written to the page cache are propagated to the
> disk, but this is done via the operating system and is not on
> the critical path."

There is no automatic `msync` / `fsync` cadence on the default
write path. Manual sync APIs do exist —
`ExcerptCommon.sync()`, `ChronicleQueue.lastIndexMSynced()`,
and Chronicle-Bytes `SyncMode` — but they are caller-driven,
not a built-in periodic flush. If the process is killed and
the host stays up, the page cache flushes normally and no data
is lost. If the **host** loses power before the kernel flushes
and the caller hasn't manually synced, anything still in the
page cache is gone, and Chronicle does not by itself bound that
window.

Chronicle Enterprise adds an "async mode" (separate buffered-
queue product with its own performance / durability profile,
plus replication), but that is a commercial product not
represented in this comparison.

rsx-cast takes the opposite **default**: `WalWriter::flush()`
invokes `File::sync_all()` (fsync), and the producer loop
calls flush every 10 ms. Worst-case data loss on power failure
is bounded to ~10 ms of records. The cost is the 651 µs fsync
on the flush path; that cost is amortised across however many
records batched into one flush window. See
`rsx-cast/benches/wal_fsync_bench.rs`.

The comparison here is about **defaults**, not capabilities.
A Chronicle deployment that calls `sync()` on every append
can match rsx-cast's durability bound; an rsx-cast deployment
that disables the 10 ms flush cadence can match Chronicle's
throughput. The defaults match the systems' assumed
deployment: Chronicle in a JVM with the OS managing the page
cache; rsx-cast in an exchange where the power-loss window
matters for audit.

### Multi-writer

OSS Chronicle Queue supports **multiple concurrent writers**
on the same machine — across threads or across JVMs — by
serialising appends through a queue write lock held in
`metadata.cq4t`. The README calls this out explicitly
("supports concurrent writers and readers even across
multiple JVMs on the same machine"). Throughput obviously
drops vs single-writer because writers contend for the lock,
but the capability is there in OSS. The Enterprise "async
mode" product is a separate buffered-queue implementation
that lifts that contention for higher throughput; it isn't
required just to have more than one writer.

rsx-cast is single-writer per stream by construction — there
is no shared write lock; each producing tile owns its own
WAL stream. Multi-producer fan-in is modeled as multiple
streams with consumers merging on `(stream_id, seq)`. This
side-steps the lock entirely rather than serialising on it.

### Cross-host

OSS Chronicle Queue is single-host only. Cross-host replication
("Chronicle Queue Enterprise Replication") is a commercial
product layered on Chronicle Network + TCP.

rsx-cast has cross-host built in: CMP/UDP unicast for the live
path and TCP replay for the cold path are both in `rsx-cast/`.
There is no separate enterprise tier; the same WAL serves the
local archival reader (`rsx-recorder`) and the cold-replay
client.

## Guarantees comparison

Comparing OSS Chronicle Queue against rsx-cast included
features. "Default" means the out-of-the-box behaviour;
where capability differs from default it's called out.

| Property | Chronicle Queue (OSS) | rsx-cast WAL + CMP |
|---|---|---|
| On-disk format | `.cq4` mmapped, Chronicle Wire (self-describing or fieldless) | `#[repr(C, align(64))]` fixed-layout records + 16 B header |
| Disk format == wire format | yes (mmap IPC reads same bytes) | yes (CMP frames + TCP replay are identical to WAL records) |
| Rotation trigger | time (daily / hourly / weekly cycle) | size (64 MB) + retention GC (48 h) |
| Random seek by sequence | O(log n) via embedded index | O(n) linear scan within a 64 MB file (no per-file index) |
| Default per-append sync | none — OS page-cache flush only | `sync_all()` on every flush; flush cadence = 10 ms |
| Manual sync API | yes (`ExcerptCommon.sync()`, `lastIndexMSynced()`, Bytes `SyncMode`) | yes (`WalWriter::flush()`) |
| Default durability bound | unbounded (caller-driven) | ~10 ms (cadence-driven) |
| Concurrent writers | yes — serialised by write lock in `metadata.cq4t` | no shared lock; one writer per stream, fan-in via multiple streams |
| Multiple readers | yes (mmap IPC, steady-state sub-µs) | yes (TCP replay; CMP unicast is point-to-point) |
| Cross-host transport | not in OSS; enterprise replication is commercial | included (CMP/UDP live + TCP replay) |
| Hot-path syscalls (steady state) | none on read; appender writes to mapped pages | `sendto` per CMP frame on send; TCP for cold path |
| Wire schema evolution | self-describing, version-tolerant | fixed C structs, version byte in `WalHeader` |
| Language | Java / Kotlin (JVM) | Rust |
| Hot-path GC | zero (off-heap) | n/a (no GC) |

## Published performance numbers

From the Chronicle Queue README (commit-current as of writing,
on a referenced i7-4790 / Linux box):

- **Same-host IPC, 40-byte messages**: "a high percentage of
  the time we achieve latencies under 1 microsecond. The 99th
  percentile reaches 0.78 µs and 99.9th percentile hits 1.2 µs
  at 10 million events per minute."
- **Throughput, 96-byte messages**: "approximately 5 million
  messages/second on an i7-4790 processor."
- **Cross-machine (Enterprise)**: "under 10 µs" — commercial,
  not directly comparable.

These are mmap-IPC numbers: writer and reader on the same host,
sharing physical pages. In the steady-state read path the kernel
is not on the critical path; setup, faults, and rollover still
involve it.

## rsx-cast reference numbers

From `rsx-cast/benches/`, criterion p50 on the dev box. These
benches are pure WAL-side; the CMP and end-to-end RTT numbers
live in the other compare/ docs.

| Bench | What it measures | p50 |
|---|---|---:|
| `wal_bench::wal_append_in_memory` | `WalWriter::append` to in-memory buffer, no flush | ~31 ns |
| `wal_fsync_bench::wal_append_fsync_single` | append + explicit flush + `sync_all()` | ~651 µs |
| `wal_fsync_bench::wal_append_fsync_batch_100` | 100 appends + one flush + fsync | ~700 µs (~7 µs/record amortised) |
| `wal_bench::wal_read_sequential_10k` | open + replay 10 K records sequentially | sub-ms |
| `wal_random_read_bench::wal_random_read_10k` | random `read_record_at_seq` over 10 K file | ~23 ms (O(n) linear scan) |
| `wal_random_read_bench::wal_random_read_100k` | random over 100 K file | linearly higher |

The take-away when comparing:

- **In-memory append** (31 ns) is in the same ballpark as the
  Chronicle "well under 1 µs" claim. Both are bounded by a
  handful of cache-line writes + one bounds check.
- **Durable append** (651 µs unamortised) is the cost rsx-cast
  pays on every flush by default. OSS Chronicle does not pay
  it by default — the OS page cache is left to flush in
  background. Callers can opt in via `ExcerptCommon.sync()`
  or `SyncMode`, at which point similar fsync-class costs
  apply.
- **Random read by seq** (23 ms over 10 K records) is rsx-cast's
  weakest leg — there is no per-file index. Chronicle's
  multi-level index makes the same operation O(log n) and would
  win on any large file. This is logged as a deferred feature.
  See `docs/benches.md` "wal_random_read".

The honest summary: Chronicle wins on **steady-state IPC
latency** (mmap > syscall) and on **random seek** (indexed >
linear). rsx-cast wins on **cross-host out of the box** and on
**bounded durability** (10 ms vs unbounded).

## Why we did not write a direct benchmark

The benches in this directory (`compare_kcp.rs`, `compare_quinn.rs`,
`compare_udp.rs`) all share one harness: spin up two endpoints
in the same Rust process, measure echo RTT for a 64-byte CMP
frame.

Chronicle Queue can't slot into that harness honestly:

1. **No Rust client.** Chronicle Queue is JVM-only. The only
   in-process route is JNI, which adds tens of µs per call and
   would dominate the measurement.
2. **Out-of-process JVM benchmark would measure the IPC, not
   Chronicle.** Spawning a Java echo subprocess and talking to
   it over TCP or a pipe benches TCP/pipe latency with Chronicle
   as a stationary participant — that's an apples-to-oranges
   number that would mislead more than inform.
3. **The model isn't comparable.** CMP is sender → wire → receiver
   over an unreliable channel with NAK retransmit. Chronicle
   IPC is appender → mmap page → tailer with no channel and no
   loss. A single RTT number in microseconds means different
   things in each.

A Java JMH benchmark would be a perfectly valid measurement
— but it would be a **different harness measuring a different
thing**: mmap IPC RTT in Java, vs CMP/UDP RTT in Rust. Sticking
those two numbers next to each other invites the wrong
conclusion. We've chosen side-by-side published / self-harness
numbers rather than fabricating an apples-to-apples RTT. The
compare/ directory does include Criterion benches for protocols
that **do** plug into the Rust harness (`compare_kcp.rs`,
`compare_quinn.rs`); Chronicle intentionally does not, for the
reasons above.

If a future deployment needs an mmap intra-host IPC path in
addition to CMP/UDP, the relevant comparison is not "rsx-cast vs
Chronicle" but "should rsx-cast grow an mmap reader for the same
WAL the producer is already writing?" The WAL bytes are already
on disk in the right layout; the missing piece is a tailer-side
mmap helper. That's a feature gap, not a protocol gap.

## Sources

- README: https://github.com/OpenHFT/Chronicle-Queue
- How it works:
  https://github.com/OpenHFT/Chronicle-Queue/blob/master/docs/How_it_works.adoc
- FAQ (durability / page cache):
  https://github.com/OpenHFT/Chronicle-Queue/blob/master/docs/FAQ.adoc
- Async mode (enterprise):
  https://github.com/OpenHFT/Chronicle-Queue/blob/master/docs/async_mode.adoc
- Chronicle Software product page:
  https://chronicle.software/queue/

Internal cross-references:
- `rsx-cast/benches/wal_bench.rs` — append + sequential read
- `rsx-cast/benches/wal_fsync_bench.rs` — durability cost
- `rsx-cast/benches/wal_random_read_bench.rs` — cold-tier seek
- `rsx-cast/src/wal.rs` — the actual WAL implementation
- `docs/benches.md` — bench index + caveats
