---
topic: syscall latency on x86_64 Linux
date: 2026-05-23
status: verified
sources:
  - https://gms.tf/on-the-costs-of-syscalls.html (Georg's Log,
    measured nanosecond figures via vDSO and bare syscall)
  - https://blogs.oracle.com/linux/syscall-latency (Oracle,
    UEK5 → UEK6 mitigation impact on getpid)
  - https://arkanis.de/weblog/2017-01-05-measurements-of-system-call-performance-and-overhead/
    (Arkanis, vDSO speedup ratios)
  - https://www.quantvps.com/blog/kernel-bypass-in-hft
    (QuantVPS, kernel-bypass numbers in HFT context)
  - https://talawah.io/blog/linux-kernel-vs-dpdk-http-performance-showdown/
    (Linux kernel vs DPDK HTTP head-to-head)
  - https://anshadameenza.com/blog/technology/2025-01-15-kernel-bypass-networking-dpdk-spdk-io_uring
    (Anshad Ameenza, kernel-bypass survey 2025)
  - https://manpages.debian.org/testing/liburing-dev/io_uring_sqpoll.7.en.html
    (Debian man, io_uring_sqpoll mechanics)
  - https://unixism.net/loti/tutorial/sq_poll.html
    (Lord of the io_uring, SQPOLL details)
  - own measurement `dfe2ef4` bench-match-rt + cmp_send_breakdown
local_measurement: rsx-dxs/benches/cmp_send_breakdown_bench.rs
                   rsx-cli/src/bin/bench_match_rt.rs
---

# Syscall latency on x86_64 Linux — and how far you can push it

## TL;DR

| Path | Per-call cost | Note |
|---|---:|---|
| vDSO-mapped call (`clock_gettime`, `gettimeofday`) | **~15-25 ns** | no kernel mode switch |
| Bare syscall **without** mitigations (pre-2018) | **~70 ns** | historical |
| Bare syscall **with** PTI + Spectre-v2 mitigations (today) | **~200-500 ns** | typical modern Linux on x86_64 |
| `getpid` via `syscall` (Skylake-X, UEK6) | **217 ns** | Oracle UEK6 measurement |
| `sendto` UDP loopback, 144 B | **~3.8 µs** | own dev-box, dfe2ef4; kernel-side mostly |
| `io_uring_enter` (one SQE submit + wait) | **~300-500 ns** | one syscall per submission, no SQPOLL |
| `io_uring` SQPOLL mode (kernel thread polls SQE ring) | **zero syscalls** | trade: kernel thread burns CPU |
| DPDK packet RTT (kernel bypass) | **1-10 µs** | poll-mode driver, no syscall |
| AF_XDP packet | **~DPDK ± 10%** | kernel still owns NIC; smaller bypass |

## What a syscall costs

The bare cost is the **user→kernel→user transition itself**:
- Saving + restoring registers
- Switching CR3 (Page Table Isolation, post-Meltdown)
- Branch-prediction barrier (IBRS / IBPB, post-Spectre v2)
- TLB flush (PTI)

Pre-mitigation (Linux pre-2018, no PTI): ~70 ns per syscall.

Post-mitigation (PTI on by default on x86_64 since 4.15):
**~200-500 ns** for the entry/exit alone, depending on CPU
generation and microcode. Oracle's blog measured `getpid`
going from 191 ns (UEK5 / pre-mitigation) → 217 ns (UEK6
/ mitigated) on Skylake-X — that's a 15% bump for the
*simplest possible* syscall; heavier syscalls absorb the
~100-300 ns mitigation tax on top of their own work.

`5×` is the commonly-cited factor for mitigation-driven
overhead increase (70 → 350 ns) on workloads that are
mostly syscall-bound.

## vDSO is essentially free

A vDSO call doesn't cross into kernel mode. It runs entirely
in user space, reading kernel-maintained shared memory.
`clock_gettime(CLOCK_REALTIME)` measures at **13-24 ns**
on Linux x86_64 — essentially the cost of an indirect
function call. About **12× faster** than the bare syscall
equivalent on a Core i7 from 2014.

Available via vDSO on x86_64:
- `clock_gettime`, `clock_getres`
- `gettimeofday`
- `getcpu`
- `time`

**Practical impact for hot-path code**: read time from
vDSO, not from a syscall. We do (`SystemTime::now` is
backed by `clock_gettime` which Rust's stdlib routes
through vDSO when available).

## `sendto` / `recvfrom`: what we actually pay

Measured locally (dev box, x86_64, kernel 6.1):

| Call | p50 |
|---|---:|
| `sendto(buf=144B, dest=loopback)` | **3.85 µs** |
| Bare syscall entry/exit (background reference) | ~200-500 ns |
| Therefore: kernel data-path work (skb alloc, loopback routing, lo rx queue push) | ~3.3-3.6 µs |

The syscall overhead is *not* the dominant cost for
networking syscalls. The kernel's data-path work
(`skb_alloc`, `ip_local_out`, `loopback_xmit`) is most of
the wall time. That's why io_uring's "eliminate syscalls"
pitch matters less for UDP than its "eliminate
context-switch latency" pitch — the kernel still does
the data-path work, but you don't pay for it on the
caller's wall clock.

## io_uring: three modes, three costs

### Mode A — default (one syscall per `io_uring_enter`)

You write SQEs into the shared ring, then call
`io_uring_enter` to tell the kernel about them.

- Submission cost: **~300-500 ns** for the syscall, plus
  whatever kernel-side work the op does.
- Same mitigation tax as any syscall.

Wins via batching: one `io_uring_enter` can submit dozens
of SQEs, amortizing the syscall over many ops.

### Mode B — SQPOLL (zero syscalls)

A kernel thread polls the SQE ring continuously. Userspace
writes SQEs; kernel sees them within ~µs without any
syscall.

- Submission cost: **0 ns of syscall overhead** —
  userspace ring write + memory barrier.
- Trade-off: kernel thread burns ~1 core. Idle threshold
  configurable (`sq_thread_idle_ms`); thread parks if no
  SQEs for that long, then needs `IORING_SQ_NEED_WAKEUP`
  to wake (which IS a syscall).
- Suitable when: sustained submission rate > a few k/s.
- Not suitable when: bursty / sub-second idle windows.

### Mode C — IOPOLL (busy-poll completions)

Similar idea, but for completions. Userspace polls CQE
ring instead of waiting for the kernel to signal.

## Kernel bypass: when the kernel itself is the bottleneck

Once you've gotten syscalls out of the way, the next layer
is the kernel's network stack itself: IP routing, conntrack,
netfilter, qdiscs, TCP/UDP state. For UDP that's ~3 µs of
loopback work (our measurement); for full-stack TCP over
real net it's ~10-100 µs.

**DPDK** — poll-mode driver running in user-space talks
directly to the NIC. The kernel doesn't see packets.

- Per-packet latency: **1-10 µs**
- Throughput: 10-100 Mpps possible on a single core
- Cost: ties up a core 100% to poll the NIC; needs DPDK-
  capable NIC; loses all kernel networking (no `ip`,
  `tcpdump`, conntrack, etc.)

**AF_XDP** — XDP-based hook at the driver layer. Packets
reach userspace via a shared memory ring; kernel still
manages the NIC.

- Per-packet latency: close to DPDK (within ~10-40%)
- Compatible with normal kernel tooling
- Better fit when "almost kernel-bypass" is enough

**RDMA / RoCE** — Layer 2 bypass with hardware offload.
Microsecond RTTs end-to-end on real hardware. Different
problem space (datacenter inter-host), not relevant for
in-process loopback.

## Concrete head-to-head

From the Talawah HTTP showdown (real workload, not micro-bench):

| Stack | Throughput (req/s) | p50 latency | p99 latency |
|---|---:|---:|---:|
| Linux kernel (baseline) | 357 K | 696 µs | 960 µs |
| Linux kernel (fully tuned) | 1.01 M | 246 µs | 333 µs |
| DPDK (initial) | 1.19 M | 204 µs | 297 µs |
| DPDK (write-combining tuned) | 1.51 M | 152 µs | 233 µs |

DPDK is ~1.5× faster at p50 over a fully-tuned kernel. The
gap is structural (kernel stack ate ~50 µs of irreducible
work). For our case, the gap is bigger because we're not
"fully tuned" — the gateway uses monoio (io_uring) but
risk and matching use std::net which pays the full
sendto cost.

## How this maps to our 3.85 µs `CmpSender::send`

Our send body is 99% sendto cost (`dfe2ef4` decomposition).
Standard `std::net::UdpSocket::send_to` on Linux 6.1
without mitigations bypass:

1. `syscall` instruction → ring 0 (~200-500 ns mitigation tax)
2. `__x64_sys_sendto` → `__sys_sendto` → `sock_sendmsg`
3. `udp_sendmsg` builds skb (~300 ns alloc + copy)
4. `ip_local_out` → `__ip_finish_output` → `loopback_xmit`
   (kernel-internal routing, no real PHY)
5. Push skb to lo's rx queue (atomic enqueue)
6. Return up the stack
7. Ring 0 → ring 3

Estimated breakdown of our 3.85 µs:
- Syscall entry/exit overhead: ~300 ns
- skb alloc + payload copy: ~500 ns
- IP layer routing: ~500 ns
- Loopback xmit + enqueue: ~500 ns
- Return path (kernel + syscall exit): ~300 ns
- Measurement / cache jitter: ~1.7 µs

## How to push this to the max

Options ordered by effort × payoff for this codebase:

### 1. monoio io_uring UDP in gateway/marketdata

rsx-dxs is runtime-free by design — `CmpReceiver` takes
bytes, not a socket. The caller (gateway, marketdata) owns
the `UdpSocket`. Replacing `std::net::UdpSocket` with
`monoio::net::UdpSocket` in the caller is the correct path;
rsx-dxs itself never gains a runtime dep.

Effort: ~50-100 LOC in gateway + marketdata only.

Expected gain: **~1.5-2 µs** off each leg by amortizing the
syscall across batched SQEs.

### 2. SQPOLL for the gateway's CMP loop

Effort: ~20 LOC in gateway on `monoio::net::UdpSocket`. Sets
`IORING_SETUP_SQPOLL` on the gateway's io_uring instance.

Expected gain: another **~500 ns** by avoiding the
`io_uring_enter` syscall entirely. Trade-off: gateway
process burns one full core (acceptable for the hot path).

### 3. sendmmsg batching

Effort: ~50 LOC in CmpSender. When multiple records are
ready, call `sendmmsg(buf_vec, n)` instead of N `sendto`s.

Expected gain: **~3 µs per record after the first** in a
batch of N. Negligible at low rate; major at sustained
order-burst rates.

### 4. AF_XDP

Effort: significant. Custom XDP program + AF_XDP socket
setup. Cross-process loopback isn't really the AF_XDP
use case — it shines for real-NIC traffic.

Expected gain: **~2-3 µs** but the wire setup complexity
is heavy and we lose normal `ss -unp` visibility.

### 5. DPDK

Effort: rewrite of the I/O layer per process. Multi-quarter.

Expected gain: **3-5 µs total** off the GW→ME→GW path.
Project spec already mentions this as the "Later" plan.

## Why our 3.85 µs is fast already

Cross-process UDP loopback at sub-4 µs is **near the floor**
for std::net + Linux 6.1 with mitigations on. The kernel's
data-path work IS ~3 µs of that; you can't make it cheaper
without bypassing it.

What you *can* do, ranked by realism, is move our 78%
send-body cost from being on the critical path to being
async (io_uring SQPOLL), or eliminate the cross-process
boundary entirely (in-process pipeline shows the 9.6 µs
floor we get when you do).

## Re-validation cadence

Re-measure when:
- Kernel jumps a major version (Linux 7.0?)
- New CPU mitigations are added (next class of speculative
  attacks)
- We port any CmpSender/Receiver to monoio::net (will
  invalidate the std::net numbers above)
- We swap to DPDK / AF_XDP

Re-run `cargo bench --bench cmp_send_breakdown_bench` and
update the "concrete head-to-head" + breakdown sections.
