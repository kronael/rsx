# Network Edge I/O

Gateway and marketdata are monoio/io_uring services because they
are I/O-bound fan-in/fan-out edges, not compute tiles. A pinned
busy-spin loop does not beat a kernel boundary; it just spends a
core waiting to pay the same syscall. The tradeoff is the same as
[tiles-and-pinning](02-tiles-and-pinning.md): runtime first, hand-roll
only when the last-mile modes matter.

## Why batching beats spinning

The per-request cost at the edge is dominated by syscalls. The cast
send breakdown measures `sendto_144b_loopback` at 3.54 us, about
100x the ~36 ns userspace framing path and roughly 99% of send
cost. Another send breakdown reports 3 846 ns for `sendto` out of
3 874 ns total.

That is why a busy-spin tile loses on gateway-shaped I/O. In
`loop-arch-bench`, the 2-core monoio-sharded variant reached
120 671 req/s; the 2-core busy-spin-tile variant reached
57 232 req/s. Both still paid about 2 syscalls/request. The
winning lever is fewer crossings, not hotter spinning.

monoio's io_uring reactor batches submissions: many SQEs go through
one `io_uring_enter`, and many CQEs are reaped together. That
amortizes the boundary across ready connections. A compute tile
helps when the work is a 60 ns match; it does not remove a 3.5 us
send syscall.

## Many events, one polling core

SQPOLL moves submission polling into the kernel. The ring is created
with `IORING_SETUP_SQPOLL`; a dedicated kernel thread watches the
submission queue, so userspace can make SQEs visible without an
`io_uring_enter` submit syscall. That is how the edge sustains a
high event rate. The cost is blunt: the kernel thread busy-polls a
core, so RSX should enable it only from the dedicated-core config,
never from a manual "fast mode" flag.

SQPOLL is not the whole win. Pair it with multishot recv and
registered/provided buffers: 1 recv submission can produce many
packet completions, and buffers are pre-pinned or supplied from a
ring instead of set up per packet. Then add egress coalescing:
`sendmmsg` or GSO packs many outbound bytes into 1 kernel crossing.
The point is fewer crossings and less per-event setup, not a hotter
spin loop.

## The scaling ladder

First, use multishot recv with registered/provided buffers. The
kernel can keep receiving into a buffer ring without per-message
buffer setup, and registered buffers skip per-op page pinning.

Second, register fds. Registered files remove per-op fd table work
on hot sockets.

Third, enable SQPOLL only when the deployment assigns a dedicated
core. SQPOLL burns a polling kernel thread so submissions can avoid
an enter syscall; it is gated on the dedicated-core config, not a
manual "go faster" flag.

Fourth, coalesce egress. `sendmmsg` and GSO reduce sends/request by
packing more outbound bytes into each kernel crossing. The
`loop-arch-bench` batched-syscall variant cut echo syscalls/request
from 2.00 to about 0.23.

## How capacity scales

Capacity scales by sharding reactors with `SO_REUSEPORT`. The
kernel hashes each connection's 4-tuple to one listener, so shards
are per-core reactors selected by flow hash, not round-robin
dispatch. Adding cores adds accept/read/write capacity close to
linearly until another resource saturates.

The ms-scale WebSocket failure mode is not solved by a compute
tile. It happens when response bytes sit behind a ms-granular
egress poll. Gateway and marketdata use per-connection/per-client
notify-based egress wake: the producer signals the handler
immediately after queuing outbound bytes. The wake path adds 0
cores and avoids the timer wait.

The tradeoff is operational. io_uring asks for Linux support,
registered resources need lifecycle discipline, and SQPOLL consumes
a core. RSX pays those costs at the many-connection edge, where the
unit of work is a 3.5 us kernel crossing, not a 60 ns match.

## Where hand-rolling starts

monoio is a generic io_uring runtime. In monoio 0.2.4, the I/O op
path is generic over `OpAble`, not a per-op `dyn` trait-object
virtual call in the driver. The overhead that remains is generic
reactor dispatch, an operation slab lookup, future polling, and
waker bookkeeping. That is small: a few ns-scale operations are
marginal next to a 3.5 us syscall.

The real reason to hand-roll is indirect. A bespoke io_uring loop can
drop the generic reactor, monomorphize the exact ops it needs, handle
SQEs/CQEs directly, and expose modes a general runtime may not expose
or may not compose for RSX's workload: SQPOLL with CPU placement,
registered files and buffers, provided-buffer rings, multishot recv,
and GSO. The direct saving is small; the mode unlock is large. That
is the I/O version of tiles: monoio is the good default, and
hand-rolling is what you do when you need both the latency floor and
the throughput ceiling.

## The tile bridge

A pinned compute tile must not block. `io_uring_enter` can block when
submitting and waiting for completions, so waiting inside the
matching tile would turn a network stall into a book stall. That is
why adding io_uring to matching is not free: the socket and polling
must live somewhere.

There are 2 workable shapes. First, a separate I/O thread owns the
ring, does recv/send, and hands decoded bytes to the tile over an
SPSC ring. The compute tile stays a 60 ns-class loop; the I/O thread
is allowed to block in the kernel. Second, SQPOLL lets the kernel own
the ring polling, so the tile can publish SQEs without the submit
syscall. That still spends a polling core. Either way, the compute
tile does not wait in `io_uring_enter`; an I/O thread or a kernel
thread owns the wait.

---

Deeper: [reports/20260530_loop-arch-bench.md](../../reports/20260530_loop-arch-bench.md),
[reports/20260703_cast-benches.md](../../reports/20260703_cast-benches.md),
[docs/benches.md](../../docs/benches.md),
[rsx-gateway/notes/io-model.md](../../rsx-gateway/notes/io-model.md),
[specs/2/11-gateway.md](../../specs/2/11-gateway.md),
[specs/2/16-marketdata.md](../../specs/2/16-marketdata.md)
