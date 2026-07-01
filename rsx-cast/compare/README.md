# rsx-cast: Protocol Comparisons

**Scope caveat, read first.** Every number here is one workload:
loopback RTT of a fixed 128-byte record on a single 6-core box,
closed-loop, synthetic. It is not a verdict on the competitors'
designed-for workloads (Aeron multicast fan-out, QUIC over the
WAN, Chronicle mmap IPC). It answers one question — *what does
each protocol cost to move one fixed record across localhost* —
and nothing more. Run the benches yourself; see "Running the
benches".

Five protocols are compared head-to-head on **speed** and
**features** (Aeron, MoldUDP64, Quinn/QUIC, Chronicle Queue,
LBM). Supporting protocols (TCP, raw UDP, KCP, SoupBinTCP) have
their own benches and docs — see the supporting section at the
bottom.

## What rsx-cast is

- NAK-based reliable UDP unicast (the **casting** half),
  `#[repr(C)]` fixed-size frames, two-tier retransmit
  (in-memory ring → on-disk WAL).
- WAL is the wire format is the audit log. One bytestream, three
  uses (live, replay, archive). No transformation between them.
- TCP cold-path replay (the **replication** half) for catch-up;
  same record layout.

## The five head-to-head competitors

### Speed (loopback p50, this 6-core Ryzen, payload **128 B**)

Every bench below uses the same 128-byte payload (matches
`size_of::<FillRecord>()`) and pins client+server to cores 2/3.
Numbers are directly comparable; the bench that produces each
row is in the right-most column. The "when" column says whether
the number was re-run 2026-07-01 or carried over from the
2026-05-24 measurement pass (`facts/cast-vs-udp-overhead.md`);
carried-over rows have not been re-run since.

| Protocol | Loopback p50 | Bench | When | Published / off-box |
|---|---:|---|---|---|
| **casting (rsx-cast)** | **~9.3 µs** | `cast_rtt_bench` | 2026-07-01 | — |
| **raw UDP** (baseline) | ~9.9 µs | `compare_all::raw_udp_128b` | 2026-07-01 | floor: `sendto + recvfrom`, no framing |
| **MoldUDP64** | ~10 µs | `compare_moldudp64` | 2026-05-24 | matches casting shape, NAK + separate request server |
| **TCP_NODELAY** | ~14 µs | `compare_all::tcp_nodelay_128b` | 2026-05-24 | persistent connection, read_exact |
| **SoupBinTCP** | ~14 µs | `compare_soupbintcp` | 2026-05-24 | TCP + 3-byte framing |
| **Aeron** (UDP) | ~305 µs | `compare_aeron` | 2026-05-24 | 21 µs on AWS c6in.16xlarge (pinned) |
| **Aeron** (IPC) | ~830 ns | `compare_aeron` | 2026-05-24 | sub-µs shared-memory IPC |
| **Quinn / QUIC** | ~37 µs | `compare_all::quinn_persistent_128b` | 2026-05-24 | 25–400 µs (published QUIC loopback) |
| **KCP** (turbo) | ~21 µs | `compare_all::kcp_spin_flush_128b` | 2026-05-24 | turbo mode + spin-flush |
| **Chronicle Queue** (Java) | — (doc only) | — | — | sub-µs IPC published, mmap-shared |
| **LBM** (commercial) | — (closed-source) | — | — | ~1–5 µs LAN, vendor whitepapers |

Re-run 2026-07-01: `cast_rtt_bench` (`cmp_rtt_fill_echo
[8.36 µs 9.29 µs 10.47 µs]`, criterion low/median/high) and
`compare_all::raw_udp_128b` (`[8.90 µs 9.91 µs 11.01 µs]`) both
reproduce their documented ~9–10 µs. The rest of `compare_all`
(KCP → Quinn → TCP) currently aborts on a KCP warmup panic
(`flush()` before `update()`; see `bugs.md`
BENCH-KCP-FLUSH-NEEDUPDATE), so the `kcp`/`quinn`/`tcp` rows are
the last-measured 2026-05-24 figures, not re-run today. The
harness is otherwise unchanged.

For this workload, casting's RTT sits at the raw-UDP floor: the
per-send breakdown attributes ~26 ns of userspace work (CRC32C +
16-byte header + ring-cache copy) on top of the ~4 µs `sendto`
syscall, so the protocol adds essentially nothing over
`sendto + recvfrom` (`facts/cast-vs-udp-overhead.md`). It ties
MoldUDP64's UDP-sequenced frame and comes in under the TCP
protocols (TCP_NODELAY, SoupBinTCP), the userspace-RUDP options
(KCP, QUIC), and Aeron's networked UDP path.

The only numbers below casting's are **shared-memory IPC**
(Aeron IPC ~830 ns, Chronicle sub-µs). That is not casting losing
a network race — those paths skip the kernel network stack
entirely (writer and reader share physical pages on one host),
so they are not comparable network transports, and neither
carries casting's WAL = wire = audit-log property. On the
same-host-IPC axis casting does not compete; on the
reliable-UDP-over-network axis it is at the floor. Both
statements are about this one loopback workload, not the
competitors' designed-for workloads (Aeron multicast fan-out,
QUIC over the WAN, Chronicle mmap IPC).

How to run them all locally:

```
cargo bench -p rsx-cast --bench compare_all
cargo bench -p rsx-cast --bench compare_aeron
cargo bench -p rsx-cast --bench compare_moldudp64
cargo bench -p rsx-cast --bench compare_soupbintcp
```

`compare_all` runs raw_udp + KCP + Quinn + TCP under one
`EchoClient` trait — same harness, same payload, same pinning,
in one process. The three standalone benches (aeron, mold, soup)
stay separate because their server setups can't fit the
in-process `EchoClient` trait (Aeron needs a media driver and
callback-driven receive; MoldUDP64/SoupBinTCP need framing
servers that don't pretend to be an echo socket). They use the
same payload size and core pinning so the numbers are still
directly comparable to compare_all.

Numbers below 30 µs from local benches are dominated by the
`sendto` syscall (~4 µs) and scheduler noise.
See [`facts/cast-vs-udp-overhead.md`](https://github.com/kronael/rsx/blob/master/facts/cast-vs-udp-overhead.md)
for the attribution breakdown.

### Features

| Property | casting/replication | Aeron | MoldUDP64 | Quinn (QUIC) | Chronicle Queue | LBM |
|---|---|---|---|---|---|---|
| Loss detection | NAK (receiver) | NAK (receiver) | NAK (receiver) | ACK (packet-num ranges) | n/a (durable log) | NAK (receiver) |
| Retransmit source | hot ring + WAL | term buffers (RAM) | separate request server | in-memory window | disk | in-memory window |
| Retransmit horizon | WAL retention (4 h default) | ~192 MB RAM | request server policy | in-flight window | disk retention | RAM-bounded |
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
  implement and bench. Closest published peer to casting's wire shape.
- **Quinn / QUIC** — the modern "what about QUIC?" answer. ACK-based
  with congestion control, mandatory TLS, multiplexed streams. Real
  benefits (NAT traversal, mobile mobility) we don't need on an
  exchange LAN; real costs (handshake + CC state machine) we don't want.
- **Chronicle Queue** — persistent-log-as-transport peer on the **other
  axis**. Where casting is UDP-over-network, Chronicle is mmap-over-shared-pages.
  Sub-µs IPC. Java-only, single-host (open-source); cross-host needs
  Chronicle Enterprise (commercial).
- **LBM (Informatica UM)** — the commercial gold standard. Same NAK+UDP
  family. Documented for context; cannot legitimately benchmark (see
  [`facts/closed-source-messaging.md`](https://github.com/kronael/rsx/blob/master/facts/closed-source-messaging.md)
  on the DeWitt clause).

## Running the benches

```bash
cargo bench -p rsx-cast --bench 'compare_*'
```

> Known issue (2026-07-01): `compare_all` aborts on a KCP warmup
> panic (`flush()` before `update()`) — `raw_udp_128b` reports,
> then the run dies before KCP/Quinn/TCP. Tracked in `bugs.md`
> as BENCH-KCP-FLUSH-NEEDUPDATE. `cast_rtt_bench`, `compare_aeron`,
> `compare_moldudp64`, and `compare_soupbintcp` are unaffected.

For loss-behavior testing (root required, exposes TCP head-of-line
blocking and casting NAK recovery under realistic loss):

```bash
sudo tc qdisc add dev lo root netem loss 0.1%
cargo bench -p rsx-cast --bench 'compare_*'
sudo tc qdisc del dev lo root
```

## Supporting cast

These are benched for completeness; they're not the framing comparison:

| Protocol | Doc | Bench | Why it's not in the main five |
|---|---|---|---|
| raw UDP | [raw-udp.md](raw-udp.md) | `compare_all::raw_udp_128b` | Baseline floor, not a competitor |
| TCP | [tcp.md](tcp.md) | `compare_all::tcp_nodelay_128b` | rsx-cast uses TCP for cold-path replay, not live |
| KCP | [kcp.md](kcp.md) | `compare_all::kcp_spin_flush_128b` | Gaming RUDP; Quinn is the same family more credibly |
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
