# Network Edge I/O

Gateway and marketdata are monoio/io_uring services because they
are I/O-bound fan-in/fan-out edges, not compute tiles. A pinned
busy-spin loop does not beat a kernel boundary; it just spends a
core waiting to pay the same syscall.

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

---

Deeper: [reports/20260530_loop-arch-bench.md](../../reports/20260530_loop-arch-bench.md),
[reports/20260703_cast-benches.md](../../reports/20260703_cast-benches.md),
[docs/benches.md](../../docs/benches.md),
[rsx-gateway/notes/io-model.md](../../rsx-gateway/notes/io-model.md),
[specs/2/11-gateway.md](../../specs/2/11-gateway.md),
[specs/2/16-marketdata.md](../../specs/2/16-marketdata.md)
