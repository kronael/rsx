# Orderbook V2 Considerations

**Status:** Not planned. This document is archival only.

Future extensions that are NOT in v1. Kept separate to avoid polluting the v1 design.

---

## Variable Market Tick Sizes (Price-Dependent)

In v1, each symbol has a single constant tick size and lot size. In v2, these
could vary with price to better serve assets spanning huge price ranges.

### The Problem

Crypto assets span enormous price ranges (BTC at $50,000 vs a memecoin at $0.000001).
A single tick size doesn't work — $0.01 is meaningless for a $0.0001 asset, and
$0.000001 is wasteful for BTC. The same applies to quantities.

### Solution: Price-Dependent Tick Size Bands

Each symbol defines a **tick size curve** — a set of price bands, each with its own
tick size. As the price moves between bands, the minimum price increment changes.

```rust
/// A single band in the tick size curve
struct TickBand {
    /// Price threshold (in base ticks) — this band applies when price >= min_price
    min_price: i64,
    /// Tick size for this band (in base units)
    tick_size: i64,
    /// Lot size for this band (in base qty units)
    lot_size: i64,
}

/// Symbol configuration (v2)
struct SymbolConfigV2 {
    symbol_id: u32,
    price_decimals: u8,
    qty_decimals: u8,
    /// Tick size bands, sorted by min_price ascending
    tick_bands: Vec<TickBand>,
}
```

### Example: BTC-PERP Tick Curve

```
Price Range          Tick Size    Lot Size
$0 - $100            $0.001       0.01 BTC
$100 - $1,000        $0.01        0.001 BTC
$1,000 - $10,000     $0.10        0.001 BTC
$10,000 - $100,000   $1.00        0.0001 BTC
$100,000+            $10.00       0.0001 BTC
```

### Example: SHIB-PERP Tick Curve

```
Price Range              Tick Size        Lot Size
$0 - $0.0001            $0.00000001      1,000,000 SHIB
$0.0001 - $0.001        $0.0000001       100,000 SHIB
$0.001 - $0.01          $0.000001        10,000 SHIB
$0.01+                  $0.00001         1,000 SHIB
```

### How Tick Size Changes Affect the Orderbook

When price moves between bands (tick size changes):

1. **Orders already resting remain valid** — they were placed at a valid tick when submitted
2. **New orders must conform to the current tick size** for their price level
3. **Validation at order entry**: `order_price % current_tick_size(order_price) == 0`

### Implementation: Tick Size Lookup (bisection for v2)

```rust
impl SymbolConfigV2 {
    fn tick_size_at(&self, price: Price) -> i64 {
        // Binary search on bands for O(log n) lookup
        let idx = self.tick_bands.partition_point(|b| b.min_price <= price.0);
        self.tick_bands[idx.saturating_sub(1)].tick_size
    }

    fn validate_price(&self, price: Price) -> bool {
        let tick = self.tick_size_at(price);
        price.0 % tick == 0
    }
}
```

### Lot Size Curves

Same principle applies to quantities — lot size bands determine the minimum order
size increment at different price ranges. This ensures notional value per lot
stays reasonable:

- High-price assets: small lot sizes (0.0001 BTC)
- Low-price assets: large lot sizes (1,000,000 SHIB)
- **Validation**: `order_qty % current_lot_size(order_price) == 0`

### Reconfiguration (Tick Size Changes)

When an admin changes the tick curve for a symbol:

1. Existing resting orders that no longer align to the new tick grid can be:
   - **Left in place** (grandfather clause — simplest, no disruption)
   - **Force-cancelled** (cleaner book, more disruptive)
2. The compression indexing is independent of market tick size — no array restructuring
3. Broadcast the new tick curve to connected clients

### Interaction with Compression Zones

Variable market ticks are orthogonal to compression zones. Compression groups
ticks by distance from mid (internal indexing). Market tick bands define what
prices are valid for orders (external validation). Both can coexist — the
compression works the same regardless of whether the market tick is constant or variable.
