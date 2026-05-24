# Chronicle Queue

Java off-heap append-only log by Peter Lawrey (OpenHFT / Chronicle
Software). Used by 8 of the top 11 investment banks per their
marketing. The closest Java prior art to rsx-dxs's "disk = wire"
design philosophy.

Repo: https://github.com/OpenHFT/Chronicle-Queue (Apache-2.0)

## Design

Chronicle Queue is not a UDP transport. It is a **persisted,
memory-mapped, off-heap log** for intra-process and inter-process
communication on a single host or replicated across hosts.

Key properties:
- **Off-heap mmapped files**: data never touched by GC.
- **Append-only, sequential**: same byte sequence on disk and in
  memory — no serialisation transformation.
- **Deterministic replay**: consumers re-read from any offset.
  Disk IS the source of truth.
- **Sub-microsecond IPC**: producer appends; consumer polls. No
  syscall on the hot path (mmapped region, no `write()`).
- Chronicle Wire: extensible, self-describing binary/text encoding
  (YAML, JSON, binary). Not `#[repr(C)]` C structs.

Chronicle Network adds TCP transport between hosts. UDP is
Chronicle Network Enterprise (commercial, undocumented).

## Relation to rsx-dxs

Both share the core insight: **the log is the source of truth and
the replay source**. One byte layout for persistence and delivery.

| Dimension | Chronicle Queue | rsx-dxs |
|---|---|---|
| Transport | mmapped file + optional TCP | CMP/UDP (hot) + TCP (cold WAL replay) |
| Wire | Chronicle Wire (self-describing) | `#[repr(C, align(64))]` fixed C structs |
| IPC mechanism | mmap (shared file, same host) | SPSC rings (rtrb) |
| Cross-host | TCP replication (Chronicle Engine) | CMP/UDP live + WAL TCP replay |
| Language | Java (GC, off-heap) | Rust (no GC, no heap on hot path) |
| Fragmentation | Supports large messages | Fixed ≤ MTU (no fragmentation) |
| NAK retransmit | N/A (mmap = no loss) | Yes (NAK + WAL cold tier) |
| Sub-µs IPC | Yes (~200–500 ns, no syscall) | Yes via SPSC rings (~50–170 ns) |

Chronicle Queue is excellent for intra-host use. It has no UDP hot
path; network delivery requires a separate TCP layer. rsx-dxs embeds
both in one crate.

## Oracle critique finding

The oracle called Chronicle Queue "the closest philosophical prior
art" to rsx-dxs's "disk = wire" claim. This is accurate for the
concept. The differences are:
1. No UDP — Chronicle Queue does not send datagrams.
2. No NAK retransmit — mmap cannot drop bytes.
3. Java / self-describing encoding vs Rust / `repr(C)` fixed structs.
4. No WAL-as-retransmit-source — there is no "retransmit" concept
   because mmap provides perfect delivery within the host.

rsx-dxs extends the "log is the truth" idea to a **network transport**
with NAK-based reliability. That is the difference.

## Direct benchmark

Not applicable — different transport model (mmap vs UDP).
Published numbers from OpenHFT:
- IPC throughput: 1–6 M messages/s depending on message size.
- Latency: ~230 ns (same host, shared mmap, OS scheduler permitting).

Source: https://github.com/OpenHFT/Chronicle-Queue,
https://github.com/OpenHFT/Chronicle-Network
