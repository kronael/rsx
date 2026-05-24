# UltraMessaging / 29West LBM

Latency Busters Messaging (LBM), created by Todd Montgomery at 29West,
acquired by Informatica, now part of TIBCO. The industry-standard
reliable UDP multicast middleware for financial market data.
Closed source, commercial. Not benchmarkable here.

Reference: https://ultramessaging.github.io/

## Protocol

LBM's core transport is **LBT-RM** (Reliable Multicast) and
**LBT-RU** (Reliable Unicast).

### LBT-RM (multicast)
- Source sends to a multicast group; all subscribed receivers receive.
- Each datagram carries a sequence number.
- Receiver detects gaps and sends **NAK** to source.
- NAK suppression: if a receiver sees the retransmit already arriving
  (from another receiver's NAK), it suppresses its own NAK.
- Source retransmits from in-memory transmission window.
- No central broker — source IS the server for its own stream.

This is the same NAK model as CMP and Aeron, adapted for multicast
fan-out.

### LBT-RU (unicast)
Same NAK-based model but point-to-point. Closer to CMP.

## Relation to rsx-dxs

LBM is the commercial precedent for what rsx-dxs does open-source in
Rust. It is the most direct production lineage:

```
LBM/29West (Todd Montgomery) → Aeron (Todd Montgomery + Martin Thompson) → CMP (rsx-dxs, NAK model)
```

| Dimension | LBM | rsx-dxs CMP |
|---|---|---|
| NAK-based | Yes | Yes |
| Retransmit source | In-memory window | Hot ring + cold WAL (48 h) |
| Multicast | Yes (primary use case) | No (unicast only) |
| Topic/subject routing | Yes | No (point-to-point per stream) |
| Broker | None | None |
| License | Commercial (TIBCO) | Open source (rsx-dxs) |
| Language | C/Java | Rust |
| WAL cold tier | No | Yes |

LBM is multicast-centric: one source, many receivers, NAK suppression
for fan-out. rsx-dxs is unicast-centric: N independent streams, one
receiver per stream. The fan-out model is different but the reliability
primitive is the same.

## Key difference: WAL as audit/ML log

LBM has no equivalent to rsx-dxs's WAL serving as audit trail and
training data. LBM's in-memory window is sized for retransmit only
(seconds of data). rsx-dxs's WAL retains 48 h of every exchange
event in the same binary format as the wire — immediately usable
for backtesting, regulatory replay, and ML training.

## Direct benchmark

Not available — closed source commercial software.

Published in academic literature: LBM achieves ~1–5 µs one-way
latency on 10 GbE LAN. Aeron (open-source successor) achieves
~21 µs P50 on AWS ENA — hardware/NIC difference accounts for the gap.

Sources: https://ultramessaging.github.io/,
https://29west.wordpress.com/tag/latency-busters-messaging-lbm/,
Infopro Digital "Latency benchmarks for ultra-low-latency messaging"
