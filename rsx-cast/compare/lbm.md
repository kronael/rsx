# UltraMessaging / 29West LBM

Latency Busters Messaging (LBM), created by Todd Montgomery at
29West, acquired by Informatica (now Informatica Ultra Messaging,
UM). Commercial, closed-source — no public source, requires a
paid license to run. The industry-standard reliable UDP multicast
middleware for financial market data, deployed at exchange scale
(CME, NASDAQ, several IDB venues) for 20+ years.

**No bench possible.** This page is design comparison only.
Closed source + commercial license precludes a like-for-like
Criterion harness. Included because LBM is the direct design
ancestor of the entire NAK-UDP-multicast family; CMP and Aeron
both descend from the lineage. Honest acknowledgment that the
approach has been production-validated at scale.

Reference: https://ultramessaging.github.io/

## Protocol

LBM's core transports are **LBT-RM** (Reliable Multicast) and
**LBT-RU** (Reliable Unicast). Both share the same NAK-based
reliability model.

### Wire format

Proprietary. Publicly disclosed via Informatica's protocol
guides and Montgomery's talks but no open RFC. What is
publicly known:

- UDP-based (multicast and unicast modes).
- Per-datagram sequence numbers.
- NAK-style loss recovery from receiver to source.
- In-memory transmission window at source.

The exact framing layout is not published byte-for-byte.

### LBT-RM (multicast)

- Source sends to a multicast group; all subscribed receivers receive.
- Each datagram carries a sequence number.
- Receiver detects gaps and sends **NAK** to source.
- NAK suppression: if a receiver sees the retransmit already
  arriving (from another receiver's NAK), it suppresses its
  own NAK to avoid NAK storms in large fan-out groups.
- Source retransmits from in-memory transmission window.
- No central broker — source IS the server for its own stream.

This is the same NAK model as CMP and Aeron, adapted for
multicast fan-out.

### LBT-RU (unicast)

Same NAK-based model but point-to-point. Closer to CMP.

### Durability

Not native. Informatica sells a separate "Persistence" layer
that records the stream to disk; the core transport's
retransmit window is RAM only, sized in seconds of buffered
data, comparable to Aeron's term buffers.

## Lineage

```
LBM/29West (Todd Montgomery)
  → Aeron (Todd Montgomery + Martin Thompson, Real Logic)
  → CMP (rsx-cast, NAK model + WAL cold tier)
```

Todd Montgomery left 29West/Informatica and co-founded Real
Logic, which built Aeron as the open-source rethink of the
same model. CMP takes the NAK-from-receiver primitive and
adds a WAL cold tier — closing the gap between in-RAM
retransmit and durable replay that Informatica sells as a
separate paid product.

## Guarantees

| Dimension | LBM (LBT-RM / LBT-RU) | rsx-cast CMP |
|---|---|---|
| Delivery | Reliable (NAK + retransmit) | Reliable (NAK + WAL retransmit) |
| Loss detection | Receiver (NAK to source) | Receiver (seq gap → NAK) |
| Retransmit source | In-memory window (seconds) | Hot ring + cold WAL (48 h) |
| Multicast | Yes (primary use case) | No (unicast only, v2 multicast planned) |
| NAK suppression | Yes (multicast fan-out) | N/A (unicast) |
| Topic/subject routing | Yes | No (point-to-point per stream) |
| Broker | None | None |
| Durability | Separate paid Persistence layer | Built-in WAL |
| Wire format | Proprietary | Public: 16-byte `WalHeader` + payload |
| License | Commercial (Informatica) | Apache-2.0 / MIT (rsx-cast) |
| Language | C, Java bindings | Rust |

Many "unknown / proprietary" cells are honest — Informatica
publishes design overviews, not protocol details.

## Key difference: WAL as audit/ML log

LBM has no equivalent to rsx-cast's WAL serving as audit
trail and training data. LBM's in-memory window is sized for
retransmit only (seconds of data). The Informatica Persistence
add-on writes to disk but is a separate product and a
separate format from the live wire stream.

rsx-cast's WAL retains 48 h of every exchange event in the
same binary format as the wire — immediately usable for
backtesting, regulatory replay, and ML training, without
transformation.

## Published numbers

Informatica's own white papers publish microsecond-range
one-way latencies for LBT-RM and LBT-RU on commodity 10 GbE
LAN, generally in the 1–5 µs band for kernel-bypass
configurations (Solarflare OpenOnload, Mellanox VMA). These
are best-case wire-bypass numbers; commodity kernel-stack
deployments run higher (~10–50 µs).

For comparison: Aeron (the open-source descendant) measures
~21 µs P50 on AWS ENA c6in.16xlarge at 100k msg/s. The gap
between LBM's published 1–5 µs and Aeron's 21 µs is largely
NIC and kernel-bypass driver choice, not protocol.

Sources:
- https://ultramessaging.github.io/
- https://www.informatica.com/products/data-integration/real-time-integration/ultra-messaging.html
- https://29west.wordpress.com/tag/latency-busters-messaging-lbm/
- Aeron AWS 2025 benchmark
  (https://aws.amazon.com/blogs/industries/aeron-on-aws-2025-performance-benchmark-results/)
