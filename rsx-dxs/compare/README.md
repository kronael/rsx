# rsx-dxs: Protocol Comparisons

State-of-the-art survey for reliable messaging over UDP. One file per
protocol; benchmark code lives in `../benches/compare_*.rs`.

## What rsx-dxs does

- **Hot path**: CMP/UDP — NAK-based reliable unicast, `#[repr(C)]`
  fixed-size frames, zero heap on send path, two-tier retransmit.
- **Cold path**: WAL TCP replay — same binary record layout on disk
  and wire, retransmit horizon = WAL retention (48 h).
- **Key claim**: the WAL is not just a retransmit buffer; it is the
  exchange audit log, the backtesting dataset, and the ML training
  data source — all at once, with zero transformation.

## Summary table

| Protocol | Transport | Loss detection | Retransmit source | Latency regime | Language |
|---|---|---|---|---|---|
| [raw-udp](raw-udp.md) | UDP | app-layer | — | ~2 µs loopback | any |
| [rsx-dxs CMP](../README.md) | UDP unicast | NAK (receiver) | hot ring + cold WAL | ~4 µs send body, ~10 µs RTT | Rust |
| [tcp](tcp.md) | TCP | ACK (cumulative) | in-flight window | ~100–1 000 µs loopback | any |
| [kcp](kcp.md) | UDP | ACK (sender) | in-memory snd_buf | ~25–300 ms (WAN) | C / Rust |
| [quinn](quinn.md) | QUIC/UDP | ACK (QUIC streams) | in-memory | ~200–2 000 µs loopback | Rust |
| [aeron](aeron.md) | UDP uni+multi+IPC | NAK (receiver) | in-memory term buffers | ~21 µs P50 | Java/C++ |
| [chronicle-queue](chronicle-queue.md) | mmapped files / TCP | n/a (durable log) | disk | sub-µs IPC | Java |
| [lbm](lbm.md) | UDP multicast | NAK (receiver) | in-memory window | ~1–5 µs LAN | C (commercial) |

## Benchmark approach

All benchmarks: loopback on the same host, payload = 64 bytes
(one cache line, matches CMP exchange frame size), Criterion with
100-iteration warmup.

```bash
# Run all comparisons
cargo bench -p rsx-dxs --bench compare_kcp
cargo bench -p rsx-dxs --bench compare_quinn

# Raw UDP baseline (already exists)
cargo bench -p rsx-dxs --bench udp_rtt_bench
```

Loss simulation (requires root):
```bash
sudo tc qdisc add dev lo root netem loss 0.1%
cargo bench ...
sudo tc qdisc del dev lo root
```

## Why these protocols

- **raw-udp**: absolute floor — anything above this is protocol overhead.
- **TCP**: stream baseline; used in rsx-dxs cold path (WAL replay). Answers
  "why not TCP for live orders?" with a number.
- **KCP**: most-cited "reliable UDP" alternative in open source; ACK-based;
  designed for WAN/gaming. Answers "why not KCP?"
- **Quinn (QUIC)**: answers "why not QUIC?" Mandatory TLS + connection
  handshake + congestion control — measurable on loopback.
- **Aeron**: the direct design ancestor of CMP. JVM media driver required;
  not directly benchmarkable here, but published numbers are included.
- **Chronicle Queue / LBM**: design comparisons only (Java / commercial).
