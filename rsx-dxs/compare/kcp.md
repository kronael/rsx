# KCP

Open-source reliable ARQ protocol over UDP by skywind3000 (Rui Kong).
~1000 lines of C (`ikcp.h` / `ikcp.c`). MIT-licensed. Widely used in
gaming, VPN tunnels, P2P. Not designed for sub-millisecond HFT.

Source: https://github.com/skywind3000/kcp

## Protocol

### Wire format — 24-byte header

```
Offset  Size  Field  Meaning
0       4     conv   Conversation ID (logical connection)
4       1     cmd    Command: PUSH=81, ACK=82, WASK=83, WINS=84
5       1     frg    Fragment index (0 = last/only; N = N more follow)
6       2     wnd    Sender's remaining receive window (in packets)
8       4     ts     Sender timestamp (ms, for RTT measurement)
12      4     sn     Sequence number
16      4     una    Cumulative ACK: all sn < una delivered
20      4     len    Payload length (0 for pure-ACK frames)
24+     var   data   Payload
```

All little-endian. MTU default 1400 B → MSS = 1376 B.
Compare: CMP/WAL header is 16 bytes with a fixed-size payload
(no fragmentation needed — messages are pre-sized ≤ MTU).

### Reliability model: ACK-based (not NAK)

KCP detects loss at the **sender** from absence of ACKs:

1. Every received segment triggers an explicit `IKCP_CMD_ACK` back
   to the sender, plus a piggybacked `una` (cumulative ACK) on
   every outgoing frame.
2. When the sender sees ACK(N+2), ACK(N+3) but no ACK(N), it
   increments `fastack` on segment N.
3. `fastack >= resend` → **fast retransmit** without waiting for RTO.
4. If no ACK arrives within RTO → timeout retransmit.

Contrast with NAK-based (CMP, Aeron): the **receiver** detects the
gap immediately and sends NAK(N) — the sender retransmits in ~1 RTT.
ACK-based fast retransmit requires ~1.5–2 RTTs minimum.

### Congestion control (optional)

TCP-style CWND/ssthresh, disabled with `nc=1` (turbo mode).
`nc=1` is correct for rsx-dxs's trusted-LAN use case — there is no
congestion to control on a 10 GbE datacenter fabric.

### RTO

RFC 6298 SRTT/RTTVAL, modified:
- Backoff: ×1.5 (vs TCP's ×2) — faster recovery.
- Min RTO: 30 ms in nodelay mode, 100 ms normal.
- Integer millisecond precision throughout.

### Fastest configuration ("turbo mode")

```c
ikcp_nodelay(kcp, 1, 10, 2, 1);
//               ^  ^   ^  ^
//               |  |   |  nc=1: no CWND
//               |  |   resend=2: fast retransmit after 2 out-of-order ACKs
//               |  interval=10ms: flush/update tick
//               nodelay=1: immediate ACK + minRTO=30ms
```

Even in turbo mode, the `interval` floor is **10 ms** — all data
written to the send queue is flushed within 10 ms. This is the
fundamental limitation: KCP is millisecond-granularity, not
microsecond-granularity.

## Relation to rsx-dxs

| Dimension | KCP turbo | rsx-dxs CMP |
|---|---|---|
| Loss detection | Sender (ACK absence) | Receiver (seq gap → NAK) |
| Retransmit latency | ~1.5–2 RTT | ~1 RTT |
| Min flush granularity | 10 ms | sub-µs (sendto on every append) |
| Framing overhead | 24 B header, auto-fragmentation | 16 B header, fixed-size msgs |
| Congestion control | Optional (nc=1 disables) | None |
| Connection state | IKCPCB: SRTT, cwnd, snd/rcv_buf, acklist | seq + NAK list |
| WAL / retransmit horizon | In-memory snd_buf only | hot ring + cold WAL (48 h) |
| Designed latency target | ~25–300 ms (WAN, gaming) | ~10 µs (LAN, exchange) |
| HFT production use | None documented | Target use case |

**KCP is designed for a different problem.** It optimises loss recovery
on the public internet (gaming, VPN) where RTTs are 20–300 ms and
link quality is unknown. rsx-dxs CMP optimises for a trusted 10 GbE
LAN where RTTs are ~2 µs and the dominant cost is the `sendto` syscall.

Sending a KCP `flush()` call to match CMP's per-message latency would
require calling `ikcp_update()` at microsecond intervals — the polling
model is fundamentally incompatible with sub-millisecond targets.

## Benchmark

`../benches/compare_kcp.rs` — Criterion, loopback, 64 B payload.

Measures: KCP turbo mode (`nodelay=1, interval=10, resend=2, nc=1`)
round-trip on 127.0.0.1 vs raw UDP vs rsx-dxs CMP.

**Expected result**: KCP latency ≫ CMP on zero-loss loopback because
the 10 ms flush interval dominates. Under 0.1% injected loss (`tc
netem`), KCP recovers faster than raw UDP (which drops silently).
CMP NAK fires in ~1 RTT and stays well below KCP's 10 ms floor.

## Published numbers

From KCP's own benchmark wiki (WAN, 10% simulated loss):
| Protocol | Worst-case sample |
|---|---|
| KCP turbo | 195–295 ms |
| libenet | 1412–1637 ms |

KCP is ~5–6× better than ENet under loss. On zero-loss LAN it has
no advantage over raw UDP and adds ~10 ms floor latency.

Aeron (AWS 2025, c6in.16xlarge, 100k msg/s):
- P50: 21–22 µs
- P99: 32–43 µs

KCP and Aeron do not compete in the same latency bracket.

## Rust ecosystem

| Crate | Notes |
|---|---|
| `kcp` v0.6.0 | Pure Rust port; optional Tokio; MIT |
| `tokio_kcp` | Async stream API on top of `kcp` |
| `kcp-tokio` | Alternative async, claims zero-copy |

Sources: https://github.com/skywind3000/kcp,
https://crates.io/crates/kcp,
https://aws.amazon.com/blogs/industries/aeron-on-aws-2025-performance-benchmark-results/
