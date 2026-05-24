# Unix domain sockets vs shared memory

Sources: [UNIX Network Programming vol.1 §15](https://www.informit.com/store/unix-network-programming-volume-1-9780131411555),
[Linux man shm_open(3)](https://man7.org/linux/man-pages/man3/shm_open.3.html),
[Aeron IPC transport](https://github.com/real-logic/aeron/wiki/IPC-Channel).

## At a glance

| | UDS | Shared memory |
|---|---|---|
| Data path | write → kernel buffer → read (2 copies) | direct R/W to mapped pages (0 copies) |
| Latency | ~2–10 µs | ~50–200 ns |
| Sync | kernel-provided | you implement (atomics, ring buffer) |
| Backpressure | socket buffer full → write blocks | ring buffer full → spin/wait |
| Crash safety | clean EOF on peer crash | corrupt shared region if writer crashes mid-write |
| Complexity | low (standard socket API) | high (layout, sync, crash recovery) |

## When to use which

**UDS**: sidecar IPC (Envoy, containerd), gRPC/HTTP over socket, anything where
simplicity and debuggability (`socat`, `strace`) matter more than nanoseconds.

**Shared memory**: market data feeds, matching engines, real-time audio — anywhere
you need sub-microsecond hand-off and control both sides.

## Hybrid pattern

Data over shared memory ring (zero-copy), wakeup notification over UDS or eventfd.
This is the approach used by [LMAX Disruptor](https://lmax-exchange.github.io/disruptor/disruptor.html),
[Aeron IPC](https://github.com/real-logic/aeron/wiki/IPC-Channel), and
[io_uring](https://unixism.net/loti/) (shared SQE/CQE rings + one `io_uring_enter` syscall).

## RSX

Intra-process IPC uses `rtrb` SPSC rings (see `notes/smrb.md`), not UDS or shmem.
Cross-process uses casting/UDP hot path and replication/TCP cold path — no shared memory
across process boundaries.
