# Raw UDP

Baseline: no reliability, no framing, no CRC, no retransmit. The
absolute floor for any protocol built on UDP. Anything more
capable than this — ordering, gap detection, retransmit, durability
— costs more than the numbers below.

## Wire format

There is none. UDP has only an 8-byte transport header
(RFC 768):

```
Offset  Size  Field
0       2     Source port
2       2     Destination port
4       2     Length (header + payload)
6       2     Checksum (optional on IPv4, mandatory on IPv6)
```

After the 8-byte header the kernel hands the application the
raw bytes the sender wrote with `sendto()`. There is no
sequence number, no length-prefix beyond the UDP `Length`
field, no message-vs-message framing across multiple
datagrams — every `sendto` is one datagram is one `recvfrom`
on the receiver.

## Protocol

`std::net::UdpSocket::send_to` / `recv_from`. OS kernel routes
the datagram through the loopback network stack. Nothing else.

Loopback path: user → `sendto` syscall → kernel socket buffer
→ loopback NIC driver (virtual) → kernel socket buffer →
`recvfrom` syscall → user.

## Guarantees

| Dimension | Raw UDP | rsx-cast casting |
|---|---|---|
| Delivery | Best-effort (may drop) | Reliable (NAK + WAL retransmit) |
| Ordering | Unordered (may reorder) | Per-stream FIFO (seq monotonic) |
| Duplicates | May duplicate | Dedup in receiver via seq |
| Framing | None (one datagram = one recvfrom) | 16-byte `WalHeader` + fixed-size payload |
| Integrity | UDP checksum (optional, weak) | CRC32C over each record |
| Durability | None | WAL on disk, 4 h retransmit horizon |
| Flow control | None (sender can overrun receiver) | None on the wire; producer stalls on WAL flush-lag; bounded reorder buffer |
| Connection state | None | seq + NAK list + idle-only heartbeat |

Everything in the casting column is layered above the kernel UDP
socket. Each row is a cost measured against the raw-UDP floor.

## Relation to rsx-cast

casting builds on raw UDP. The cost of casting above this baseline is:

- 16-byte `WalHeader` framing + CRC32C verification
- `send_ring` slot write (WAL record caching for hot-tier retransmit)
- NAK handling on a detected gap + an idle-only heartbeat (100 ms)
- Sequence number assignment

Measured overhead (loopback, 128 B payload):

```
raw UDP RTT      8.90 – 11.01 µs  (compare_all::raw_udp_128b, re-run 2026-07-01)
casting RTT      8.36 – 10.47 µs  (cast_rtt_bench cmp_rtt_fill_echo, re-run 2026-07-01)
casting send body    ~4.10 µs     (one-way; cast_send_breakdown_bench, 2026-05-24)
  └─ sendto syscall: 4.07 µs (99.4%)
  └─ userspace (CRC32C + framing + ring copy): ~26 ns
```

**The earlier "~2 µs" raw-UDP baseline claim was wrong** for this
host — see `facts/cast-vs-udp-overhead.md` for the full
measurement, attribution, and walk-back. Summary: the `sendto`
syscall dominates 99 % of casting's per-send cost; casting's userspace
work (CRC32C + WalHeader + ring cache) adds ~26 ns, not microseconds.

Sender + echoer are pinned to cores 2/3 in every RTT bench
(`core_affinity`), which tightened the casting distribution by
10–40% vs the pre-pinning baseline (see the facts file).

## Benchmark

`benches/compare_all.rs::raw_udp_128b` (run with `cargo bench -p
rsx-cast --bench compare_all`). The standalone `compare_udp.rs`
was folded into `compare_all.rs` in commit 836cfb1.

Two non-blocking sockets on 127.0.0.1, both threads
busy-spinning. No per-iteration `setsockopt`. No blocking
recv wake-up. 128-byte payload (matches `FillRecord`). Measures
true kernel UDP round-trip.

## Published numbers

| Environment | RTT P50 |
|---|---|
| Linux loopback, this host, non-blocking + spin (measured) | ~9.9 µs |
| Linux loopback, blocking recv | ~5–10 µs |
| Same-rack 10 GbE, non-blocking | ~5–15 µs |
| Cross-DC WAN | 500 µs – 50 ms |

The first row is our measured `compare_all::raw_udp_128b`
(2026-07-01), dominated by two `sendto` + two `recvfrom` at
~4 µs each. Some published loopback micro-benchmarks quote
~2 µs; that figure did not reproduce on this host, and
`facts/cast-vs-udp-overhead.md` documents the walk-back.

Sources: RFC 768 (UDP), `udp(7)` Linux man page,
`facts/syscall-latency.md` (local measurement dfe2ef4),
`facts/cast-vs-udp-overhead.md` (the ~2 µs walk-back).

## Why not raw UDP for exchange IPC

- No ordering guarantee across reorder buffers.
- No gap detection — a dropped fill is silently lost.
- No retransmit — consumer must implement all reliability.

Every exchange transport that uses UDP (LBM, Aeron, casting)
adds reliability on top. The question is how: NAK-based
(Aeron, casting, LBM), ACK-based (KCP), or FEC (Solana Turbine).
