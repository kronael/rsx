# I/O model: epoll (readiness) vs io_uring (completion), and the road to zero-copy

Why the gateway/marketdata use monoio (io_uring) while the casting hot path
busy-spins std UDP — and what "fully zero-copy" would actually take. Hands-on,
because the model choice is most of the latency story for a many-connection
edge.

## Two kernels, two contracts

- **epoll = readiness.** The kernel tells you a fd *can* be read; **you** do
  the `recv()` (and its copy).
- **io_uring = completion.** You submit "recv into this buffer"; the kernel
  does it (copy included) and tells you it's **done**.

That one difference — readiness vs completion — drives syscall count, which is
the budget at scale (a `recvfrom`/`sendto` is ~1–4 µs; our cast bench measured
`sendto` as ~99% of send cost).

## epoll / readiness — what tokio does

```rust
// tokio's reactor is epoll-based. The await registers read-interest;
// epoll_wait reports the fd READY; tokio wakes the task; the task then
// issues the actual read() syscall, which copies kernel -> buf.
let n = socket.read(&mut buf).await?;   // = epoll readiness  +  read() syscall  +  copy
```

So tokio **serializes readiness**: its reactor loop collects "these fds are
ready" and wakes the matching tasks; each task does its *own* `read()`
(syscall + kernel→user copy). `epoll_wait` is amortized over many fds, but the
reads/writes are **one syscall + one copy per op**. With N ready connections
you pay N read syscalls per loop.

## io_uring / completion — what monoio does

```rust
// monoio's reactor is io_uring. The await SUBMITS a recv SQE (submission-queue
// entry) with an OWNED buffer; the kernel performs recv+copy asynchronously and
// posts a CQE (completion). The task is woken with the buffer ALREADY filled —
// no separate read() syscall is ever issued by userspace.
let (res, buf) = socket.read(buf).await;   // buf already contains the bytes
```

So monoio **serializes completions**: by the time you see the CQE, the read is
done and the bytes are already in your buffer. `io_uring_enter` submits a
**batch** of SQEs and reaps a **batch** of CQEs in one syscall (amortizing
syscalls across many ops), and with **SQPOLL** a kernel thread polls the
submission ring so submission costs **zero** syscalls. monoio hands the buffer
to the kernel for the op's duration (hence `read(buf) -> (res, buf)` returning
ownership) — that's the completion model's signature in the type system.

| | epoll / tokio (readiness) | io_uring / monoio (completion) |
|---|---|---|
| reactor serializes | "fd is ready" events | "op done, data copied" events |
| who does the read+copy | the task, after wakeup (`read()` syscall) | the kernel, before wakeup |
| syscalls per op | ~1 read/write each (epoll_wait amortized) | ~0 (batched submit/reap; SQPOLL → 0 submit) |
| critical-path copy | in the task, on the hot path | in-kernel, off the task |

## Why HFT cares (and the gateway caveat)

Syscalls are the budget at 10k connections — io_uring's batching is what keeps
the edge from being syscall-bound (see `reports/20260530_loop-arch-bench.md`:
the workload was 2 syscalls/req and bound by exactly that; batching cut it to
~0.25). The completion model also removes a kernel↔user round-trip (readiness →
read → copy collapses to one completion).

**But** (the gateway lesson, stated correctly): the reactor per-lap is **fast**
(µs) and no-priority is the right default — *until the core saturates*. The ~ms
gateway response stall we measured was a **single reactor core drowning in a
synthetic order flood** (≈138k orders on one core), i.e. a **capacity** problem,
**not** a missing priority. Priority wouldn't help: on a maxed core it only
reshuffles *which* task waits — the WS writes (also on that reactor) still back
up because there aren't enough cycles; it adds no throughput. The real fixes are
**capacity** — shard reactors across cores (`SO_REUSEPORT`) so none saturates,
and give the latency-critical path its *own* unsaturated core via the egress
tile — and **work-reduction** (batch syscalls, binary not JSON, fewer copies) so
each lap does less and the core saturates at a far higher rate. Not priority,
not a flag. See the gateway Runtime Model spec.

## The road to fully zero-copy

io_uring's recv still **copies** kernel socket buffer → your buffer. Removing
that copy, in increasing aggressiveness:

1. **Registered buffers** (`IORING_REGISTER_BUFFERS`) — pre-pin user buffers so
   the kernel skips per-op page pinning. Lower overhead; the copy still happens.
2. **Provided-buffer rings + multishot recv** — the kernel picks a buffer from a
   ring per arriving datagram; no per-op buffer plumbing. Great for many small
   reads (a WS edge).
3. **Zero-copy send** (`IORING_OP_SEND_ZC` / `SENDMSG_ZC`, kernel ≥6.0) — the
   kernel transmits **directly from your buffer** (pins the pages, DMAs to the
   NIC); a later notification CQE says the buffer is reusable. No TX copy.
4. **Kernel bypass — true zero-copy RX *and* TX:**
   - **AF_XDP** — the NIC DMAs each packet directly into a user-space **UMEM**
     frame ring (memory shared by NIC/kernel/user). You read/write the packet
     **in place**: no copy, no kernel network stack, no `recvfrom`.
   - **DPDK** — a userspace poll-mode driver owns the NIC; packets land in
     hugepage mempool buffers. Fully kernel-bypass, zero-copy, poll-driven.

## RSX's zero-copy path (concrete)

The casting wire format is fixed `#[repr(C, align(64))]` records — **no
parsing**. rsx-cast already does a near-zero-copy **decode**: `try_recv_with(|hdr,
payload| …)` hands you a *borrow* of the receive buffer, and the typed access is
a single `read_unaligned` (the one remaining copy — into a register-width typed
value, an alignment necessity; keep the `&[u8]` borrow if you want a literal
zero-copy view).

The only other copy is kernel → receive-buffer. Put casting RX on **AF_XDP**:
the NIC DMAs the UDP datagram into a UMEM frame, and the decode reads the WAL
record struct **in place** from that frame → **wire → struct with one DMA and
zero copies**, no kernel stack, no `recvfrom` syscall. Pair with `SEND_ZC`/AF_XDP
TX for the response. That is the architecture's "Later: DPDK/AF_XDP swaps the
I/O layer" made concrete — the fixed-record format and in-place decode were
designed for it; only the I/O layer changes, not the SPSC rings or the engines.

**Today:** the casting hot path is std non-blocking UDP busy-spin (one copy off
the kernel buffer via `read_unaligned`); gateway + marketdata use monoio/io_uring
for the many-WS-connection side. The zero-copy steps above are the forward path,
not current state.
