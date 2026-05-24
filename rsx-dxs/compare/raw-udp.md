# Raw UDP

Baseline: no reliability, no framing, no CRC, no retransmit. The
absolute floor for any protocol built on UDP.

## Protocol

`std::net::UdpSocket::send_to` / `recv_from`. OS kernel routes the
datagram through the loopback network stack. Nothing else.

Loopback path: user → syscall → kernel socket buffer → loopback NIC
driver (virtual) → kernel socket buffer → syscall → user.

## Relation to rsx-dxs

CMP builds on raw UDP. The cost of CMP above this baseline is:
- 16-byte `WalHeader` framing + CRC32C verification
- `send_ring` slot write (WAL record caching for hot-tier retransmit)
- Periodic NAK / heartbeat / StatusMessage processing
- Sequence number assignment and `peer_consumption_seq` flow control

Measured overhead (loopback, 64 B payload):
```
raw UDP RTT      ~2.0 µs  (baseline, udp_rtt_bench)
CMP send body    ~3.87 µs  (one-way; cmp_send_breakdown_bench)
CMP RTT          ~10.3 µs  (round-trip; cmp_rtt_bench)
```

CMP adds ~1.9 µs per send over raw UDP on the hot path. The dominant
cost is `sendto` syscall (~3.85 µs measured in dfe2ef4), not CMP
protocol overhead.

## Benchmark

`../benches/udp_rtt_bench.rs` — pre-existing, ships with rsx-dxs.

Two non-blocking sockets on 127.0.0.1, both threads busy-spinning.
No per-iteration syscall overhead (no `set_read_timeout`). Measures
true kernel UDP round-trip.

## Published numbers

| Environment | RTT P50 |
|---|---|
| Linux loopback, same host, non-blocking + spin | ~2.0 µs |
| Linux loopback, blocking recv | ~5–10 µs |
| Same-rack 10 GbE, non-blocking | ~5–15 µs |
| Cross-DC WAN | 500 µs – 50 ms |

Sources: `facts/syscall-latency.md` (local measurement dfe2ef4),
Cloudflare kernel bypass post, Databento microstructure guide.

## Why not raw UDP for exchange IPC

- No ordering guarantee across reorder buffers.
- No gap detection — a dropped fill is silently lost.
- No retransmit — consumer must implement all reliability.

Every exchange using UDP (LBM, Aeron, CMP) adds reliability on top.
The question is how: NAK-based (Aeron, CMP), ACK-based (KCP),
or FEC (Solana Turbine).
