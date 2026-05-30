# Sharding Axes

RSX has two independent axes of scale-out. They do not
interfere with each other.

## The two axes

Risk shards by `user_id`. Each shard owns a contiguous range
of users and holds all of their positions and margin in RAM.
The routing key is `user_id mod num_shards` (or an explicit
range assignment). A shard processes every symbol for its users
— it receives fills from all matching engines but only applies
the ones belonging to its user range. Adding users means adding
Risk shards.

Matching shards by `symbol_id`. One matching engine per
tradeable instrument. Each engine is a pinned thread with its
own orderbook, its own WAL, its own TCP replay server. There is
no cross-symbol shared state. Adding symbols means adding ME
instances.

The gateway is stateless. It routes each incoming order to the
correct Risk shard by `user_id` and embeds the `symbol_id` in
the order payload so Risk can forward to the correct ME. Adding
gateway instances scales connection capacity without any
coordination.

## The path of an order

User U sends an order on symbol S:

```
Gateway → Risk[U] → ME[S]
                 ↓ (fill, async)
            Risk[U] ← ME[S]
Gateway ← (fill, direct from ME[S])
```

Gateway routes by `user_id`. Risk[U] validates margin and
freezes collateral, then routes by `symbol_id` to ME[S]. ME[S]
matches against the book and appends the fill to its WAL.
The fill notification goes directly from ME to the gateway
(3 hops to the client) while the settlement — updating the
position and releasing the frozen margin — goes from ME back
to Risk asynchronously (off the client's critical path).

The margin reservation is synchronous and worst-case (full
notional plus fees) so that the async settle gap never creates
an over-leveraged position. The trade-off is that margin is
eventually-consistent from the client's perspective: a very
fast client recycling freed margin within the async lag (
microseconds to one casting hop plus Risk queue) may see
a spurious rejection on a subsequent order. Solvency is
never at risk; only a fast client racing its own freed margin.

## Why the axes are orthogonal

A Risk shard does not care which symbols exist. It receives
fills from all ME instances and applies the ones for its users.
Adding a new symbol adds a new ME instance and a new WAL stream;
existing Risk shards subscribe to that stream and begin
processing its fills automatically.

A matching engine does not care how many users exist. It
processes orders keyed by `user_id` and `order_id`, both
of which are opaque integers to it. Adding a new Risk shard
does not change any ME.

Gateway is already stateless with respect to both axes. A new
gateway instance needs only the list of Risk shard addresses
(keyed by user range) and ME addresses (keyed by symbol). Both
are configuration.

This is the constraint that makes the scale-out clean. Any
design that puts per-user state in the ME, or per-symbol state
in Risk, would couple the axes and require coordination whenever
either grows.

---

Deeper: [specs/2/28-risk.md](../../specs/2/28-risk.md),
[specs/2/20-network.md](../../specs/2/20-network.md),
[specs/2/45-tiles.md](../../specs/2/45-tiles.md)
