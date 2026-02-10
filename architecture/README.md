# RSX Component Architecture

Per-component architecture documentation. For the
system-level view see
[specs/v1/ARCHITECTURE.md](../specs/v1/ARCHITECTURE.md).

## Components

| Document | Component | Crate |
|----------|-----------|-------|
| [MATCHING.md](MATCHING.md) | Matching Engine | rsx-matching, rsx-book |
| [RISK.md](RISK.md) | Risk Engine | rsx-risk |
| [GATEWAY.md](GATEWAY.md) | Gateway | rsx-gateway |
| [MARKETDATA.md](MARKETDATA.md) | Market Data | rsx-marketdata |
| [DXS.md](DXS.md) | DXS / WAL / CMP | rsx-dxs |
| [MARK.md](MARK.md) | Mark Price | rsx-mark |

## System Topology

```
User -> Gateway -> Risk -> ME (per symbol)
                     ^       |
                     |       +-> Marketdata -> Users
                     |       +-> WAL -> Recorder
                     +-- Mark (external feeds)
                     +-- Postgres (write-behind)
```

## Inter-Process Communication

All hot-path communication uses CMP/UDP (C Message
Protocol). Cold-path uses WAL/TCP (DXS replay).
Intra-process uses SPSC rings (rtrb).

See [specs/v1/CMP.md](../specs/v1/CMP.md) and
[specs/v1/TILES.md](../specs/v1/TILES.md).
