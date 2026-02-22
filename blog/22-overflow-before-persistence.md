# Validate Before Persisting, Not After

Overflow bugs are bad. Overflow bugs that reach the WAL are permanent.

## Three Overflows, Same Codebase

We found three integer overflow bugs in a single audit pass. All three
share the same property: the corrupted value is written to a durable
store before anyone checks it.

**Bug 1: parse_price() in rsx-mark**

```rust
fn parse_price(raw: &str, scale: i64) -> Option<i64> {
    let whole_val: i64 = whole.parse().ok()?;
    let frac_val: i64 = frac_scaled.parse().ok()?;
    whole_val * scale + frac_val  // no checked_mul
}
```

`whole_val` comes from Binance or Coinbase. `scale` is configured by
the operator. For a price of 100,000 USDT with scale 1_000_000, the
multiplication is `100_000 * 1_000_000 = 100_000_000_000`. That's
within i64. But `whole_val` is unbounded — a malformed feed could
send `"92233720368547758"`, and `92233720368547758 * 1_000_000` wraps
to a negative i64. No error returned. `parse_price` returns
`Some(corrupted_price)`.

Fixed: `whole_val.checked_mul(scale)?` — if it overflows, return None
and skip the tick.

**Bug 2: liquidation_slip() in rsx-risk**

```rust
let slip = state.round * state.round * self.base_slip_bps;
```

`round` is a `u32`. `base_slip_bps` is `i64`. `round * round` as u32
can overflow before the i64 multiplication even begins. At round 65536,
`round * round` wraps to 0. The liquidation slip disappears. The user
gets the mark price with zero slippage on what should be a heavily
penalized late-stage liquidation.

Fixed: `(state.round as i64).checked_mul(state.round as i64)` then
`.checked_mul(self.base_slip_bps)`, capped at 9999 bps. The slip is
now bounded and correct.

**Bug 3: funding_rate() in rsx-risk**

The original calculation:

```rust
let premium = (mark - index) * 10_000 / index;
```

`mark` and `index` are i64 raw prices. The difference alone is fine.
But `(mark - index) * 10_000` multiplies a market-data-sourced delta
by 10,000. For BTC at index 90,000 in raw units with scale 1_000,
the delta in raw units could be thousands. `3_000 * 10_000 = 30_000_000`
— fine. But in production with scale 1_000_000, index raw values are
in the billions. `3_000_000_000 * 10_000` overflows i64
(`i64::MAX` ≈ 9.2 × 10^18, product ≈ 3 × 10^13 — tight).

Fixed: use i128 for the intermediate:

```rust
let premium = ((mark as i128 - index as i128)
    * 10_000
    / index as i128) as i64;
```

The i128 holds the intermediate cleanly. The final value is always
in bps range after the division, so the downcast to i64 is safe.

## Why Durability Makes This Worse

A corrupted price that stays in memory is bad. The next tick replaces
it, the mark price recovers, and you've had a brief incorrect
liquidation threshold.

A corrupted price that reaches the WAL is a different problem. WAL
records are replicated to all consumers immediately. The risk engine
applies the fill, the recorder archives it, the marketdata tile fans
it out. Recovery replays from the WAL tip. Every consumer rebuilds
state from the corrupted record.

There is no downstream check that says "this fill price looks
implausibly large." It's an i64. It's within the i64 range (it wrapped,
not overflowed to infinity). It looks like valid state.

To fix a corrupted WAL record you need to: stop all consumers, identify
the corrupted segment, roll back to the last clean tip, and replay from
a pre-corruption backup. For a live exchange, this means downtime and
manual reconciliation of every position that touched the corrupted fills.

The correct approach is pre-validate before writing:

```
boundary: parse external price string
  -> checked_mul at system boundary
  -> return None / Err on overflow
  -> drop the tick, log the anomaly
  -> do NOT propagate corrupted value
```

## Where Overflow Bugs Cluster

All three bugs were at the same class of boundary: externally sourced
inputs multiplied by a configured or market-derived scale factor.

- API boundary: external price strings (Binance, Coinbase)
- Config boundary: operator-supplied `base_slip_bps`, `price_scale`
- Market boundary: `mark - index` delta as market data input

These are not hot-path computations. They don't need to be fast. They
need to be correct. The rule is:

Use `checked_mul` at system boundaries. Use normal arithmetic on the
hot path. Check once at order entry. If the value passed entry
validation, it's safe to use in matching.

An overflowed price that reaches WAL is now persistent. Persistent
means replicated. Replicated means every consumer has it. Validate
before persisting, not after.

## See Also

- `rsx-mark/src/source.rs` - `parse_price()`, now uses `checked_mul`
- `rsx-risk/src/liquidation.rs` - `liquidation_slip()`, now uses
  `checked_mul` chain with 9999 cap
- `rsx-risk/src/funding.rs` - `calculate_rate()`, now uses i128
  intermediate
- `specs/v1/RISK.md` §5 - Funding rate calculation spec
