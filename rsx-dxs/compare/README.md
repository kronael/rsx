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
| [tcp](tcp.md) | TCP | ACK (cumulative) | in-flight window | ~12–18 µs loopback (std spin), ~100–1 000 µs (tokio) | any |
| [kcp](kcp.md) | UDP | ACK (sender) | in-memory snd_buf | ~17 µs spin / ~11 ms timer (`compare_kcp`, 128 B loopback); ~25–300 ms (WAN) | C / Rust |
| [quinn](quinn.md) | QUIC/UDP | ACK (packet-number ranges) | in-memory | ~37 µs persistent loopback (`compare_quinn`, 128 B); 25–400 µs published | Rust |
| [aeron](aeron.md) | UDP uni+multi+IPC | NAK (receiver) | in-memory term buffers | ~21 µs P50 (AWS c6in.16xlarge), ~305 µs (our 6-core box) | Java/C++ |
| [chronicle-queue](chronicle-queue.md) | mmapped files / TCP | n/a (durable log) | disk | sub-µs IPC | Java |
| [lbm](lbm.md) | UDP multicast | NAK (receiver) | in-memory window | ~1–5 µs LAN | C (commercial) |

## Benchmark approach

All benchmarks: loopback on the same host. Comparison benches
(`compare_kcp`, `compare_quinn`) use a 128-byte payload to match
CMP's `FillRecord` (`mem::size_of::<FillRecord>() == 128`); older
benches that pre-date this alignment may still use 64 bytes.
Criterion with sample_size=50 on the compare benches.

```bash
# Run all comparisons
cargo bench -p rsx-dxs --bench compare_kcp
cargo bench -p rsx-dxs --bench compare_quinn
cargo bench -p rsx-dxs --bench compare_tcp
cargo bench -p rsx-dxs --bench compare_aeron

# Raw UDP baseline (already exists)
cargo bench -p rsx-dxs --bench compare_udp
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
- **Aeron**: the direct design ancestor of CMP. Loopback bench via the
  rusteron C media driver (no JVM needed). Apples-to-apples is hard
  because Aeron's UDP path includes a driver-IPC hop CMP doesn't have;
  the doc spells out the methodology and published reference numbers.
- **Chronicle Queue / LBM**: design comparisons only (Java / commercial).

## Long-tail census

See [niche.md](niche.md) for a deeper census of NAK/ACK reliable UDP,
persistent-log-as-transport, kernel-bypass, multicast, and gossip
projects that don't (yet) have their own deep doc.
