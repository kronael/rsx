# X Thread: RSX Exchange Update (2026-02-16)

## Thread

1/ Building a perpetuals exchange in Rust. 9 crates,
813 tests, ~35k LOC. Here's where we are.

2/ The matching engine runs at 180ns per insert, 120ns
per fill, 90ns cancel. Single-threaded, pinned core,
zero heap on hot path. Cache-line aligned structs,
fixed-point i64 math, slab allocator.

3/ We deleted the serialization layer. WAL = wire =
memory format. Same C struct from disk to network to
memory. No FlatBuffers, no protobuf. Just memcpy +
CRC32. Saved 150ns per message.

4/ Every producer is its own broker. No Kafka. Matching
engine serves its WAL over TCP. Consumers connect,
send start sequence, get raw records streamed. 10us
latency vs Kafka's 10ms.

5/ The orderbook fits in 15MB. Distance-based compression
zones: 1:1 resolution near mid-price, 1:1000 far away.
20M theoretical price levels compressed to 617K slots.
Bisection lookup in 2-5ns.

6/ Numeric safety everywhere. saturating_mul on zone
boundaries, checked_mul on notional, try_from on i128
downcasts, bounds checks on array access. No silent
overflow, no wrapping, no UB.

7/ Built a dev dashboard with HTMX + FastAPI. Two Python
files, zero JavaScript. 10 screens: process control,
live orderbook, order injection, invariant verification,
fault injection, unified logs. 156 Playwright tests.

8/ The dashboard replaced six terminals. Change code,
click "Build & Start All", submit test order, check
orderbook, run invariants. 30 seconds from code change
to verified correctness.

9/ Smart search in the log viewer: type "gateway error
timeout" and it extracts process=gateway, level=error,
search=timeout. One input replaces three dropdowns.
Keyboard shortcuts: / to focus, Ctrl+L to clear.

10/ Fills are sacred (0ms loss, fsync before downstream).
Orders are ephemeral (lost on crash, user retries with
same cid). Position = sum of fills, always recoverable
from WAL replay.

11/ 813 Rust tests + 156 Playwright tests. Zero failures.
Hostile testing: assume every component lies. Position =
sum of fills tested in every scenario. Backpressure never
drops. Exactly-one completion per order.

12/ All specs written before code. 35+ spec documents in
specs/1/. Implementation validates spec. Tests encode
invariants. Code follows architecture.

Spec-first. Delete what you can. Test what remains.
