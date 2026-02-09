# Progress

## Timeline

```
Feb 7 22:13  first commit (networking spec)
Feb 8 23:15  all 36 CRITIQUE.md items resolved
Feb 9 06:58  orderbook + matching engine shipped
Feb 9 07:47  refined (zero warnings)
```

33 hours from first spec to working matching engine.
35 commits. 8,856 lines of spec. 1,358 lines of impl.
1,115 lines of tests. 75 tests passing.

## What Shipped

Three crates: `rsx-types`, `rsx-book`, `rsx-matching`.

**rsx-types** (55 lines) — Price/Qty newtypes (i64,
repr(transparent)), Side/TimeInForce enums, SymbolConfig,
validate_order.

**rsx-book** (1,263 lines) — The core orderbook:
- Slab arena allocator (generic, O(1) alloc/free)
- CompressionMap (5-zone price indexing, ~617K slots)
- PriceLevel (24 bytes, compile-time assert)
- OrderSlot (128 bytes, align(64), compile-time assert)
- Matching algorithm (GTC/IOC/FOK, smooshed tick support)
- Incremental CoW recentering (frontier-based migration)
- User position tracking (reduce-only enforcement)
- Event buffer (fixed array, no heap)

**rsx-matching** (40 lines) — Binary stub with main loop
skeleton, panic handler, busy-spin.

## Shocking Parts

**The sonnet.** The README opens with a love poem to the
exchange architecture. "Thy slab did catch me: firm,
pre-allocated / No malloc on thy hot path — O! how pure."

**36 critique items before any code.** The spec went through
a full adversarial audit (9 critical, 12 high, 15 medium)
and every item was resolved before writing a single line
of Rust. Items like "IOC/FOK missing from matching",
"dedup window unsafe across restarts", "no clock sync for
funding settlement".

**8,856 lines of spec for 1,358 lines of code.** 6.5:1
spec-to-code ratio. The specs cover matching, risk, WAL,
liquidation, mark price, gateway, market data, gRPC, and
testing — most of which isn't implemented yet.

**The infinite loop bug.** First test run hung on
`match_no_cross_taker_rests`. A buy at 50,100 with best
ask at 50,200 should rest without matching. But the
matching loop checked `remaining_qty > 0 && best_ask != NONE`
without verifying the price actually crosses. The smooshed
tick check inside `match_at_level` skipped all orders but
never signaled "no match possible" to the outer loop.
Fix: track remaining_qty before/after, break if unchanged.

**The hook that keeps reverting Cargo.toml.** A pre-commit
or post-write hook keeps resetting the workspace members
list to `["rsx-dxs", "rsx-recorder"]`. Every edit to any
file triggers it. Had to re-add rsx-types/rsx-book/
rsx-matching to the members list at least 4 times during
the session.

**128 bytes of OrderSlot, designed by hand.** Hot fields
(price, qty, side, flags, next/prev) in cache line 1.
Cold fields (user_id, timestamp, original_qty) in cache
line 2. 40 bytes of explicit padding to hit exactly 128.
Compile-time size and alignment asserts enforce it.

**CompressionMap: 617K slots instead of 20M.** Naive
price-to-index for BTC at $0.01 ticks covering $1K-$200K
needs 20M slots (477 MB). Five compression zones reduce
this to ~617K slots (~14.8 MB) while keeping 1:1
resolution near the mid price.

## What's Next

Per the crate layout in CLAUDE.md, remaining crates:
- `rsx-dxs` — WAL writer/reader (partially exists)
- `rsx-risk` — risk engine binary
- `rsx-mark` — mark price aggregator
- `rsx-gateway` — WS + gRPC
- `rsx-marketdata` — market data fan-out
- `rsx-recorder` — archival consumer (partially exists)

The matching engine stub needs SPSC ring wiring and WAL
integration. The main loop is a bare busy-spin waiting
for those connections.
