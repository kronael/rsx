# Low-Latency IPC: RTRB, SPSC, and Beyond

## Protocol Speed Hierarchy (same machine)

| Method                              | Latency       | Notes                                    |
|-------------------------------------|---------------|------------------------------------------|
| Direct shared struct (no sync)      | ~10-20ns      | Risk of torn reads                       |
| Seqlock                             | ~30-50ns      | Reader retries; bounded by c2c latency   |
| **Shared Memory + SPSC Ring Buffer**| **~50-170ns** | Every message, ordered, no locks         |
| MPSC (e.g. crossbeam)              | ~100-300ns    | CAS contention on write side             |
| MPMC                                | ~200-500ns    | Contention on both sides                 |
| Unix Domain Sockets                 | ~1-5us        | Kernel syscall overhead                  |
| gRPC (no TLS, UDS)                 | ~50-100us     | Protobuf serialization overhead          |
| gRPC (TCP, no TLS)                 | ~100-300us    | Loopback + serialization                 |
| gRPC (TCP + TLS)                   | ~100-350us    | Per-msg with persistent connections      |

## Queue Pattern Spectrum

```
Slower <----------------------------------------------> Faster

MPMC  -->  MPSC  -->  SPSC  -->  Seqlock  -->  Direct write
 |           |          |           |               |
locks     CAS only   atomics    retry-loop      no sync
           on write  acq/rel     on read       (torn reads
            side      only                      possible)
```

- **MPMC** = Multiple Producer Multiple Consumer (locks or complex CAS both sides)
- **MPSC** = Multiple Producer Single Consumer (CAS on write side)
- **SPSC** = Single Producer Single Consumer (acquire/release atomics only)

## How SPSC Works

A circular array with two pointers, each **owned** by exactly one side:

```
        write_idx (only producer moves this)
            |
  [ ][ ][D][E][F][ ][ ][ ]
         |
     read_idx (only consumer moves this)
```

### Key Insight

- **Producer** only writes `write_idx`, reads `read_idx` to check fullness
- **Consumer** only writes `read_idx`, reads `write_idx` to check emptiness
- No two threads ever WRITE the same variable
- Result: no locks, no CAS loops, no contention

### Push Operation (Producer)

```
1. Read read_idx (Acquire)         -> "consumer is at slot 2"
2. Check: write_idx - read_idx < capacity?  -> not full
3. Write data to buffer[write_idx % capacity]
4. Store write_idx += 1 (Release)  -> consumer can now see it
```

### Pop Operation (Consumer)

```
1. Read write_idx (Acquire)        -> "producer is at slot 6"
2. Check: read_idx != write_idx?   -> data available
3. Read data from buffer[read_idx % capacity]
4. Store read_idx += 1 (Release)   -> producer can now reclaim slot
```

One atomic load + one atomic store per operation. No retries. ~50ns.

## rtrb (Real-Time Ring Buffer)

Rust crate implementing a **wait-free** SPSC ring buffer (stronger than lock-free —
every operation completes in bounded steps, critical for real-time/audio).

### Features

- **Wait-free** SPSC queue (every op completes in bounded steps)
- No heap allocation after creation
- `no_std` compatible (with `alloc`)
- Cache-line padded read/write indices (avoids false sharing)
- Built by the Rust audio community

### Usage (Same Process, Two Threads)

```rust
let (mut producer, mut consumer) = rtrb::RingBuffer::new(8192);

// Risk engine thread
producer.push(order).unwrap();

// Orderbook thread (busy-spin on dedicated core)
loop {
    if let Ok(order) = consumer.pop() {
        process(order);
    }
}
```

### Limitation: Single Process Only

rtrb moves data between **threads**, not **processes**. It cannot be placed in
shared memory (`mmap`/`shm_open`) out of the box.

| Scenario                                  | rtrb? |
|-------------------------------------------|-------|
| Risk + orderbook in same process (threads) | Yes   |
| Separate processes, same machine           | No    |

For cross-process, options:
- Hand-roll SPSC over `mmap` (~50 lines with atomics)
- Aeron IPC (has Rust bindings, designed for this)
- `shared_memory` crate + custom SPSC

## What Beats SPSC?

Only by removing the queue concept entirely:

### Seqlock (~30-50ns)

Use when you only care about the **latest state**, not every message.
Bounded by core-to-core cache coherence latency (~34-52ns on modern x86, same socket).

```
Writer:                          Reader:
  seq++ (odd = writing)            loop {
  write data                         s1 = seq.load()
  seq++ (even = done)                if s1 is odd -> retry
                                     read data
                                     s2 = seq.load()
                                     if s1 == s2 -> data valid
                                   }
```

Trade-off: reader may see stale data or retry. Messages can be skipped.

### When to Use What

| Need                        | Pattern            |
|-----------------------------|--------------------|
| Every message, ordered      | SPSC ring buffer   |
| Only latest value matters   | Seqlock            |
| Multiple writers            | MPSC (crossbeam)   |
| Multiple readers + writers  | MPMC               |

For risk engine -> orderbook: **SPSC is correct** (can't skip risk checks).

## SSL/TLS Considerations

### Same Machine: No

- OS provides process isolation
- Attacker with local access already owns the box
- TLS adds ~50-100us per call for no security gain

### Cross Machine

| Network          | Recommendation                              |
|------------------|---------------------------------------------|
| Private VLAN     | Skip TLS (or use IPsec at network layer)    |
| Shared/untrusted | Use TLS                                     |

Many firms use **IPsec at the network layer** so the application doesn't pay
per-message TLS cost.

## Performance Optimizations

- **Cache-line padding**: Keep write_idx and read_idx on separate 64-byte cache lines (avoids false sharing)
- **Power-of-2 capacity**: Use bitwise AND instead of modulo (`idx & (cap - 1)` vs `idx % cap`)
- **Flat structs, no serialization**: Both sides compile the same struct definition. No protobuf/flatbuffers.
- **Busy-spin reader**: Don't sleep/epoll. Spin on the read index. Pin to a dedicated core.
- **Core pinning**: Pin producer and consumer to specific cores, same NUMA node.
- **Huge pages**: 2MB huge pages for shared memory region (reduces TLB misses).
