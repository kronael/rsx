# rsx-cast: Protocol Comparisons

Five serious competitors, judged on **speed** and **features**.
Supporting-cast protocols (TCP, raw UDP, KCP, SoupBinTCP,
gossip / FEC / log) have their own benches and docs for
completeness — see the supporting-cast section at the bottom.

## What rsx-cast is

- NAK-based reliable UDP unicast (CMP), `#[repr(C)]` fixed-size
  frames, two-tier retransmit (in-memory ring → on-disk WAL).
- WAL is the wire format is the audit log. One bytestream, three
  uses (live, replay, archive). No transformation between them.
- TCP cold-path replay for catch-up; same record layout.

## The five serious competitors

### Speed (loopback p50, this 6-core Ryzen, payload 64–128 B)

| Protocol | Measured here | Published / pinned |
|---|---:|---|
| **CMP (rsx-cast)** | **~10 µs** | — |
| **MoldUDP64** | ~10 µs | matches CMP shape, NAK + separate request server |
| **Aeron** (UDP) | ~305 µs | 21 µs (AWS c6in.16xlarge, pinned cores) |
| **Aeron** (IPC) | 830 ns | sub-µs IPC, JVM warmup excluded |
| **Quinn / QUIC** | ~37 µs | 25–400 µs (Cloudflare / Google QUIC) |
| **Chronicle Queue** (Java) | — (doc only) | sub-µs IPC published, mmap-shared |
| **LBM** (commercial) | — (closed-source) | ~1–5 µs LAN, vendor whitepapers |

Numbers below 30 µs from local benches are dominated by the
`sendto` syscall (~4 µs) and unpinned-thread scheduler noise.
See [`facts/cmp-vs-udp-overhead.md`](../../facts/cmp-vs-udp-overhead.md)
for the attribution breakdown.

### Features

| Property | CMP/DXS | Aeron | MoldUDP64 | Quinn (QUIC) | Chronicle Queue | LBM |
|---|---|---|---|---|---|---|
| Loss detection | NAK (receiver) | NAK (receiver) | NAK (receiver) | ACK (packet-num ranges) | n/a (durable log) | NAK (receiver) |
| Retransmit source | hot ring + WAL | term buffers (RAM) | separate request server | in-memory window | disk | in-memory window |
| Retransmit horizon | WAL retention (48 h default) | ~192 MB RAM | request server policy | in-flight window | disk retention | RAM-bounded |
| Durability | **wire = disk format** | Aeron Archive (separate) | external | none | mmap files | external |
| Multi-receiver | unicast only | unicast + multicast + IPC | multicast | unicast | multi-reader via mmap | multicast + unicast |
| Connection model | connection-less | connection-less | connection-less | TLS 1.3 handshake | mmap session | session |
| FEC | — | optional | — | — | — | — |
| Wire format | 16-byte header, repr(C) | 32-byte header, term offsets | 20-byte header | variable QUIC frames | self-describing | proprietary |
| Language | Rust | Java + C++ | any (public spec) | Rust | Java/Kotlin | Java + C |
| License | open (project) | Apache 2.0 | public spec, free to implement | Apache 2.0 | Apache 2.0 | commercial (no public bench) |

### One-paragraph framing

- **Aeron** — direct design ancestor. Same NAK+UDP-unicast philosophy,
  decade-plus of HFT production. Separates archive (Aeron Archive) from
  the transport; rsx-cast fuses them.
- **MoldUDP64** — Nasdaq's UDP wire protocol for ITCH market data,
  production-deployed at exchange scale. Public spec — anyone can
  implement and bench. Closest published peer to CMP's wire shape.
- **Quinn / QUIC** — the modern "what about QUIC?" answer. ACK-based
  with congestion control, mandatory TLS, multiplexed streams. Real
  benefits (NAT traversal, mobile mobility) we don't need on an
  exchange LAN; real costs (handshake + CC state machine) we don't want.
- **Chronicle Queue** — persistent-log-as-transport peer on the **other
  axis**. Where CMP is UDP-over-network, Chronicle is mmap-over-shared-pages.
  Sub-µs IPC. Java-only, single-host (open-source); cross-host needs
  Chronicle Enterprise (commercial).
- **LBM (Informatica UM)** — the commercial gold standard. Same NAK+UDP
  family. Documented for context; cannot legitimately benchmark (see
  [`facts/closed-source-messaging.md`](../../facts/closed-source-messaging.md)
  on the DeWitt clause).

## Running the benches

```bash
cargo bench -p rsx-cast --bench 'compare_*'
```

For loss-behavior testing (root required, exposes TCP head-of-line
blocking and CMP NAK recovery under realistic loss):

```bash
sudo tc qdisc add dev lo root netem loss 0.1%
cargo bench -p rsx-cast --bench 'compare_*'
sudo tc qdisc del dev lo root
```

## Supporting cast

These are benched for completeness; they're not the framing comparison:

| Protocol | Doc | Bench | Why it's not in the main five |
|---|---|---|---|
| raw UDP | [raw-udp.md](raw-udp.md) | `compare_udp` | Baseline floor, not a competitor |
| TCP | [tcp.md](tcp.md) | `compare_tcp` | rsx-cast uses TCP for cold-path replay, not live |
| KCP | [kcp.md](kcp.md) | `compare_kcp` | Gaming RUDP; Quinn is the same family more credibly |
| SoupBinTCP | [soupbintcp.md](soupbintcp.md) | `compare_soupbintcp` | TCP + 3-byte framing; cost is within TCP noise |

Payload formats (ITCH 5.0, OUCH 5.0, SBE, FAST) belong to the
`rsx-messages` comparison axis, not the transport axis. They are
not in scope here. MoldUDP64 carries ITCH in production;
SoupBinTCP carries OUCH — see those docs for context.

## Long-tail census

See [`niche.md`](niche.md) for ~70 further projects across NAK/ACK
reliable UDP, persistent-log-as-transport, kernel-bypass, multicast,
FEC (Solana Turbine), gossip (SWIM/HyParView), and per-language libs.
Includes a 10-candidate promotion shortlist (NORM, TigerBeetle,
Apache Iggy, netcode.io, iceoryx2).
