# Aeron

Open-source Java/C++ reliable UDP transport by Real Logic (Martin
Thompson, Todd Montgomery). The direct design ancestor of rsx-dxs CMP.
Widely deployed in HFT and trading systems. Acquired by Adaptive
Financial Consulting 2022.

Repo: https://github.com/aeron-io/aeron (Apache-2.0)

## Protocol

### Wire format

Fixed 32-byte base frame header. Data frames:
```
 0-3    frame_length   (includes header)
 4      version
 5      flags          (FRAGMENT_BEGIN, END, etc.)
 6-7    type           (DATA=0x01, PAD=0x02, NAK=0x03, SM=0x04, ...)
 8-11   term_offset    (byte offset within the term buffer)
12-15   session_id
16-19   stream_id
20-23   term_id
24-27   reserved_value
28-31   (depends on frame type)
32+     payload
```

Encoding: little-endian. Three rotating 64 MB "term buffers" per stream.
Position = term_id × term_length + term_offset. The position abstraction
unifies flow control and gap detection.

### NAK-based reliability

Aeron is NAK-based like CMP. Loss detection is at the receiver:
- Receiver detects gap when expected term_offset not received.
- **Unicast**: NAK sent immediately (no delay). Single receiver, no
  NAK implosion risk.
- **Multicast**: NAK sent after a randomized backoff to elect a single
  sender per gap (NAK suppression). Prevents implosion.
- Sender retransmits from in-memory term buffer.

This is the same NAK model as CMP. Retransmit path is ~1 RTT.

### Retransmit source: in-memory term buffers only

Aeron's retransmit horizon is bounded by the in-memory term buffer
depth (typically 3 × 64 MB = 192 MB per stream). Once the term is
overwritten, the gap is unrecoverable **without Aeron Archive**.

Aeron Archive is a separate sidecar service that records streams to
disk and supports replay. But it is a separate component, not embedded
in the sender.

rsx-dxs difference: the WAL is embedded in every producer. There is
no separate archiver — retransmit falls through to the WAL directly in
`CmpSender::retransmit_from_wal()` when the hot ring misses. The
retransmit horizon is WAL retention (48 h), not buffer depth.

### Media driver

Aeron uses a separate **media driver process** that owns all UDP
sockets and shared-memory term buffers. Application code communicates
with the driver via IPC (lock-free SPSC rings in shared memory). This
adds one extra hop vs CMP, which embeds the UDP socket directly in
the application thread.

CMP has no media driver — `CmpSender::send()` calls `sendto()` directly
from the application thread.

## Relation to rsx-dxs

| Dimension | Aeron | rsx-dxs CMP |
|---|---|---|
| NAK-based | Yes | Yes |
| Retransmit source | In-memory term buffers | Hot ring + cold WAL (48 h) |
| Media driver | Separate process (shared-mem IPC) | None (embedded) |
| Multicast | Yes | No (per-pair unicast) |
| Congestion control | Optional (configurable) | None |
| Session setup | SETUP / OFFER handshake | None (sendto, zero setup) |
| Language | Java/C++ (Rust: community binding) | Rust (native) |
| IPC mode | Shared memory (zero-copy between driver+app) | SPSC rings (rtrb) |
| Suitable for | LAN + WAN, trusted + untrusted | Trusted LAN only |

CMP is Aeron simplified for a single trust assumption (LAN), a single
topology (unicast), and a single language (Rust). The NAK model is
identical; everything else is stripped.

## Published benchmark numbers

AWS 2025 (c6in.16xlarge, ENA networking):

| Load | Percentile | Open Source | Premium (kernel bypass) |
|---|---|---|---|
| 100 k msg/s | P50 | 21–22 µs | 24–25 µs |
| 100 k msg/s | P99 | 32–43 µs | 29–30 µs |
| 1 M msg/s | P50 | 30–35 µs | 30–31 µs |
| 1 M msg/s | P99 | 57–84 µs | 39–40 µs |

Source: https://aws.amazon.com/blogs/industries/aeron-on-aws-2025-performance-benchmark-results/

rsx-dxs CMP send body: 3.87 µs (loopback, cmp_send_breakdown_bench,
commit dfe2ef4). Cross-process RTT (monoio + tokio + 100 µs sleep):
~1 128 µs. The 100 µs sleep is a known bug (see sleep audit); fixing
it would bring cross-process RTT to ~50–100 µs range — competitive
with Aeron open source.

## Direct benchmark

Aeron requires the JVM media driver. Not directly benchmarkable in
this Rust workspace. Published AWS numbers above are the reference.

See `../benches/udp_rtt_bench.rs` for the raw UDP floor that Aeron
builds on.

Sources: https://github.com/aeron-io/aeron,
https://github.com/real-logic/aeron/wiki/Transport-Protocol-Specification,
https://aws.amazon.com/blogs/industries/aeron-on-aws-2025-performance-benchmark-results/
