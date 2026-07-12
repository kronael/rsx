# rsx-matching/notes

Why the matching-engine tile is shaped the way it is — the design
rationale, not "how it is" (that's `ARCHITECTURE.md`) and not "how fast"
(that's `reports/`). One file per non-obvious decision, each
**Problem → Fix → Cost-it-removes**. Follows the `doc-topology` skill.

| File | Question answered |
|------|-------------------|
| [depth-independent.md](depth-independent.md) | Why match latency is flat in book depth (and where the number comes from) |
| [fifo-time-priority.md](fifo-time-priority.md) | Why fairness (first-in-first-out per level) survives match → WAL → replay |
| [trust-boundary.md](trust-boundary.md) | Why the ME does **not** re-validate its input |
| [zero-heap.md](zero-heap.md) | Why the loop allocates nothing per order |
| [single-pinned-loop.md](single-pinned-loop.md) | Why one busy-spin loop, no SPSC rings, no reactor |
| [authoritative-wal.md](authoritative-wal.md) | Why fill-path WAL appends crash but cast sends only warn |
| [one-crc-fanout.md](one-crc-fanout.md) | Why each record is framed once and fanned to all streams (the SEQ-1 story) |
| [cancel-index.md](cancel-index.md) | Why an O(1) `(user,oid)→handle` map instead of a slab scan |
| [dedup-persistence.md](dedup-persistence.md) | Why dedup is rebuilt from the WAL, not held only in memory |
| [faulted-skip.md](faulted-skip.md) | Why an inbound-order gap is skipped, not replayed or panicked on |

## The through-line

The matching engine is the **authoritative writer of fills**: once it
persists a fill, that fill happened, and every other component's state is
derived from the ME's WAL. Every decision here serves that one role from
one of two sides.

**Above the ME — trust the boundary, keep authority cheap.** The gateway
and risk tile already validated the order, so the ME does not re-check it
(`trust-boundary`). That lets the hot path stay bounded and
depth-independent: delegate the match to rsx-book's `O(1)` index
(`depth-independent`), preserve fairness for free on the way to disk
(`fifo-time-priority`), allocate nothing per order (`zero-heap`), run
inline on one pinned core with no ring hop (`single-pinned-loop`), look up
cancels in `O(1)` (`cancel-index`), and frame each record exactly once
(`one-crc-fanout`). The accept path is a fixed ~266 ns that does not grow
with the book — authority costs the same at depth 1 or 100 000 (see the
caveats in `depth-independent`).

**Below the ME — the WAL is the single source of truth.** Because
authority must survive a crash, the durable record wins over both memory
and the wire: fill-path appends crash rather than drop a record while
best-effort casts only warn (`authoritative-wal`), dedup is rebuilt from
the WAL so exactly-once holds across restart (`dedup-persistence`), one
seq counter keeps every stream contiguous (`one-crc-fanout`), and inbound
gaps are skipped because client-retry + WAL-dedup already recover them —
only the outbound fill stream, which is genuinely unrecoverable, gets its
own replication path (`faulted-skip`).

Trust the boundary above; own the record below. That is the whole tile.
