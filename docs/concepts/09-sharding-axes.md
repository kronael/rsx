# Sharding Axes

RSX has two independent axes of scale-out. They do not
interfere with each other.

## The two axes

Risk shards by `user_id`. Each shard owns a set of users and
holds all of their positions and margin in RAM. The routing key
is a fixed virtual shard, `vshard = hash(user_id) % N_VSHARDS`,
mapped through a mutable `shardmap` table to a node — so the
cluster can grow without a global reshuffle. A shard processes
every symbol for its users
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

## Why matching can't shard finer than a symbol

A symbol is one market. Every buy and sell order for BTC-PERP has
to meet in a single book, because that is the only way to hold
price-time priority: the best bid must trade with the best ask,
and the order that arrived first at a price must fill first. Split
one symbol's book across two machines and you lose that — a taker
could match a worse price on shard A while a better resting order
sits on shard B, and no amount of cross-shard chatter fixes it
without serializing back into one place. So within a symbol the
book is centralized and single-threaded by necessity, not by
choice.

That is why the only clean matching axis is *across* symbols —
one engine per instrument. You add BTC-PERP and ETH-PERP as
separate engines; you cannot add "half of BTC-PERP."

And it is why matching has to be fast. A symbol's whole order flow
funnels through one core with no parallelism to hide behind, so
that core's per-order time is the symbol's throughput ceiling. A
100 ns match is ~10M orders/sec for that symbol; a 10 µs match is
100k. There is no scaling around it — you make the one book fast,
which is the entire point of `rsx-book`. Users scale out on the
Risk axis; a single hot symbol scales only by getting the match
itself cheaper.

## The path of an order

User U sends an order on symbol S:

```
Gateway → Risk[U] → ME[S] → Risk[U] → Gateway
```

Gateway routes by `user_id`. Risk[U] validates margin and
freezes collateral (worst-case: full notional plus fees), then
routes by `symbol_id` to ME[S]. ME[S] matches against the book,
appends the fill to its WAL, and sends the fill back to Risk[U].
Risk applies the settlement — updates the position, releases the
frozen margin — and forwards the fill to the gateway, which
delivers it to the client. Five hops, one book per symbol, one
margin authority per user.

Freezing worst-case at entry and settling on the return path
means the client never sees margin freed before the fill: margin
is consistent from the client's view, and solvency is never at
risk. A planned optimization (see `28-risk.md`) would send the
fill from ME straight to the gateway and settle to Risk
asynchronously — shaving hops at the cost of eventually-consistent
margin — but that split is not implemented today.

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
