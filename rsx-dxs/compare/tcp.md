# TCP

Baseline for stream-oriented transports. TCP provides reliable,
ordered, connection-oriented delivery over IP. No framing, no
message boundaries (stream protocol) — the application must add
length-prefix framing.

## Protocol

OS kernel TCP stack. Three-way handshake to establish. Reliable
delivery via cumulative ACK + selective retransmit (SACK). Congestion
control (CUBIC by default on Linux). Nagle algorithm coalesces small
writes (disabled by `TCP_NODELAY`).

## Relation to rsx-dxs

rsx-dxs uses TCP for the WAL cold path (DXS replay service), not for
the live order path. The design decision:

- **Hot path (CMP/UDP)**: connectionless, per-message NAK retransmit,
  no congestion control, no three-way handshake. Optimal for trusted
  LAN with near-zero loss.
- **Cold path (DXS/TCP)**: TCP is appropriate here. Replay is a bulk
  sequential read from WAL; throughput matters more than per-message
  latency; the TCP handshake overhead is amortised over the session.

Using TCP for live order flow would add:
- 3-way handshake (~1 RTT per connection) — zero in CMP
- Congestion control — irrelevant on a 10 GbE fabric
- Head-of-line blocking across all orders on one stream
- Nagle coalescing unless `TCP_NODELAY` is set

iggy project measured TCP avg ~990 µs vs CMP ~10 µs on loopback — a
~100× penalty for the live order path.

## Benchmark

`../benches/compare_quinn.rs` — includes `tcp_rtt_loopback_64b`.

Tokio async TCP with `set_nodelay(true)`, same 64-byte payload,
single persistent connection. This is the best-case TCP latency:
no handshake in the loop, no Nagle delay, no encryption.

| Transport | Loopback p50 (expected) |
|---|---|
| raw UDP | ~2 µs |
| rsx-dxs CMP | ~10 µs |
| TCP nodelay | ~100–1 000 µs |
| Quinn QUIC | ~200–2 000 µs |

Sources: iggy/#606 (github.com/apache/iggy/issues/606),
picoquic loopback paper (privateoctopus.com/2024/10/13/...)
