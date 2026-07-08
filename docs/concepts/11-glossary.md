# Glossary

RSX-specific meanings, one line each. For depth, follow the link — this
page stays short on purpose.

## Transport & storage

- **casting** — RSX's reliable-UDP *live* transport between processes: NAK
  gap-recovery + idle-only heartbeats, deliberately no flow control. The
  hot path. → [casting](01-casting.md), [spec 4-cast](../../specs/2/4-cast.md)
- **replication** — TCP replay of the *same* WAL records; the cold path
  (catch-up, recovery, warm-standby). → [spec 10-replication](../../specs/2/10-replication.md)
- **WAL** — write-ahead log. The on-disk bytes = the casting wire frame =
  the replication stream, no serialization step. → [casting](01-casting.md), [spec 48-wal](../../specs/2/48-wal.md)
- **FAULTED** — a cast receiver's state when it detects a sequence gap;
  triggers TCP replay to recover. → [spec 4-cast](../../specs/2/4-cast.md)
- **UDS** (Unix Domain Socket) — a Gateway→Risk transport *candidate* that
  was evaluated but **not** chosen (casting/UDP won). Still used off the
  hot path for log forwarding to Vector. → [rsx-risk/notes/uds.md](../../rsx-risk/notes/uds.md), [spec 33-telemetry](../../specs/2/33-telemetry.md)
- **SPSC ring** — single-producer/single-consumer lock-free queue (rtrb),
  intra-process only, ~50–170 ns/hop. → [spsc-rings](03-spsc-rings.md)

## Architecture

- **tile** — a pinned OS thread running a busy-spin hot loop (plus any
  SPSC rings to sibling threads); the unit of intra-process concurrency.
  → [tiles-and-pinning](02-tiles-and-pinning.md), [spec 45-tiles](../../specs/2/45-tiles.md)
- **Gateway / Risk / ME / Marketdata / Mark / Recorder** — the six
  processes. ME = matching engine. → [spec 1-architecture](../../specs/2/1-architecture.md)
- **vshard / shardmap** — risk shards users by `hash(user_id) % N_VSHARDS`
  (fixed) → a mutable `shardmap` table → node. Lets the cluster grow
  without a global reshuffle. → [sharding-axes](09-sharding-axes.md), [spec 28-risk §Sharding](../../specs/2/28-risk.md)

## Orderbook & matching

- **slab** — pre-allocated arena of fixed-size `OrderSlot`s; capacity is
  the `Orderbook::new` argument, often sized to tens of millions.
  → [slab-and-compression](05-slab-and-compression.md)
- **CompressionMap** — distance-zone price→index compression (1:1 near
  mid, up to 1000:1 far); a 20M-level book in ~15 MB. → [slab-and-compression](05-slab-and-compression.md), [spec 21-orderbook](../../specs/2/21-orderbook.md)
- **BBO** — best bid & offer (top of book): `bid_px / bid_qty / ask_px /
  ask_qty`. → [spec 16-marketdata](../../specs/2/16-marketdata.md)
- **IOC / FOK / GTC** — time-in-force: immediate-or-cancel /
  fill-or-kill / good-till-cancel. → [spec 21-orderbook](../../specs/2/21-orderbook.md)

## Risk & pricing

- **fixed-point** — every price/qty is an `i64` in smallest units; convert
  only at the API boundary, never a float. → [fixed-point](06-fixed-point.md)
- **mark price** — median of external CEX feeds, computed by the Mark
  process; used for margin. → [spec 15-mark](../../specs/2/15-mark.md)
- **index price** — size-weighted mid derived from the ME BBO by risk; the
  fallback when mark is unavailable. → [spec 28-risk](../../specs/2/28-risk.md)
- **frozen margin** — margin reserved at order entry, released on
  ORDER_DONE; the durable record is written on ME `OrderAccepted`. → [spec 28-risk](../../specs/2/28-risk.md)

## Identifiers & ordering

- **cid** — client order id, a 20-char string; the client's idempotency
  key (dedup is persisted in the WAL on accept). → [spec 18-messages](../../specs/2/18-messages.md)
- **oid** — exchange order id, a UUIDv7 (16 bytes). → [spec 18-messages](../../specs/2/18-messages.md)
- **seq** — per-stream monotonic sequence number on every WAL/cast record;
  gaps trigger FAULTED. → [spec 4-cast](../../specs/2/4-cast.md)

---

Deeper: [specs/2/1-architecture.md](../../specs/2/1-architecture.md),
[specs/2/21-orderbook.md](../../specs/2/21-orderbook.md),
[specs/2/45-tiles.md](../../specs/2/45-tiles.md)
