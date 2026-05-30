# Hot-path design — the rules every tile follows

The low-latency design contract for RSX's pinned busy-loop tiles (risk, ME,
and the planned gateway egress tile). This is the *why*; the *how* (the shared
setup) is `rsx-types::cpu`. Applies to both kernel-resident (io_uring) and
kernel-bypass paths.

## Tier 0 — the decision above io_uring

io_uring with SQPOLL + a busy-polled CQ is excellent, **but it still traverses
the kernel TCP/IP stack and the softirq path**. The genuine bottom tier is
**kernel bypass** — DPDK, AF_XDP, Solarflare Onload, RDMA — where you poll the
NIC's RX descriptor ring directly, the NIC DMAs packets straight into userspace
hugepages, and you run TCP/UDP yourself.

- **io_uring = best kernel-resident option.**
- **Bypass = best option, period.**
- Pick the tier your latency budget demands. Everything below applies to both.

(RSX: casting hot path is std busy-spin UDP today; AF_XDP is the spec'd forward
path — see `rsx-gateway/notes/io-model.md`.)

## The core loop

One thread, pinned to an **isolated** core, tight busy loop:
`poll input → process inline → submit output`. Non-negotiables on the hot path:

- **No syscalls, no mutexes/futexes, no malloc, no logging, no page faults.**
  Preallocate everything, `mlock` it, **pre-fault** it.
- **Isolate the cores:** `isolcpus` + `nohz_full` (kill the scheduler tick) +
  steer IRQs off them (`irqaffinity`). Otherwise a timer tick or a migrated
  interrupt is a tail-latency spike.
- **NUMA-local** memory on the same node as the core *and* the NIC; **hugepages**
  to cut TLB misses.

## More submission threads? No.

Multiple threads on one io_uring ring means synchronizing the SQ — contention +
jitter; rings aren't meant to be shared. More threads buy **throughput, not
single-message latency**, and cost cross-thread coordination + cache-line
bouncing. **Thread-per-core: one ring per core, no shared submission.**

If one core can't keep up and queuing delay starts dominating latency, you do
**not** add submitters — you **shard the input at the NIC** (RSS / flow
steering) so independent flows land on independent cores, each a self-contained
loop.

## Two cores from one input — pipeline vs parallel

- **Pipeline** ("tiles joined by SPSC queues"): the right primitive, but each
  hop **adds latency** (a cross-core cache transfer + a queue). It overlaps work
  and lowers throughput-per-stage, but **raises single-message latency**. Use it
  **only when one core genuinely can't finish a message inside budget.**
- **Parallel / fan-out** (replicate the same stage, split inputs): keeps
  single-message latency low *under load* by preventing saturation, but doesn't
  speed up an individual message.
- **Split one message across two cores and re-join: almost never wins** — the
  join/sync cost usually exceeds the parallel savings.

**Default: do as much as fits inline on one core; minimize hops. Reach for the
pipeline only when forced.**

## SPSC + ownership + the cache truth

Lock-free (wait-free) **SPSC rings are the correct inter-core channel.** But
**passing a pointer does not avoid the cost** you're worried about: the payload
is hot in the producer core's L1/L2 in *modified* state; when the consumer
dereferences it, **MESI forces a coherence miss** — those 64 B cache lines get
pulled across the interconnect. Ownership transfer avoids the *copy* but **not
the coherence traffic**. The only way to avoid it is **not to cross cores** (keep
it on one core).

Given that, the counterintuitive but standard rule:

- **Small messages (≤ a few cache lines): copy inline.** Preallocate a ring of
  fixed slots and write the payload *in place* into the slot (LMAX Disruptor
  style). The consumer reads contiguous, prefetcher-friendly memory with no
  pointer-chase. A dependent load to a random heap address is often **slower**
  than transferring the data inline (you lose hardware prefetch + add a chase).
  So for bounded messages the "copy" beats the "zero-copy" pointer.
- **Large messages: pass an index into a preallocated pool** and eat the one
  coherence miss — copying would cost more. **Indices > raw pointers** (smaller,
  bounds-checkable, no allocator).

Ring hygiene:
- **Pad head and tail onto separate cache lines** — producer writes tail,
  consumer writes head; sharing a line = false sharing that bounces every iter.
- **Batch dequeues** to amortize the coherence cost over many items.
- Keep producer + consumer on cores that **share an LLC / same NUMA node** —
  cross-socket adds ~100 ns+ over the ~tens-of-ns on-socket snoop.
- **Software-prefetch the next slot** while processing the current one.

## The two corrections to remember

1. **More submitters HURT** latency — **shard the input** (RSS) instead.
2. **Cross-core movement always costs a coherence miss**, copy-or-pointer alike
   — so **small messages copy inline** into a preallocated ring, and **pipeline
   only when a single core can't meet the budget.**

## RSX status vs this contract

- ✅ Busy-spin, pinned: risk + ME (std non-blocking UDP, no runtime on the hot
  loop). Gateway egress tile is the planned addition (see gateway Runtime Model).
- ✅ SPSC rings (rtrb) for intra-process handoff; per-consumer rings.
- ⚠️ **CPU setup is being concentrated** into `rsx-types::cpu` (was duplicated
  across 5 binaries). isolcpus/nohz_full/irqaffinity + mlock + NUMA + hugepages
  are deployment concerns the shared setup should assert/warn about.
- ⛳ Forward path: AF_XDP/DPDK for casting RX/TX (kernel bypass); fixed-record
  in-place decode already designed for it.
- Audit TODO: verify no hot-path malloc/log/syscall; SPSC head/tail padding;
  small-msg-copy vs large-msg-index; batch dequeue; prefetch.
