# MoldUDP64

Nasdaq's UDP multicast dissemination protocol. Carries ITCH 5.0
market data feeds (TotalView, BX, PSX). Public specification, freely
implementable. The closest published peer to casting's wire shape: a
sequence-numbered, NAK-recovered, fixed-header UDP frame with a
fan-out delivery model.

Spec: https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/moldudp64.pdf

Why we include it: same protocol family as casting (UDP + seq + NAK
recovery), but with multicast fan-out and a separate retransmit
channel. Lets us bench framing overhead against a real exchange
wire protocol with a known footprint.

## Protocol

### Wire format — 20-byte downstream header

All multi-byte integers are **big-endian** (network byte order),
unlike KCP's little-endian framing.

```
Offset  Size  Field            Meaning
0       10    session          ASCII session ID (left-padded space)
10      8     seq              Sequence number of the FIRST message
                                in this packet (1-based)
18      2     msg_count        Number of messages in this packet
                                (0x0000 = heartbeat, 0xFFFF = end-of-session)
20+     var   messages...      Concatenated length-prefixed messages
```

Each downstream message inside the packet:

```
0       2     msg_len   Big-endian length of msg_data
2       N     msg_data  Opaque payload (ITCH 5.0 record, etc.)
```

Packets are sent over **UDP multicast** to a well-known group/port.
A single packet typically carries one message; bursty market events
pack multiple. MTU governs the upper bound (Nasdaq uses 1 472 B
payload to stay below 1 500 B Ethernet MTU).

Compare casting/WAL: 16-byte header (`version:u8` at offset 0,
`_pad0:u8`, `record_type:u16`, `len:u16`, `_pad1:u16`,
`crc32:u32` — a CRC32C value in the `crc32`-named field —
`_reserved:[u8;4]`) + one fixed-size `#[repr(C, align(64))]`
payload per packet. No per-packet message-count; one record per
UDP datagram by construction.

### Reliability: NAK to a separate request server

MoldUDP64 separates dissemination from retransmit:

1. **Downstream** UDP multicast carries the live stream
   (one-to-many).
2. **Request channel** is a separate UDP unicast (sometimes TCP)
   endpoint that the receiver queries with a
   `MoldUDP64 Request Packet`:

   ```
   0    10  session
   10   8   seq             First missing sequence
   18   2   msg_count       How many sequenced messages requested
   ```

3. The request server replies on the **same downstream multicast
   group** (so other receivers see the retransmission too — same
   "NAK suppression" property as Aeron multicast).

End-of-session: a packet with `msg_count = 0xFFFF` tells
receivers the stream is done. Heartbeats (`msg_count = 0`) keep
liveness without payload.

### No congestion control, no flow control

MoldUDP64 assumes a fixed-capacity multicast fabric. There is no
ACK, no window, no sender-side rate limiting. Receivers that fall
behind use the request channel to catch up; the dissemination
side never slows down.

This matches casting's design assumption (trusted, fixed-capacity
LAN). casting likewise has no wire-level flow control — it is unicast
and relies on WAL-writer backpressure (producer stalls on flush-lag
> 10 ms) plus a bounded receiver reorder buffer rather than any
receiver-advertised window on the wire.

### Latency characteristics

Public Nasdaq feed numbers (ITCH 5.0 / TotalView):
- Wire frame overhead: ~20 B + 2 B per message.
- One-way LAN latency reported by Nasdaq colo customers:
  10–30 µs (NIC-to-NIC, kernel bypass).
- The protocol itself adds essentially zero processing — parse
  header, dispatch payload.

## Relation to rsx-cast

| Dimension | MoldUDP64 | rsx-cast casting |
|---|---|---|
| Transport | UDP multicast (1:N) | UDP unicast (1:1) |
| Byte order | Big-endian | Little-endian (native x86_64) |
| Header size | 20 B (per packet) + 2 B (per msg) | 16 B (per record) |
| Multiple msgs per packet | Yes (`msg_count`) | No (one record per datagram) |
| Loss detection | Receiver (seq gap) | Receiver (seq gap) |
| Retransmit source | Separate request server | Embedded: hot ring + cold WAL |
| Retransmit channel | Out-of-band UDP/TCP to request server | Same socket (NAK + sendto) |
| Multicast NAK suppression | Yes (retransmit on group) | N/A (unicast) |
| Durable archive | External (TotalView Glimpse) | Embedded WAL (4 h) |
| End-of-session marker | `msg_count = 0xFFFF` | None (live tail forever) |
| Designed use | Market data dissemination (downstream only) | Bidirectional order flow + market data |

MoldUDP64 is the dissemination half of an exchange feed (downstream
only — no order entry). casting handles both directions in one protocol;
it bundles the request-server role into the sender via the embedded
WAL.

### Stronger than casting

- **Multicast fan-out is native.** One sender, N receivers, zero
  per-receiver state on the sender side. casting requires one
  `CastSender` instance per peer (point-to-point).
- **Multiple messages per UDP datagram.** Saves header overhead
  on bursty market events. casting pays a full 16-byte header per
  record.
- **NAK suppression in multicast** means a single retransmit
  recovers loss for the entire receiver group. casting retransmits
  per receiver.

### Weaker than casting

- **Retransmit horizon is implementation-defined.** Nasdaq's
  Glimpse service replays the start-of-day snapshot via a
  separate TCP protocol. casting's WAL is always there, always
  4 h deep.
- **Big-endian framing** costs `bswap64`/`bswap16` on x86_64
  every parse. casting is native little-endian.
- **Downstream only.** No model for order entry — Nasdaq uses
  OUCH (SoupBinTCP) for that, two protocols where casting has one.

## Benchmark

`benches/compare_moldudp64.rs::moldudp64_rtt_loopback_128b` (run
with `cargo bench -p rsx-cast --bench compare_moldudp64`) —
Criterion, loopback, 128 B payload (matches `FillRecord`),
**unicast** UDP (not multicast). MoldUDP64 stays a standalone
bench (its framing server does not fit the `compare_all`
`EchoClient` trait) but uses the same payload size and core
pinning, so the number is directly comparable.

We bench unicast for fair RTT comparison with the `compare_all`
raw_udp / kcp / tcp set. Loopback multicast on Linux is finicky
(IGMP, IP_ADD_MEMBERSHIP, route hints) and would measure kernel
multicast plumbing rather than the protocol's framing cost —
which is what we want to isolate.

Frame: 20 B downstream header + 2 B message-length + 128 B
payload = 150 B on the wire per direction. Sequence number
incremented on every send; `msg_count = 1`. The echoer parses
the header, validates the seq and message count, extracts the
payload, then frames its own MoldUDP64 packet back with the
echoer's own seq counter (a fair, full-stack parse + emit on
both sides — not a raw byte echo).

Measured p50 on Linux loopback: ~10 µs (2026-05-24) — at the
raw-UDP floor, since the header parse on both sides is tens of
ns and the ~4 µs `sendto`/`recvfrom` syscalls dominate. Not
re-run this session.

## Sources

- https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/moldudp64.pdf (official spec)
- https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/NQTVITCHspecification.pdf (ITCH 5.0, the payload format)
- https://github.com/martinsumner/moldudp64 (Erlang reference implementation, MIT)
- https://www.fixtrading.org/standards/ (FIX is not MoldUDP, but the request/dissemination split is the same pattern)
