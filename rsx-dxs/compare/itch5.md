# ITCH 5.0

Nasdaq's market-data **message format** (the "what"), not a
transport. ITCH 5.0 messages are the payloads that ride inside
MoldUDP64 packets (UDP multicast) and SoupBinTCP `S` packets
(TCP). When people say "the ITCH feed", they mean ITCH-over-
MoldUDP64.

Spec: https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/NQTVITCHspecification.pdf

Why we include it: framing/transport docs already cover
[MoldUDP64](moldudp64.md) and [SoupBinTCP](soupbintcp.md). This
entry exists to make the relationship explicit and to size the
ITCH record itself so an apples-to-apples comparison against CMP
records is possible.

## Message catalog (abbreviated)

ITCH 5.0 messages are fixed-format, big-endian, type-tagged by a
single ASCII letter. The record sizes below come straight from
the spec.

| Letter | Size (B) | Meaning |
|---|---|---|
| `S` | 12 | System Event (start of session, etc.) |
| `R` | 39 | Stock Directory |
| `H` | 25 | Stock Trading Action |
| `Y` | 20 | Reg SHO Short Sale Price Test Restriction |
| `L` | 26 | Market Participant Position |
| `V` | 35 | MWCB Decline Level |
| `A` | 36 | Add Order — No MPID Attribution |
| `F` | 40 | Add Order — With MPID Attribution |
| `E` | 31 | Order Executed |
| `C` | 36 | Order Executed With Price |
| `X` | 23 | Order Cancel |
| `D` | 19 | Order Delete |
| `U` | 35 | Order Replace |
| `P` | 44 | Trade (non-cross) |
| `Q` | 40 | Cross Trade |
| `B` | 19 | Broken Trade |

Each message has a `Stock Locate` (u16) and `Tracking Number`
(u16) prefix immediately after the type byte, plus a 6-byte
`Timestamp` (nanoseconds since midnight, stored as 48-bit
big-endian integer — the spec's quirkiest field).

## Relation to CMP records

ITCH `A` (Add Order, 36 B) maps almost 1:1 to `rsx-messages`'
`OrderRecord`:

| ITCH `A` field | Size | CMP equivalent |
|---|---|---|
| `message_type ('A')` | 1 B | `WalHeader.record_type:u16` |
| `stock_locate` | 2 B | `symbol_id:u32` |
| `tracking_number` | 2 B | (unused; CMP uses CRC32 in header) |
| `timestamp` | 6 B | `ts_ns:u64` (8 B, full 64-bit nanoseconds) |
| `order_reference_number` | 8 B | `oid:[u8;16]` (UUIDv7, twice as wide) |
| `buy_sell` | 1 B | `Side` (enum, u8) |
| `shares` | 4 B | `Qty:i64` |
| `stock` | 8 B | (CMP uses `symbol_id`, not symbol string) |
| `price` | 4 B | `Price:i64` (4 B vs CMP's 8 B — ITCH price = 1/10000 USD u32) |

CMP records are wider (i64 fixed-point everywhere, UUIDv7 IDs,
ns timestamps) because they need to survive a 48 h WAL plus
serve as the audit log for an exchange. ITCH is dissemination-
only; its compact representation is appropriate for a feed.

## Why no bench

ITCH is a payload format, not a transport. The transport-level
RTT is covered by `compare_moldudp64.rs` and `compare_soupbintcp.rs`.
Benching "ITCH-parse" in isolation would measure `bswap16`/`bswap32`
+ struct decode on the wire bytes — interesting for a parser
shootout, not for the rsx-dxs transport survey.

## Sources

- https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/NQTVITCHspecification.pdf (ITCH 5.0)
- https://www.nasdaqtrader.com/Trader.aspx?id=Totalview2 (TotalView product page; ITCH is the wire format)
- See [moldudp64.md](moldudp64.md) and [soupbintcp.md](soupbintcp.md) for the transports.
