# Casting

Casting is RSX's internal transport — the wire between processes.
Two ideas make it cheap: one byte layout serves memory, disk, and
wire at once; and a thin reliability layer over UDP that never lets
a slow consumer stall the producer.

## WAL is wire is stream

Most systems keep three representations of the same data — an
in-memory struct, a wire encoding, a disk format — and pay to
convert between them (FlatBuffers ~150 ns/message, hand-rolled
msgpack ~80 ns). Across five hops (gateway → risk → matching → risk
→ gateway) that adds up.

Casting keeps **one** representation. Every record — fills, BBO
updates, order events — is a `#[repr(C, align(64))]` struct with
explicit field order and padding. The same bytes that sit in the
matching engine's event buffer are written verbatim to the WAL,
sent verbatim in a UDP datagram, and streamed verbatim over TCP for
replay. No encoder, no decoder — a CRC32C over the payload, and
that is all. A record on the wire is a single `memcpy` into a
socket buffer the kernel would have to do anyway.

A WAL append is a 16-byte header then the raw struct bytes. The
header carries a version byte (at offset 0, so a receiver can gate
on it before reading anything else), record type, payload length,
and the CRC. Measured: `WalWriter::prepare` + `append_framed` = 31 ns;
`FillRecord` encode 23 ns, decode 9 ns.

The cost is a frozen layout: append-only, pad-reuse. You cannot
reorder or remove fields. A new field fills a padding slot or
extends the struct; a genuinely breaking record change gets a new
`record_type` (`RECORD_FILL_V2`) that old consumers ignore. The
header's version byte is reserved for changes that break framing
itself (header layout, CRC algorithm, alignment) — a stop-the-world
upgrade that has happened once. Little-endian x86_64/aarch64 only,
checked at compile time. The `align(64)` and `_pad` fields are
load-bearing: treat these structs as a wire spec, because they are.

## Reliable UDP, no flow control

TCP would give reliable ordered delivery — and head-of-line
blocking: one lost packet stalls the stream. A matching engine must
keep producing fills regardless of how fast consumers drain, so
casting puts a thin reliability layer over UDP instead.

Every record carries a monotonic sequence number in its first eight
bytes. The sender keeps a 4096-entry ring of recent frames; the
receiver a 2048-entry reorder buffer. A gap (`seq = N+2` after `N`)
fires a NAK naming the missing seq; the sender retransmits from its
ring, or — if the ring already wrapped — seeks the WAL on disk (the
retransmit horizon is WAL retention, ~10 min on the hot tier). Idle
streams stay detectable via a 100 ms heartbeat carrying the highest
seq; on busy streams the arriving seq is the liveness signal and
heartbeats are suppressed. NAKs are debounced (50 ms/gap) and
retransmits deduplicated (1 ms window), so a stuck peer can't flood
the sender.

There is **no flow control** — deliberately. Market events drive the
engine's clock: a trade produces fills immediately, time-stamped for
price-time priority. If the engine had to wait for the slowest
consumer before the next fill, every trade's latency would couple to
every consumer's, and the slowest of {marketdata, recorder, risk}
would set the pace for the whole symbol. Instead, a consumer that
falls past the NAK budget (8 retries × 50 ms ≈ 400 ms) goes
`Faulted`, connects to the producer's TCP replication server, catches
up from its last seq, and resumes live — the producer never knew it
was behind. Streams are per-consumer, not multicast: two copies of
every byte, so a slow recorder can't back-pressure live marketdata.
The independence is the point.

---

Deeper: [blog/12-deleted-serialization.md](../../blog/12-deleted-serialization.md),
[blog/16-dxs-no-broker.md](../../blog/16-dxs-no-broker.md),
[blog/dont-yolo-structs-over-the-wire.md](../../blog/dont-yolo-structs-over-the-wire.md),
[specs/2/4-cast.md](../../specs/2/4-cast.md),
[specs/2/48-wal.md](../../specs/2/48-wal.md),
[specs/2/10-replication.md](../../specs/2/10-replication.md)
