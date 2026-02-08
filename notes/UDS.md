# UDS vs Shared Memory

Two main IPC mechanisms for same-host communication. They sit at different points on the latency/complexity spectrum.

## Overview

```
                  UDS                          Shared Memory
          ┌──────────────┐              ┌──────────────────────┐
 Process A│  write(sock)  │              │   mmap'd region       │
          │       │       │              │  ┌────────────────┐  │
          │       ▼       │              │  │  data written   │  │
          │    kernel      │◄── copy ──► │  │  directly by A  │  │
          │    buffer      │              │  │  read by B      │  │
          │       │       │              │  └────────────────┘  │
          │       ▼       │              │                      │
 Process B│  read(sock)   │              │  (no kernel transit) │
          └──────────────┘              └──────────────────────┘
```

## Head-to-Head Comparison

| Aspect | UDS (Unix Domain Socket) | Shared Memory (shmem) |
|---|---|---|
| **Mechanism** | Kernel-mediated stream/datagram socket | Memory-mapped region visible to both processes |
| **Data path** | write → kernel buffer → read (2 copies) | Direct read/write to same physical pages (0 copies) |
| **Typical latency** | ~2–10 µs | ~50–200 ns |
| **Throughput** | ~2–6 GB/s | ~10–50+ GB/s (memory bandwidth bound) |
| **Syscalls per message** | 2 (write + read), reducible with io_uring | 0 after setup (just memory loads/stores) |
| **Synchronization** | Kernel handles it (socket is inherently ordered) | You handle it (atomics, futexes, spinlocks) |
| **Backpressure** | Built-in (socket buffer full → write blocks) | You implement it (ring buffer full → spin/wait) |
| **Framing** | SOCK_STREAM needs manual framing; SOCK_SEQPACKET gives message boundaries | You define the protocol entirely |
| **Security** | Filesystem permissions + SO_PEERCRED | Filesystem permissions on shm_open / mmap |
| **Failure isolation** | Clean — if one side crashes, the other gets EOF/error | Dangerous — if writer crashes mid-write, reader sees corrupt data |
| **Complexity** | Low — standard socket API | High — manual sync, memory layout, crash recovery |

## When to Use Each

### Use UDS when:

- **Simplicity matters** — socket API is well-understood, debuggable with `socat`/`strace`
- **Structured protocols** — gRPC/HTTP over UDS gives you serialization, streaming, deadlines for free
- **Sidecar communication** — Envoy, Istio, and most service meshes use UDS
- **Moderate throughput** — a few GB/s is sufficient (most microservices)
- **You want crash safety** — kernel buffers mean a writer crash doesn't corrupt the reader

### Use shared memory when:

- **Nanosecond latency is required** — market data feeds, matching engines, real-time audio/video
- **High throughput** — moving large payloads (frames, tensors, buffers) without copying
- **Tight loops** — busy-polling a shared ring buffer is faster than syscall-based notification
- **You control both sides** — same team, same deploy, can handle the complexity
- **Zero-copy is critical** — producer writes once, consumer reads in-place

## Hybrid Pattern: Shared Memory + UDS Notification

A common architecture combines both — shared memory for data, UDS for signaling:

```
Producer                                          Consumer
   │                                                  │
   ├── write payload to shmem ring buffer ──────────► │ (zero-copy)
   │                                                  │
   ├── write 1 byte to UDS ─── "data ready" ───────► │ (wake notification)
   │                                                  │
   │                                              read from shmem
```

This gives you:
- **Zero-copy data transfer** via shared memory
- **Efficient wakeup** via UDS (no busy-spinning, epoll-friendly)
- **Backpressure** via the ring buffer being full

This is essentially what high-performance systems like LMAX Disruptor, DPDK, and io_uring use conceptually — a shared data plane with a separate signaling mechanism.

## Rust Examples

### UDS (tokio)

```rust
use tokio::net::{UnixListener, UnixStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// Server
let listener = UnixListener::bind("/tmp/my.sock")?;
let (mut stream, _) = listener.accept().await?;
let mut buf = [0u8; 1024];
let n = stream.read(&mut buf).await?;

// Client
let mut stream = UnixStream::connect("/tmp/my.sock").await?;
stream.write_all(b"hello").await?;
```

### Shared Memory (POSIX shmem)

```rust
use std::ptr;
use libc::{shm_open, ftruncate, mmap, PROT_READ, PROT_WRITE, MAP_SHARED, O_CREAT, O_RDWR};
use std::ffi::CString;

let name = CString::new("/my_shm").unwrap();
let size = 4096;

unsafe {
    let fd = shm_open(name.as_ptr(), O_CREAT | O_RDWR, 0o600);
    ftruncate(fd, size as i64);
    let ptr = mmap(
        ptr::null_mut(), size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0,
    ) as *mut u8;

    // Write
    ptr::write_volatile(ptr, 42);

    // Read (from another process mapping the same region)
    let val = ptr::read_volatile(ptr);
}
```

### Shared Memory Ring Buffer (lock-free SPSC)

```rust
use std::sync::atomic::{AtomicUsize, Ordering};

#[repr(C)]
struct ShmRingBuffer {
    head: AtomicUsize,  // written by producer
    tail: AtomicUsize,  // written by consumer
    capacity: usize,
    // data follows in the mmap'd region
}

impl ShmRingBuffer {
    fn push(&self, data: &[u8]) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let next = (head + 1) % self.capacity;
        if next == tail { return false; } // full

        // write data at head offset...
        self.head.store(next, Ordering::Release);
        true
    }

    fn pop(&self, buf: &mut [u8]) -> bool {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head { return false; } // empty

        // read data at tail offset...
        self.tail.store((tail + 1) % self.capacity, Ordering::Release);
        true
    }
}
```

## Performance Spectrum (Same-Host IPC)

```
Fastest                                                    Simplest
   │                                                          │
   ▼                                                          ▼
Shared Mem    Shared Mem     UDS          UDS + gRPC     TCP loopback
(spin-poll)   + eventfd    (raw)         (protobuf)      + gRPC
 ~50 ns       ~200 ns      ~2-10 µs      ~10-50 µs       ~50-100 µs
```

## Related

- [SMRB.md](SMRB.md) — SPSC ring buffer design for shared memory IPC
