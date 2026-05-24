# SPSC ring buffers

Sources: [rtrb crate](https://docs.rs/rtrb), [Dmitry Vyukov — SPSC queue](https://www.1024cores.net/home/lock-free-algorithms/queues/bounded-mpmc-queue),
[Martin Thompson — Mechanical Sympathy blog](https://mechanical-sympathy.blogspot.com/2011/09/single-writer-principle.html),
[LMAX Disruptor paper](https://lmax-exchange.github.io/disruptor/disruptor.html).

## Latency hierarchy (same machine)

| Mechanism | Latency | Notes |
|---|---:|---|
| Direct write (no sync) | ~10–20 ns | unsafe, torn reads possible |
| Seqlock | ~30–50 ns | OK for latest-value-only (reader may retry) |
| **SPSC ring (rtrb)** | **~50–170 ns** | every message, ordered, no locks |
| MPSC (crossbeam) | ~100–300 ns | CAS on write side |
| Unix domain socket | ~2–10 µs | kernel round-trip |

## Why SPSC is lock-free

Single producer owns `write_idx`; single consumer owns `read_idx`. Neither
ever contends a write with the other side — one atomic load + one atomic store
per operation, no CAS loops.

```
producer: read read_idx (Acquire) → write slot → store write_idx+1 (Release)
consumer: read write_idx (Acquire) → read slot → store read_idx+1 (Release)
```

## RSX usage

`rsx-risk` uses [`rtrb`](https://docs.rs/rtrb) rings for intra-process IPC between
tile threads: one ring per consumer (risk→ME, ME→risk fills, BBO→risk). Slow consumers
(mktdata) get their own ring so they can't stall the fast path. Ring full = producer
stalls (backpressure); no silent drops.

## Seqlock — when latest value is enough

For BBO / mark price broadcast where only the current value matters and skipping
an intermediate update is fine:

```
writer: seq++ (odd) → write data → seq++ (even)
reader: loop { s = seq.load(); read data; if seq.load() == s && s is even: done }
```

Bounded by core-to-core latency (~34–52 ns, same socket). Readers may retry on
concurrent writes. See [Linux kernel seqlock](https://www.kernel.org/doc/html/latest/locking/seqlock.html).
