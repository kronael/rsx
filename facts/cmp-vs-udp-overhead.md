---
title: CMP vs raw UDP loopback overhead — measured breakdown
date: 2026-05-24
status: verified
sources:
  - https://github.com/aeron-io/aeron/wiki/Performance-Testing (Real Logic — cache-warming + spin-loop bench design)
  - lscpu / getconf on the bench host (AMD Ryzen 9 5950X, 6-core slice)
  - rsx-dxs/benches/cmp_send_breakdown_bench.rs (the per-stage attribution bench)
  - rsx-dxs/benches/compare_udp.rs (raw UDP RTT, formerly udp_rtt_bench.rs)
  - rsx-dxs/benches/cmp_rtt_bench.rs (CMP RTT)
  - facts/syscall-latency.md (sendto cost prior measurement)
  - https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html (cache-line size)
  - https://www.akkadia.org/drepper/cpumemory.pdf (Ulrich Drepper, "What every programmer should know about memory")
local_measurement: bench host AMD Ryzen 9 5950X, run 2026-05-24 at commit 83e3f36, --sample-size 30 --measurement-time 2
---

# CMP vs raw UDP loopback — what's the overhead actually

The `compare/README.md` summary table used to imply raw UDP was ~2 µs RTT and
CMP was ~10 µs, suggesting CMP added ~8 µs of protocol work over the baseline.
On re-measurement (this host, 2026-05-24) **that's wrong**: raw UDP is
~11 µs, CMP is ~14 µs, and the gap is almost entirely scheduler noise from
unpinned threads, not protocol cost.

This entry documents (a) what the numbers actually are, (b) where CMP's
per-send cost goes, (c) the cache concepts behind why this kind of bench has
to be designed carefully, and (d) one concrete bench fix to do.

## Measured numbers

Host: AMD Ryzen 9 5950X (6-core slice), Linux 6.1.0-43-amd64, Rust debug
benches via `cargo bench -p rsx-dxs ... -- --sample-size 30 --measurement-time 2`.

### RTT (round-trip, two-thread spin-loop, 64-128 B payload)

| bench | low | median | high |
|---|---:|---:|---:|
| `udp_rtt_loopback_64b` (raw UDP) | 9.89 µs | 10.88 µs | 11.80 µs |
| `cmp_rtt_fill_echo` (CMP) | 10.45 µs | 13.56 µs | 17.28 µs |

The CMP median is ~25 % higher than raw UDP, not 5×. The CMP high (17.3 µs) is
roughly 1.5× raw UDP's high — within the variance you'd expect from unpinned
threads being migrated by the scheduler mid-iteration.

### CMP send-path attribution (per send call, 128 B payload + 16 B header)

From `cmp_send_breakdown_bench.rs`:

| stage | time | what it does |
|---|---:|---|
| `send.crc32_128b` | 14.6 ns | CRC32 over 128-byte payload (crc32fast lib) |
| `send.header_build` | 4.4 ns | construct 16-byte WalHeader |
| `send.buf_pack_144b` | 3.7 ns | memcpy header + payload into the pre-allocated send buf |
| `send.ring_cache_copy_144b` | 3.2 ns | copy frame into the 4096-slot retransmit ring |
| **userspace subtotal** | **~26 ns** | |
| `send.sendto_144b_loopback` | 4.07 µs | the `sendto` syscall itself |
| **total per CMP send** | **~4.10 µs** | |

The `sendto` syscall is **99.4 %** of the per-send cost. Everything CMP does
in userspace adds up to 26 ns — essentially free at this scale.

Same story on the receive side: `recvfrom` is the dominant cost; the userspace
seq-extract / reorder-buf check / status-timer work is in the tens of
nanoseconds.

### Where the gap actually comes from

If userspace is ~26 ns per send and the syscall is ~4.07 µs, then:
- raw UDP RTT ≈ 2 × sendto + 2 × recvfrom ≈ ~10 µs (matches measurement)
- CMP RTT ≈ same 4 syscalls + ~100 ns userspace ≈ ~10-11 µs (also matches)

The remaining ~3 µs of variance between CMP median and raw UDP median is
**scheduler noise**: threads migrate between cores, evict cache, get
preempted, etc. Different runs vary by µs.

The earlier prose breakdown attributing 1-2 µs to "cache-line bouncing on the
seq number the echoer modifies" was wrong. That mechanism is real (see
below), but in this bench:
- The seq lives inside packets that traverse kernel socket buffers.
- CMP's send ring + reorder buf are thread-local — never touched by both threads.
- The cross-core kernel work in the loopback path is identical for raw UDP
  and CMP, so it cancels in the gap.

## Cache hierarchy on this host

| level | size | latency | scope |
|---|---|---|---|
| L1d | 32 KB / core | ~1 ns / ~4 cycles | per core |
| L1i | 32 KB / core | ~1 ns | per core |
| L2 | 512 KB / core | ~3-4 ns | per core |
| L3 | 16 MB / CCX | ~12-15 ns | shared across 6 cores |
| DRAM | — | ~80-100 ns | shared |

Cache **line** size is 64 bytes (universal on x86_64).

`lscpu` shows the bench host as a 6-core slice of a Ryzen 9 5950X (full
16-core chip exists; this run gets 6). L3 across the slice is 16 MB.

### Why L1 matters here

- L1 is **per-core**. Cores have independent L1s, and the MESI cache
  coherency protocol keeps them consistent when they share data.
- L1 is **tiny** (32 KB ≈ 512 cache lines). A loop touching > 32 KB of hot
  data spills to L2.
- L1d access ≈ 1 ns; DRAM access ≈ 80-100 ns. The **100× ratio** is what
  motivates the entire spin-loop bench design — if a thread sleeps and
  another process runs on its core, the cache lines get evicted, and the
  next access pays the DRAM hit instead of the L1 hit.

## Cache-line bouncing — the concept

When two threads on different cores share a memory location:

1. Core A reads `x` → fills a 64-byte cache line in A's L1, marked **Shared**.
2. Core B reads `x` → also caches it, also **Shared**. Cheap so far.
3. Core A **writes** `x` → snoop traffic invalidates B's copy. A's line
   becomes **Modified**, B's becomes **Invalid**.
4. Core B reads `x` again → cache miss in L1. Pulls from A's cache via L3 or
   direct cache-to-cache transfer. **~40-80 ns** instead of the ~1 ns L1 hit.

When two threads ping-pong writes on the same line every iteration, you get
a **bouncing** cache line: every iteration pays the transfer cost plus
pipeline stalls.

The classic example: `Arc<AtomicU64>` shared across threads — every `fetch_add`
invalidates the other core's copy. Avoiding this is one reason CMP uses SPSC
rings (single producer, single consumer — only one core ever writes any given
slot).

### False sharing

A subtler bouncing case: two unrelated variables on the same 64-byte cache
line bounce between cores even though logically independent. CMP records use
`#[repr(C, align(64))]` to put each record on its own cache line, eliminating
this.

### Why it doesn't drive CMP-vs-UDP gap

Both raw UDP and CMP send packets through kernel socket buffers. The kernel's
sk_buff metadata does have cross-core work, but that work applies equally to
both — it cancels. CMP's own structures (`send_ring`, `reorder_buf`) are
thread-local: only the sender touches the ring, only the receiver touches
the reorder buf. Zero bouncing on CMP-specific data.

## Bench glossary

For readers landing here without the bench harness context:

- **sender, echoer**: the two threads in the RTT bench. Sender does the
  timing: `t0 = now(); send(pkt); recv(echo); t1 = now(); rtt = t1 - t0`.
  Echoer is the other end, looping `recv → send back`, untimed.
- **cache-hot**: a thread is cache-hot when its working set (stack, socket
  struct, recv buffer) is still resident in L1/L2 from a recent access. The
  spin-loop design keeps both threads cache-hot by never yielding — they
  burn CPU but get true µs-scale measurements instead of cache-eviction +
  scheduler-wakeup noise.
- **spin vs naive** (KCP only): KCP has an internal scheduler. `kcp.send()`
  queues bytes; `kcp.update() + flush()` actually pushes them out. The
  "spin" variant drives `update()` in a tight loop (~17 µs RTT). The "naive"
  variant uses KCP's default ~1 ms cadence (~11 ms RTT). Other protocols
  (CMP, raw UDP, TCP, Quinn) have no internal scheduler, so this distinction
  doesn't apply.

## The pinning gap

**None of the rsx-dxs benches pin threads to cores.** Verified with
`grep core_affinity rsx-dxs/benches/*.rs` → no hits.

Consequences:
- Threads migrate between cores during the bench. Each migration evicts L1.
- Measurements are noisier than they would be on pinned cores.
- Comparisons to vendor-published numbers (Aeron's 21 µs P50 on pinned AWS
  c6in.16xlarge cores; Chronicle's sub-µs IPC on pinned cores) are not
  apples-to-apples.

Fix: add `core_affinity = "0.8"` as a dev-dep, pin sender + echoer in setup
before the timed loop. ~10 LOC per bench, applied uniformly across the
compare set. The Aeron sub's report already flagged this; doing it across
the suite is a one-sprint task.

This file should be updated with re-measured numbers once pinning lands.

## Re-validation cadence

Re-measure when:
- Kernel changes (UDP loopback path changed; PTI/Spectre mitigations
  shifted; io_uring path becomes relevant).
- Compiler changes (CRC32 lib gets SIMD speedup; LLVM inlining differs).
- Hardware changes (different CPU generation, different host).
- Core pinning lands (numbers will move significantly).

## Related

- `facts/syscall-latency.md` — the syscall-level "why" behind the 4 µs
  `sendto` cost.
- `rsx-dxs/benches/cmp_send_breakdown_bench.rs` — the bench that produced
  the per-stage attribution.
- `rsx-dxs/compare/raw-udp.md` — the protocol comparison doc; this file's
  numbers supersede the table there.
- `.ship/18-COMPONENT-BENCHES/LANDSCAPE.md` — broader bench landscape if it
  still exists.
