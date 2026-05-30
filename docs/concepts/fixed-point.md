# Fixed-Point Arithmetic

Every price and quantity in RSX is an `i64` in the smallest
representable unit. The `Price` and `Qty` types are
`#[repr(transparent)]` newtypes wrapping `i64`. Floating-point
arithmetic does not appear on the hot path at any point.

## Why not float

The short answer is reproducibility. IEEE 754 floating-point
is non-deterministic in a specific sense: the result of an
operation can differ between architectures, between compilers,
and between optimization levels. An exchange that uses floats
cannot guarantee that two replicas computing the same
sequence of fills will arrive at the same position, the same
PnL, the same liquidation threshold. Replay would not be
deterministic. Recovery would not be reliable.

The practical answer is rounding. Suppose BTC is at $50 000.00
and a user buys 0.001 BTC. The notional in float is
50 000.00 × 0.001 = 50.000000000000004 on most x86 hardware.
Repeated across thousands of fills, rounding errors accumulate.
Every production exchange uses integer arithmetic for this
reason.

## How it works

The conversion from human-readable units to raw integers
happens once, at the API boundary:

```
price_raw = floor(human_price / tick_size) as i64
qty_raw   = floor(human_qty   / lot_size)  as i64
```

Inside the matching engine and risk engine, everything is
integer comparison, integer addition, and integer multiplication.
No floating-point unit is involved. Integer multiply is 3
cycles on modern x86; float multiply is 5 cycles plus rounding
plus potential NaN propagation.

Overflow is caught at order entry, not on the hot path. The
notional of an order — price times quantity — is computed with
`checked_mul` at the risk boundary. If it overflows `i64`,
the order is rejected. Once an order is accepted, all
subsequent arithmetic is within bounds by construction.

## The newtype discipline

`Price(pub i64)` and `Qty(pub i64)` are separate types. The
compiler rejects multiplying a `Price` by a `Price`, or
passing a `Qty` where a `Price` is expected. This eliminates
an entire class of unit-confusion bugs that are common in
systems that pass `f64` everywhere with an implicit "dollars"
or "contracts" convention.

Fields in wire structs are named `px` and `qty` in WAL/wire
contexts to keep struct sizes predictable. At the API boundary,
`price_decimals` and `qty_decimals` from `SymbolConfig`
reconstruct the human-readable form.

## What you give up

The API boundary conversion is lossy if the input is already
a float and the tick size does not divide it evenly. A client
submitting a price of $50 000.005 on a $0.01 tick symbol will
be floored to $50 000.00 or rejected, depending on validation
policy. This is correct exchange behavior — all valid prices
are multiples of the tick size — but it means the gateway must
validate alignment before accepting the order.

There is no decimal library, no rational arithmetic, no
arbitrary precision. The system is correct precisely because
the domain constrains it: tick sizes and lot sizes are chosen
such that all valid prices and quantities fit in `i64` with
no rounding.

---

Deeper: [blog/18-100ns-matching.md](../../blog/18-100ns-matching.md),
[specs/2/21-orderbook.md](../../specs/2/21-orderbook.md)
