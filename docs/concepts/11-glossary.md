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
  mid, up to 1000:1 far); collapses a 100M-slot address space to ~617k
  price-level slots. → [slab-and-compression](05-slab-and-compression.md), [spec 21-orderbook](../../specs/2/21-orderbook.md)
- **BBO** — best bid & offer (top of book): `bid_px / bid_qty / ask_px /
  ask_qty`. → [spec 16-marketdata](../../specs/2/16-marketdata.md)
- **IOC / FOK / GTC** — time-in-force: immediate-or-cancel /
  fill-or-kill / good-till-cancel. → [spec 21-orderbook](../../specs/2/21-orderbook.md)

## Risk & pricing

- **fixed-point** — every price/qty is an `i64` in smallest units; convert
  only at the API boundary, never a float. → [fixed-point](06-fixed-point.md)
- **mark price** — the "fair" price RSX trusts for margin and liquidation, so
  one bad print can't wipe you out: the median of external CEX feeds, computed
  by the Mark process. → [spec 15-mark](../../specs/2/15-mark.md)
- **index price** — RSX's own fair-price estimate from its order book, used as
  the mark's fallback: a size-weighted mid of the ME BBO, computed by risk.
  → [spec 28-risk](../../specs/2/28-risk.md)
- **frozen margin** (initial margin, IM) — the collateral locked when you place
  an order, so you can't take a position you can't back: reserved at order
  entry, released on ORDER_DONE (durable record on ME `OrderAccepted`).
  → [spec 28-risk](../../specs/2/28-risk.md)

## Trading & positions

- **position** — how much of a symbol you hold: positive if you're long
  (bought, betting the price up), negative if short (sold, betting it down);
  your PnL and margin are computed from it. → [spec 28-risk](../../specs/2/28-risk.md)
- **equity** — your collateral plus unrealized PnL — the live number checked
  against maintenance margin every time the book moves. → [spec 28-risk](../../specs/2/28-risk.md)
- **unrealized PnL** (uPnL) — the paper profit or loss on an open position,
  marked to the current mark price; it becomes real only when you close.
  → [spec 28-risk](../../specs/2/28-risk.md)
- **maintenance margin** (MM) — the minimum equity to keep a position open;
  drop below it (`equity < maint_margin`) and you're liquidated. A lower bar
  than the initial margin that opened it. → [spec 28-risk](../../specs/2/28-risk.md)
- **liquidation** — when equity falls under maintenance margin, the risk engine
  force-closes your position at the mark to stop the loss growing; liquidation
  orders bypass the normal margin check. → [spec 13-liquidator](../../specs/2/13-liquidator.md), [spec 28-risk](../../specs/2/28-risk.md)
- **open interest** (OI) — the total size of all open positions on a symbol:
  how much is actually at stake in that market. → [spec 28-risk](../../specs/2/28-risk.md)
- **funding** — a small payment swapped between longs and shorts every 8h that
  nudges a perpetual's price back toward the index. → [spec 28-risk](../../specs/2/28-risk.md)
- **tick size** — the smallest price step a symbol allows; every valid price is
  a whole number of ticks (fixed per symbol in v1). → [spec 21-orderbook](../../specs/2/21-orderbook.md)
- **lot size** — the smallest quantity step a symbol allows; every valid size is
  a whole number of lots. → [spec 21-orderbook](../../specs/2/21-orderbook.md)

## Identifiers & ordering

- **cid** — client order id, a 20-char string; the client's idempotency
  key, tracked at the gateway. The ME's own dedup key is `(user_id,
  order_id)`, persisted in the WAL on accept. → [spec 18-messages](../../specs/2/18-messages.md)
- **oid** — exchange order id, a UUIDv7 (16 bytes). → [spec 18-messages](../../specs/2/18-messages.md)
- **seq** — per-stream monotonic sequence number on every WAL/cast record;
  gaps trigger FAULTED. → [spec 4-cast](../../specs/2/4-cast.md)

---

Deeper: [specs/2/1-architecture.md](../../specs/2/1-architecture.md),
[specs/2/21-orderbook.md](../../specs/2/21-orderbook.md),
[specs/2/45-tiles.md](../../specs/2/45-tiles.md)
