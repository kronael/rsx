# OUCH

Nasdaq's order-entry **message format**. Counterpart to ITCH:
ITCH is what the exchange sends out (market data), OUCH is what
the customer sends in (orders). OUCH messages are framed over
[SoupBinTCP](soupbintcp.md) — specifically as `U` (unsequenced
data) packets in the client → server direction and `S` (sequenced
data) in the server → client direction.

Spec: https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/ouch5.0.pdf

Why we include it: OUCH/SoupBinTCP is the closest published peer
to CMP for order entry. The transport benchmark already lives in
[compare_soupbintcp](../benches/compare_soupbintcp.rs); this doc
is the message-level counterpart of [itch5.md](itch5.md).

## Message catalog (abbreviated, OUCH 5.0)

Inbound (client → server, framed as SoupBin `U`):

| Letter | Size (B) | Meaning |
|---|---|---|
| `O` | 47 | Enter Order |
| `U` | 25 | Replace Order |
| `X` | 21 | Cancel Order |
| `Y` | 17 | Cancel by Order ID |
| `M` | 9 | Modify Order |
| `Q` | 14 | Mass Cancel by Firm Identifier |

Outbound (server → client, framed as SoupBin `S`):

| Letter | Size (B) | Meaning |
|---|---|---|
| `S` | 10 | System Event |
| `A` | 65 | Order Accepted |
| `J` | 28 | Order Rejected |
| `U` | 36 | Order Replaced |
| `C` | 28 | Order Canceled |
| `E` | 40 | Order Executed |
| `B` | 24 | Broken Trade |
| `I` | 13 | Order Modified |

The `O` (Enter Order) message is the heart of the protocol:

```
0    1   message_type = 'O'
1    14  order_token        ASCII, client-assigned
15   1   buy_sell            'B' or 'S'
16   4   shares              u32 BE
20   8   stock               ASCII, padded
28   4   price               u32 BE (price = raw / 10000)
32   4   time_in_force       u32 BE (0=IOC, 99999=DAY)
36   4   firm                ASCII, padded
40   1   display
41   1   capacity
42   1   intermarket_sweep_eligibility
43   4   minimum_quantity    u32 BE
47       (end)
```

47 B per inbound order. Compare CMP/RSX's order ingress: 64 B
fixed `#[repr(C, align(64))]` record (one cache line), i64
fixed-point prices, full 16-byte UUIDv7 `oid`.

## Relation to CMP order flow

| Dimension | OUCH 5.0 / SoupBinTCP | RSX CMP order flow |
|---|---|---|
| Transport | TCP (SoupBinTCP `U` packet) | UDP unicast (CMP) |
| Message size | 47 B (Enter Order) | 64 B (cache-line) |
| Order ID width | 14-byte ASCII `order_token` | 16-byte UUIDv7 `oid` |
| Price width | u32 BE, scaled by 10000 | i64 fixed-point |
| Time precision | None (server timestamps) | ns since epoch |
| Auth | SoupBin Login (cleartext password) | Gateway JWT (out-of-protocol) |
| Reliability | TCP + SoupBin session resume | NAK + WAL replay (48 h) |
| Round-trip latency, colo | ~25–50 µs (Nasdaq published) | Target <50 µs GW→ME→GW |

OUCH's design choices (compact ASCII fields, u32 prices, TCP
transport) are products of a 1990s-era exchange architecture
that still works. CMP's design choices (cache-line records,
i64 fixed-point, UDP unicast + NAK) are products of a 2020s
target latency budget that's ~10× tighter — and a willingness
to assume a trusted LAN.

## Why no bench

The OUCH application layer rides on SoupBinTCP. The transport
RTT is in [compare_soupbintcp](../benches/compare_soupbintcp.rs).
Adding an OUCH-message-parse bench would measure `bswap32` +
struct decode — the same parsing primitives as ITCH. Not
informative for the rsx-dxs transport survey.

## Sources

- https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/ouch5.0.pdf (OUCH 5.0)
- https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/soupbintcp.pdf (SoupBinTCP, the transport)
- See [soupbintcp.md](soupbintcp.md) for transport-level comparison.
