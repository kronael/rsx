# Quinn (QUIC)

The dominant Rust QUIC implementation. QUIC is a UDP-based transport
protocol (RFC 9000) with mandatory TLS 1.3, multiplexed streams,
connection migration, and congestion control. Quinn is used by Solana
TPU, iroh, and many other Rust network projects.

Crate: https://crates.io/crates/quinn (v0.11.x, MIT/Apache-2.0, ~180 M dl)

## Protocol

QUIC runs over UDP but adds:
- **TLS 1.3** on every connection (mandatory, cannot be disabled).
- **Connection handshake**: 1 RTT minimum (0-RTT for resumed sessions).
- **Multiplexed streams**: many independent bidirectional streams on one
  UDP flow. Head-of-line blocking eliminated between streams.
- **ACK-based loss recovery**: QUIC's ACK frames are more expressive
  than TCP SACK. Per-stream ordering, no cross-stream HOL blocking.
- **Congestion control**: CUBIC (default), BBR, or NewReno selectable.
- **Connection migration**: client IP change survives transparently.

## Relation to rsx-dxs

This is the answer to: *"why not QUIC for exchange IPC?"*

| Dimension | Quinn (QUIC) | rsx-dxs CMP |
|---|---|---|
| Auth/encryption | TLS 1.3 mandatory (~200 ns/msg AES-GCM) | None (trusted LAN) |
| Connection setup | 1 RTT handshake before first message | None (sendto, zero setup) |
| Congestion control | CUBIC/BBR/NewReno | None |
| Loss detection | ACK-based (sender-side) | NAK-based (receiver-side) |
| WAL cold tier | No | Yes (48 h retransmit horizon) |
| Loopback P50 | ~200–500 µs (est. from picoquic) | ~10 µs |
| Use case | Untrusted public internet | Trusted datacenter LAN |

QUIC's overheads are correct tradeoffs for the public internet (Solana
TPU, HTTP/3). On a trusted 10 GbE fabric behind a firewall:
- TLS is paying to solve a problem the network layer already solves.
- Congestion control is paying to solve a problem the fixed-capacity
  fabric doesn't have.
- The handshake is paying a 1 RTT tax on every new connection.

The iggy project measured Quinn at ~1.97 ms vs TCP at ~0.99 ms on
localhost for 40-byte messages — QUIC's protocol overhead is ~2× TCP,
which is ~20-100× CMP on loopback.

## Published loopback numbers

| Source | Metric | Value |
|---|---|---|
| picoquic (2024, Linux loopback) | RTT min | ~20 µs |
| picoquic (2024, Linux loopback) | RTT p50 range | 20–400 µs |
| picoquic (2024, Linux loopback) | RTT outliers (scheduler jitter) | up to 1 400 µs |
| iggy/#606 (40-byte, localhost) | avg RTT | ~1 970 µs |
| iggy/#606 TCP (40-byte, localhost) | avg RTT | ~990 µs |

picoquic uses the same QUIC wire format (RFC 9000); numbers are
representative of the protocol overhead, not implementation.

No side-by-side published benchmark of Quinn vs raw UDP vs KCP exists.

## Benchmark

`../benches/compare_quinn.rs` — Criterion, loopback, 64 B payload.

The TLS handshake is outside the timed loop (established once in
setup). Each benchmark iteration opens a new bidirectional stream,
sends 64 B, receives echo. Measures QUIC stream overhead, not
handshake overhead.

Self-signed cert generated at test time with `rcgen`.

```toml
[dev-dependencies]
quinn = "0.11"
rcgen = "0.13"
tokio = { version = "1", features = ["rt", "macros"] }
```

## Why Quinn over s2n-quic

Both are production-quality. Quinn chosen because:
- Older, more public loopback latency data available.
- MIT/Apache-2.0 dual license (vs Apache-2.0 only for s2n-quic).
- More prior art comparisons in the literature.

Sources: https://crates.io/crates/quinn,
https://www.privateoctopus.com/2024/10/13/RandomLoopbackDelaysSlowBbr.html,
https://github.com/apache/iggy/issues/606
