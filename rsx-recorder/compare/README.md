# rsx-recorder: durable-append alternatives — the wider field

Companion to `rsx-cast/compare/` and `.ship/34-COMPARE-RESEARCH/`.
This is the `niche.md`-style census for the **archival** end of the
pipeline: where the recorder sits against the well-known durable-log
and archive systems, what axis each one actually optimises, and why a
raw messages/s number across them is apples-to-oranges.

**Read the fairness rule first.** A published number is a head-to-head
only if it is *same-op + same-durability-model + same-hardware +
same-language*. Everything else is directional context, never
"N× faster". The recorder is **off** the GW→ME→GW critical path — its
latency never enters the 7.5 µs round-trip budget, so never place its
lag next to an on-path number.

## What rsx-recorder is

A single-purpose archival replication consumer
(`rsx-recorder/src/main.rs`: one `RecorderState` + one
`rsx_cast::ReplicationConsumer`). It opens **one long-lived TCP
connection** to a matching engine's replication server, receives the
raw WAL record stream (`RawWalRecord` = header + payload, no
transformation), buffers into a 64 KB `Vec`, and flushes to a
date-partitioned archive file (`{stream_id}/{stream_id}_{date}.wal`).
It rotates daily at UTC midnight and persists a consumption tip for
idempotent restart. Runtime is **tokio** — async file I/O + one
socket, no hot loop, no core pinning: an explicit trade of latency
(TCP head-of-line, kernel cwnd) for operational simplicity
(`rsx-recorder/ARCHITECTURE.md`). The one property none of the peers
below advertise is **wire format = disk format = audit log, no
transformation** (the rsx-cast thesis).

## The one metric that matters

**Sustained durable-append throughput (records/s) and durability lag**
— time from "record received on the socket" to "record `fsync`'d to
the archive file". The current design calls `file.sync_all()` (a real
fsync) **every 1000 records** (`main.rs::flush`), so durability lag is
bounded by 1000-record batches and throughput is gated by fsync cost
amortised over 1000 records + TCP delivery. This is a **stricter
durability model** than every headline number below — state that
before any comparison.

## Honest reference points

| System | Number | HW / durability caveat | Head-to-head? | Source |
|---|---|---|---|---|
| **Chronicle Queue** | sustained **5M msg/s**; 99%ile **3.69 µs @ 500k/s**; <10 µs 99.9%ile to 1.4M/s | 2×12-core Xeon E5-2650 v4, **queues on `tmpfs`** (RAM-backed, not disk-durable); Java; mmap IPC not TCP | no — tmpfs ≠ fsync, different transport + language | [chronicle.software](https://chronicle.software/throughput-benchmarks-upto-5-million-messages-per-second/) |
| **Chronicle Queue** (persisted event) | **~660 ns/event** persisted | commercial, mmap-shared, single host | no — mmap IPC, different durability | [chronicle.software](https://chronicle.software/building-fast-trading-engines-chronicles-approach-to-low-latency-trading/) |
| **Aeron Archive** | records to disk **at full transport rate**; OSS >350k msg/s, Premium >3M msg/s | separate archive process; Java/C++; rsx-cast *fuses* wire=disk into one consumer | no — separate process, different language | [AWS](https://aws.amazon.com/blogs/industries/aeron-performance-enables-capital-markets-to-move-to-the-cloud-on-aws/) |
| **Kafka (LinkedIn, 2014)** | **2M writes/s** on 3 machines | commodity cluster, **page-cache batched, no per-record fsync** (durable only on later flush) | no — batched durability, cluster not single consumer | [LinkedIn Eng](https://engineering.linkedin.com/kafka/benchmarking-apache-kafka-2-million-writes-second-three-cheap-machines) |
| **Kafka (tuned 3-broker)** | **~1.05M rec/s (100 MB/s)** | `batch.size=131072, linger.ms=10, lz4` | no — batched + compressed | [oneuptime](https://oneuptime.com/blog/post/2026-01-25-tune-kafka-million-messages-per-second/view) |

**The honest reading.** Every one of these optimises a *different
durability axis* than rsx-recorder's per-1000-record fsync-to-disk.
Chronicle's 5M/s is on `tmpfs` (RAM — nothing survives a crash);
Kafka's 1-2M/s is page-cache batched (durable only when the OS or a
`flush.messages` threshold later syncs); Aeron Archive is a dedicated
process, not a fused wire=disk consumer. Comparing their headline
msg/s to ours is comparing "how fast can bytes reach RAM/page-cache"
against "how fast can bytes reach a `fsync`'d disk block". The recorder
is **not competing on raw MB/s** — it is the trivially-simple tail of a
`ReplicationConsumer` that already did the hard part (`rsx-cast`), and
its differentiator is a fsync'd audit log identical to the wire format.

## One-paragraph framing

- **Chronicle Queue** — the persistent-log-as-transport gold standard,
  Java/OpenHFT. Its published 5M/s is a `tmpfs` (RAM) run; its
  disk-durable numbers are lower and less advertised. Different
  durability axis; cite the tmpfs caveat every time.
- **Aeron Archive** — the direct architectural peer: a separate process
  that records an Aeron stream to disk at full transport rate. rsx-cast
  fuses that role into the transport (wire = disk), so the recorder is
  a thin consumer, not a co-equal archive daemon.
- **Kafka** — the commodity durable log. Its million-msg/s headlines are
  page-cache batched across a broker cluster; per-record-fsync Kafka is
  a much slower configuration. Different guarantee, different scale-unit.

## What we could actually build in-repo

**A Criterion microbench does NOT fit** — this is I/O-bound (fsync +
TCP), not CPU-bound, and `rsx-recorder/` currently has **zero benches**.
The honest harness is a self-timed throughput + durability-lag loop
(a `harness = false` bench like `risk_flood_bench`), mirroring
`rsx-book/benches/compare_naive_bench.rs`:

- **Throughput bracket:** feed N synthetic `RawWalRecord`s straight into
  `RecorderState::write_record` (bypass TCP), measure records/s + bytes/s
  to (a) a real disk and (b) `tmpfs` — brackets the fsync cost the way
  Chronicle's tmpfs run does, so the comparison becomes apples-to-apples.
- **Durability-lag sweep:** vary flush batch (per-record / per-100 /
  per-1000), report p50/p99 lag = `write_record` return → fsync
  complete. This is the number that actually matters and **no competitor
  publishes it comparably**.
- **Naive baseline:** the same stream through a plain `BufWriter<File>`
  + `flush()` (no fsync, no rotation, no tip) — shows what the
  durability + rotation + tip machinery costs over a dumb append.
- **Cannot measure in a bench:** real TCP replication under
  loss/backpressure, cross-DC RTT, disk contention from a colocated ME.
  Those need `playground start-all` and belong in a `reports/` run.

## Traps

- **Durability-model mismatch (the big one).** Quoting Kafka's 2M/s or
  Chronicle's 5M/s next to rsx-recorder is dishonest unless the
  durability matches: their headline numbers are page-cache-batched /
  tmpfs, ours is per-1000-record fsync-to-disk. Compare fsync-to-fsync
  or explicitly say "different guarantee".
- **Single TCP consumer vs multicast archive.** Aeron Archive can tap a
  multicast stream; the recorder is one unicast TCP consumer. Different
  fan-in.
- **Off the critical path.** The recorder's lag never enters the
  GW→ME→GW budget — never place it next to the 7.5 µs round-trip.
