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

| Dimension | Raw UDP | rsx-dxs CMP |
|---|---|---|
| Delivery | Best-effort (may drop) | Reliable (NAK + WAL retransmit) |
| Ordering | Unordered (may reorder) | Per-stream FIFO (seq monotonic) |
| Duplicates | May duplicate | Dedup in receiver via seq |
| Framing | None (one datagram = one recvfrom) | 16-byte `WalHeader` + fixed-size payload |
| Integrity | UDP checksum (optional, weak) | CRC32C over each record |
| Durability | None | WAL on disk, 48 h retransmit horizon |
| Flow control | None (sender can overrun receiver) | `peer_consumption_seq` window |
| Connection state | None | seq + NAK list + status heartbeat |

Everything in the CMP column is layered above the kernel UDP
socket. Each row is a cost measured against the raw-UDP floor.

## Relation to rsx-dxs

CMP builds on raw UDP. The cost of CMP above this baseline is:

- 16-byte `WalHeader` framing + CRC32C verification
- `send_ring` slot write (WAL record caching for hot-tier retransmit)
- Periodic NAK / heartbeat / StatusMessage processing
- Sequence number assignment and `peer_consumption_seq` flow control

Measured overhead (loopback, 64 B payload):

```
raw UDP RTT      ~2.0 µs  (baseline, compare_udp)
CMP send body    ~3.87 µs  (one-way; cmp_send_breakdown_bench)
CMP RTT          ~10.3 µs  (round-trip; cmp_rtt_bench)
```

CMP adds ~1.9 µs per send over raw UDP on the hot path. The
dominant cost is the `sendto` syscall (~3.85 µs measured in
dfe2ef4), not CMP protocol overhead.

## Benchmark

`../benches/compare_udp.rs` — pre-existing, ships with rsx-dxs.

Two non-blocking sockets on 127.0.0.1, both threads
busy-spinning. No per-iteration `setsockopt`. No blocking
recv wake-up. 64-byte payload (one cache line). Measures
true kernel UDP round-trip.

## Published numbers

| Environment | RTT P50 |
|---|---|
| Linux loopback, same host, non-blocking + spin | ~2.0 µs |
| Linux loopback, blocking recv | ~5–10 µs |
| Same-rack 10 GbE, non-blocking | ~5–15 µs |
| Cross-DC WAN | 500 µs – 50 ms |

Sources: RFC 768 (UDP), `udp(7)` Linux man page,
`facts/syscall-latency.md` (local measurement dfe2ef4),
Cloudflare kernel-bypass write-up.

## Why not raw UDP for exchange IPC

- No ordering guarantee across reorder buffers.
- No gap detection — a dropped fill is silently lost.
- No retransmit — consumer must implement all reliability.

Every exchange transport that uses UDP (LBM, Aeron, CMP)
adds reliability on top. The question is how: NAK-based
(Aeron, CMP, LBM), ACK-based (KCP), or FEC (Solana Turbine).
