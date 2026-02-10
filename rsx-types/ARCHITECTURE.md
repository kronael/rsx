# rsx-types Architecture

Foundation crate with zero external dependencies. Defines
fixed-point numeric types, order enums, symbol configuration,
and utility macros used across the exchange.

## Key Types

- `Price(pub i64)` -- price in smallest tick units,
  `#[repr(transparent)]`
- `Qty(pub i64)` -- quantity in smallest lot units,
  `#[repr(transparent)]`
- `Side` -- `Buy = 0`, `Sell = 1`, `#[repr(u8)]`
- `TimeInForce` -- `GTC = 0`, `IOC = 1`, `FOK = 2`
- `FinalStatus` -- `Filled`, `Resting`, `Cancelled`
- `OrderStatus` -- `Filled`, `Resting`, `Cancelled`, `Failed`
- `FailureReason` -- validation failures, margin, dedup,
  rate limit, etc.
- `SymbolConfig` -- per-symbol parameters: `symbol_id`,
  `tick_size`, `lot_size`, `price_decimals`, `qty_decimals`
- `SlabIdx` -- `u32` alias for slab handles
- `NONE` -- `u32::MAX` sentinel for empty linked list pointers

## Module Layout

| File | Purpose |
|------|---------|
| `lib.rs` | Type definitions, `validate_order()`, re-exports |
| `macros.rs` | `install_panic_handler()`, `DeferCall`, `defer!`, `on_error_continue!`, `on_none_continue!`, `on_error_return_ok!`, `on_none_return_ok!` |
| `time.rs` | `time_ns()`, `time_us()`, `time_ms()`, `time()` -- epoch timestamps at various resolutions |

## Design Decisions

- `#[repr(transparent)]` on Price/Qty: zero-cost newtypes,
  same layout as i64 for FFI/CMP wire compatibility
- All enums use explicit discriminants for wire stability
- `validate_order()` checks price > 0, qty > 0, tick
  alignment, lot alignment -- called at order entry boundary
- `install_panic_handler()` replaces default panic hook to
  `exit(1)` on any thread panic -- called in every binary
