# Reliable UDP

TCP gives you reliable delivery, ordered bytes, and flow
control. It also gives you head-of-line blocking: one lost
packet stalls the stream until retransmission arrives. For a
matching engine that must keep producing fills regardless of
how fast its consumers drain them, that stall is intolerable.

casting uses UDP with a thin reliability layer bolted on top.

## How it works

Every data record carries a monotonic sequence number in its
first eight bytes. The sender keeps a 4096-entry in-memory
ring of recently sent frames. On the receiver side, a reorder
buffer (2048 entries) absorbs out-of-order arrivals.

When the receiver sees `seq = N+2` after delivering `seq = N`,
it knows `N+1` is missing. It fires a NAK to the sender —
a 64-byte control record naming the gap. The sender looks up
`N+1` in its send ring and retransmits. If the ring has already
wrapped (the gap is old), it seeks the WAL file on disk and
retransmits from there. The retransmit horizon is WAL retention:
10 minutes on the hot tier by default.

Heartbeats keep idle streams detectable. A sender that has gone
quiet for 100 ms emits a heartbeat carrying the highest sequence
number sent. If the receiver sees `highest_seq > expected_seq`
with no intervening data, it fires a NAK to close the trailing
gap. On busy streams, data records suppress heartbeats entirely —
the arriving seq doubles as the liveness signal.

NAKs are rate-limited per gap (50 ms debounce) and the sender
deduplicates retransmits within a 1 ms window. A stuck or
malicious peer cannot flood the sender with redundant work.

## No flow control

There is no flow control. The sender cannot be slowed by a
slow consumer.

This is deliberate. Market events drive the matching engine's
clock: when a trade happens, the engine produces fills
immediately and time-stamps them for price-time priority. If
the engine had to wait for the slowest downstream consumer
before producing the next fill, time priority would be
compromised and the latency of every trade would become coupled
to the latency of every consumer. With multiple consumers
(marketdata, recorder, risk), the slowest one would set the
pace for all trades on that symbol.

Instead, slow consumers recover via TCP replay. If a consumer
falls far enough behind that the NAK-retransmit budget
exhausts (8 retries × 50 ms debounce = 400 ms recovery
window), the receiver surfaces a `Faulted` state rather than
silently advancing. The consumer connects to the producer's
TCP replication server, catches up from its last delivered
sequence number, and then resumes live UDP delivery. The
producer never knew the consumer was behind.

## The tradeoff

The price of no flow control is that a receiver that cannot
keep up will fault and pay the TCP-replay cost. On a properly
sized dedicated network this should be rare. When it does
happen, the faulted consumer is invisible to the matching
engine — no stall, no slowdown, no coupling.

A related concern: per-consumer independent streams rather than
multicast. Marketdata and the recorder each get their own UDP
stream from the matching engine. This means two copies of every
byte on the wire, but it means a slow recorder cannot back-
pressure the live marketdata path. The independence is the
point.

---

Deeper: [blog/16-dxs-no-broker.md](../../blog/16-dxs-no-broker.md),
[specs/2/4-cast.md](../../specs/2/4-cast.md),
[specs/2/10-replication.md](../../specs/2/10-replication.md)
