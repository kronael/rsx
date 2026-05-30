# Blog Plan — Building a Perpetuals Exchange in Rust

Target: engineers who have built distributed systems or worked near exchange
infrastructure. Not a tutorial. A "here is what we learned and measured" post.

---

## Working titles

1. *Building a perpetuals exchange in Rust: what the numbers actually say*
2. *WAL = wire = stream: the design that makes exchange infra boring again*
3. *54 ns to match a trade: what it costs and what it doesn't*

---

## Core claim (one sentence)

> We built a complete perpetuals exchange spec-first in Rust, measured every
> layer, and the surprising finding is that the matching algorithm itself is
> almost free — the bottleneck is the network syscall, and that's solvable.

---

## Outline

### 1. What we built (300 words)

12 Rust crates, one process per concern, C structs on the wire.
No Kafka, no Protobuf, no gRPC. The data model is: every record
that hits the disk is the same bytes that go over UDP and the
same bytes that stream to downstream consumers.

Diagram: GW → Risk → ME → fills path (from BLOG.md).

### 2. The numbers up front (table)

Show the bench table before explaining anything. Readers who know
exchange systems will immediately see what's unusual.

| | |
|---|---|
| Match single fill | 54 ns |
| WAL append (pre-fsync) | 31 ns |
| casting send body (UDP) | 3.87 µs |
| Loopback RTT (GW→ME→GW) | ~10 µs (component sum) |

Cite: `rsx-book/benches/book_bench.rs`, `rsx-cast/compare/bench_report`,
`facts/cast-vs-udp-overhead.md`.

### 3. WAL = wire = stream (the interesting design decision)

The key claim. Every record: 16B header + `repr(C, align(64))` payload.
Same bytes: WAL file, casting/UDP datagram, replication/TCP replay stream.

What this enables:
- Retransmit reads from the WAL file directly (horizon = retention, not RAM)
- Audit log and retransmit buffer are the same artifact
- Downstream consumers (ML, backtesting, surveillance) read the same format
  at rest and in flight

Compare to: Chronicle Queue ([github](https://github.com/OpenHFT/Chronicle-Queue))
which has the same disk=wire insight but no UDP/NAK path. Kafka separates the
replication log from the wire format. Aeron separates term buffers (hot) from
archive (cold). replication collapses all three.

Cite: `specs/2/4-cast.md`, `specs/2/48-wal.md`, `rsx-cast/README.md`.

### 4. casting: NAK not ACK (why it matters for LAN)

ACK-based: sender infers loss from silence. ~1.5–2 RTTs to retransmit.
NAK-based: receiver detects gap immediately. ~1 RTT to retransmit.

On a trusted LAN with <0.01% loss, NAKs are rare. StatusMessage every 10 ms
is the only control-plane traffic when nothing is lost. Per-message ACK (KCP,
TCP, QUIC) burns bandwidth for work that isn't needed.

Protocol overhead bench:
- raw UDP: 9.89 µs RTT
- casting: 11.26 µs RTT  (+14% — framing + NAK bookkeeping)
- KCP spin: ~25–50 µs RTT (ACK round-trip even on loopback)
- QUIC persistent: ~200–500 µs (TLS + congestion control)

Cite: `rsx-cast/compare/kcp.md`, `rsx-cast/compare/quinn.md`,
`facts/cast-vs-udp-overhead.md`, `rsx-cast/compare/compare_all.rs`.

Prior art acknowledgement: Aeron (Real Logic) is the direct design ancestor.
[Todd Montgomery](https://github.com/tmont) designed the original PGM/multicast
stack at Informatica/29West that became LBM, then Aeron. casting is a Rust
re-implementation of the same ideas with the WAL as the retransmit source.

### 5. The matching engine: what 54 ns is made of

Dedup check: 70 ns. WAL accept: 60 ns. Match: 80 ns. WAL events: 110 ns.
Total: ~320 ns hot path.

The 54 ns is single fill, cold to hot. Under sustained load the pipeline is
the WAL flush interval (10 ms) — not the algorithm.

Key structures: slab allocator (128B slots, align(64)), CompressionMap (617K
slots instead of 20M for full tick range), FIFO doubly-linked list per level.

Cite: `rsx-book/benches/book_bench.rs`, `notes/arena.md`, `notes/align.md`,
`rsx-book/src/compression.rs`.

### 6. Fixed-point everywhere

All prices and quantities are i64 in smallest units. `Price(pub i64)`,
`Qty(pub i64)` as `#[repr(transparent)]` newtypes. IEEE 754 rounding is
eliminated at the API boundary; the matching engine never sees a float.

This matters for: determinism across replays, correctness of fill math,
interop with exchange-grade risk systems. The Hyperliquid architecture doc
([2024](https://hyperliquid.gitbook.io/hyperliquid-docs/)) makes the same choice.

### 7. What's still open

- Tile parity for gateway + marketdata (monoio reactors today, not pinned tiles)
- Measured GW→ME→GW p50/p99 under load (component sum says <50 µs, harness not yet asserted)
- casting v2 multicast (one ME → N consumers, no per-receiver copy) — spec at `specs/2/51-cmp-v2-multicast.md`
- monoio io_uring UDP in gateway (caller owns socket; rsx-cast is runtime-free by design)

### 8. The rsx-cast transport layer as a standalone crate

`cargo tree -p rsx-cast --edges normal | grep rsx-` returns empty.
Domain-agnostic. Any project that needs log-backed reliable UDP with TCP
cold-path replay can use it independently. Worked example for the blog post:
a metrics ingest pipeline using rsx-cast without any exchange domain knowledge.

---

## Benchmarks to run before publishing

- [ ] `cargo bench -p rsx-book` — confirm 54 ns / 857 ns
- [ ] `cargo bench -p rsx-cast --bench compare_all` → run `rsx-cast/compare/bench_report --md`
- [ ] `cargo bench -p rsx-cast --bench compare_kcp --bench compare_quinn`
- [ ] `cargo bench -p rsx-cast --bench cast_send_breakdown_bench` — confirm 3.87 µs
- [ ] End-to-end latency probe under sustained load (currently manual; needs automation)

---

## What NOT to include

- Fundraising language, "design partners", GTM, "exchange-in-a-box"
- Unverified claims (anything not backed by a bench or a citation)
- The `<50 µs` end-to-end claim until the sustained-load harness is done
- Internal sprint/audit artifacts (`.ship/`, CEO/CTO review scores)
