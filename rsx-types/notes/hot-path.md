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

## No page faults — the full discipline

`mlock` pins a range of virtual address space into physical RAM (no swap/
reclaim) **and populates it** — the kernel faults the pages in and pins them at
lock time. `mlockall(MCL_CURRENT | MCL_FUTURE)` = everything mapped now + later,
the one-call startup move. Needs `CAP_IPC_LOCK` or a raised `RLIMIT_MEMLOCK`.

Two fault kinds to eliminate:
- **Major (disk/swap):** `mlockall` + disable swap (`swapoff`, or
  `vm.swappiness=0`). Locked pages never swap.
- **Minor (first touch of a freshly-mapped page, COW, the shared zero page):**
  pre-fault before the hot path — `mmap(MAP_POPULATE)`, or walk the region
  **writing** one byte per page (write, not read — to break COW and the zero
  page), or rely on `mlock`'s population.

Then the hot-path discipline:
- **Allocator:** never `malloc`/`free`/grow on the hot path — the allocator
  itself faults (`mmap`/`brk` on growth, `munmap` on free → the next alloc
  re-faults). Configure it to retain (jemalloc `retain`, large glibc
  `MALLOC_TRIM_THRESHOLD_`) or don't use it on the hot path at all.
- **THP off:** khugepaged compaction is a classic latency spike — disable
  Transparent Huge Pages (or `madvise`) and use **explicit hugepages** (which
  separately cut TLB misses — a different problem from faults).
- **Preallocate, never realloc:** allocate every buffer/pool at startup sized
  for worst case, prefault + lock, reuse via object pools / free lists /
  fixed-slot rings. Don't `realloc` (it can move data → fresh pages → faults +
  copy); reserve one large **arena** up front (prefaulted + locked) and
  **bump-allocate** within it, so growth is just advancing a pointer in
  resident memory. Genuine resizing happens during warmup, never in the loop.
- **The stack (the extra step):** the stack is lazily-grown anonymous memory;
  `mlockall(MCL_CURRENT)` only locks the pages **already mapped** — pages you
  haven't descended into yet aren't mapped, so future stack growth still
  faults. Fix = **stack warming**: at thread startup, before the hot loop,
  touch the deepest stack you'll use (a large local array that writes a byte
  per page down to max depth), then return; combined with `mlockall` those
  pages are resident + pinned. Set a fixed adequate stack size
  (`pthread_attr_setstacksize` / `ulimit -s`); on the hot path avoid `alloca`,
  large VLAs, and deep call chains to cold pages. **`rsx-types::cpu::warm_stack`
  does this** (256 KiB by default), called by `setup_hot_thread` *before*
  `mlockall` so the warmed pages get pinned.

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

## False sharing — the util + how to find it

The flip side of the coherence truth above. **False sharing** is when two
threads write two *different* variables that happen to share one cache line —
nothing is logically shared, but MESI invalidates the other core's copy on
every write, so the line ping-pongs across the interconnect each iteration.
Classic victims: a producer's tail next to a consumer's head, two per-thread
counters in one struct, a flag set by one thread and polled by another.

**The util** (`rsx-types::cache`, concentrated so layout is consistent
everywhere):
- `Padded<T>` — wraps a value so it sits alone on its own line span; give each
  independently-written datum its own `Padded`. `Deref`/`DerefMut` so it's
  transparent at the use site.
- `LINE = 64` (the real line, for layout reasoning) and `PAD = 128` (the
  alignment to *avoid* false sharing). It's 128, not 64, because Intel's
  adjacent-line prefetcher pulls lines in pairs — the destructive-interference
  unit is two lines. (Same choice as crossbeam's `CachePadded`.)

**How to find it** — don't guess, measure:
- **`perf c2c`** is the purpose-built tool. `perf c2c record -- ./prog` then
  `perf c2c report` lists **HITM** (hit-modified) cache lines — the lines being
  pulled dirty from another core — with the *offsets within the line* and the
  two functions/PIDs fighting over it. A hot line with two different writers at
  different offsets is the smoking gun.
- **Quick yes/no:** `perf stat -e mem_load_l3_hit_retired.xsnp_hitm ...` under
  load — a high HITM rate that scales with thread count = lines bouncing.
- **Static, before it ships:** `pahole -C <Struct> <binary>` prints field
  offsets, holes, and which fields share a 64 B line. Cross-check every
  `repr(C)` struct touched by more than one thread.
- **Heuristic:** any field written by thread A within 64 B of a field written
  by thread B is a candidate. Per-thread mutable state (cursors, counters,
  flags, backpressure bits) is the usual culprit — `Padded` each, or group all
  of one thread's mutable fields together and pad the boundary.

(rtrb already pads its own head/tail internally; `Padded` is for *our*
cross-thread fields — shard counters/flags, future egress-tile cursors.)

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
