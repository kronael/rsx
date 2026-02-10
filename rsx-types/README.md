# rsx-types

Shared newtypes, enums, and utilities used by all RSX crates.

## What It Provides

- `Price(i64)`, `Qty(i64)` -- fixed-point newtypes
- `Side`, `TimeInForce`, `OrderStatus`, `FailureReason` enums
- `SymbolConfig` -- per-symbol tick/lot/decimal configuration
- `validate_order()` -- tick/lot alignment checks
- `install_panic_handler()` -- exit(1) on any thread panic
- `time_ns()`, `time_us()`, `time_ms()` -- epoch timestamps
- Utility macros: `defer!`, `on_error_continue!`,
  `on_none_continue!`

## Public API

```rust
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::validate_order;
use rsx_types::install_panic_handler;
use rsx_types::time::time_ns;
```

## Building

```
cargo check -p rsx-types
cargo build -p rsx-types
```

No features, no external dependencies.

## Testing

```
cargo test -p rsx-types
```

Tests in `tests/types_test.rs`.

## Dependencies

None. This is the leaf crate -- everything depends on it.

## Gotchas

- All prices/quantities are i64 in raw tick/lot units.
  Conversion to/from human-readable happens at API
  boundary only (gateway, marketdata).
- `NONE` (`u32::MAX`) is the sentinel for empty slab
  pointers -- do not use as a valid index.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) -- type layouts,
  design decisions
