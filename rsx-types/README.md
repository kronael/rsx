# rsx-types

Fixed-point newtypes, order enums, and hot-thread setup
helpers shared by every RSX exchange crate.

This is the leaf crate: it depends on nothing in the project
and everything else depends on it. It carries the primitives
that must have one definition across processes — the i64
fixed-point `Price`/`Qty` newtypes that travel unchanged over
the wire, the order-lifecycle enums with wire-stable
discriminants, and the CPU/cache helpers a pinned busy-loop
tile needs before it starts spinning.

## What it provides

- **`Price(pub i64)` / `Qty(pub i64)`** — `#[repr(transparent)]`
  newtypes. Both hold raw integer counts of ticks / lots; there
  is no float anywhere. `1` = one tick / one lot. Human-readable
  conversion happens only at the API boundary (gateway,
  marketdata), never on the hot path.
- **Order enums** — `Side` (`Buy`/`Sell`), `TimeInForce`
  (`GTC`/`IOC`/`FOK`), `FinalStatus`, `OrderStatus`,
  `FailureReason`. All `#[repr(u8)]` with explicit
  discriminants so their integer values are wire-stable.
- **`SymbolConfig`** — per-symbol `tick_size`, `lot_size`,
  `price_decimals`, `qty_decimals`, `symbol_id`.
- **`validate_order(config, price, qty)`** — the tick/lot
  alignment + positivity check, applied at order entry.
- **`NONE` (`u32::MAX`)** — the sentinel for "no index" in
  slab and price-level linked lists.
- **`cpu::setup_hot_thread(core)`** — pin the current thread to
  a core, `mlockall` its address space, pre-fault the stack,
  and report core isolation. Best-effort and non-fatal; returns
  a `HotSetup` the caller logs.
- **`cache::Padded<T>`** — 128-byte-aligned wrapper that gives a
  value its own cache-line span to defeat false sharing between
  independently-written data (a producer tail vs a consumer
  head, two per-thread counters).
- **`install_panic_handler()`** — replaces the panic hook with
  one that prints and `exit(1)`s, so any thread panic crashes
  the process rather than silently wedging a tile.
- **`time_utils::{time_ns, time_ms, time}`** — epoch timestamps
  at ns / ms / s resolution.

## Public API

```rust
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::validate_order;
use rsx_types::install_panic_handler;
use rsx_types::cpu::setup_hot_thread;
use rsx_types::cache::Padded;
use rsx_types::time_utils::time_ns;
```

The re-exports live at the top of `src/lib.rs`; the `cpu`,
`cache`, `macros`, and `time_utils` modules are public.

## Invariants and guarantees

- **`Price`/`Qty` are `#[repr(transparent)]`**: identical layout
  to `i64`, so a record field of type `Price` is byte-compatible
  with an `i64` on the wire and in a `#[repr(C)]` struct. No
  conversion cost, no layout surprise.
- **Enum discriminants are explicit and stable.** A `Side::Sell`
  is `1` today and forever; changing a discriminant is a
  wire-format break, not a refactor.
- **`validate_order` is the single alignment gate.** It returns
  `true` only when `price > 0`, `qty > 0`, `price % tick_size == 0`,
  and `qty % lot_size == 0`.
- **`setup_hot_thread` never aborts the caller.** Pinning or
  `mlock` denial (no `CAP_IPC_LOCK`, a non-isolated dev box)
  degrades to a `HotSetup` with the relevant flag `false`; the
  tile still runs, just without the tail-latency guarantees.

## Dependencies

Two, both minimal and both for the hot-thread helpers only:

- `core_affinity` — pins a thread to a logical core.
- `libc` — the `mlockall` FFI call.

The numeric types, enums, `SymbolConfig`, and `validate_order`
pull in nothing. There is no async runtime, no serialization
framework, no I/O.

## Testing

```
cargo test -p rsx-types
```

Integration tests in `tests/types_test.rs`; unit tests for the
CPU/cache helpers in `src/cpu_test.rs` and `src/cache_test.rs`.

## MSRV

Rust 1.78+ on stable, edition 2021. No nightly features. The
MSRV floor is not pinned in `Cargo.toml`; a future bump lands in
a `0.x` minor version.

## See also

- [ARCHITECTURE.md](ARCHITECTURE.md) — type layouts, module map,
  and design decisions.
- `notes/hot-path.md` — the rules `cpu::setup_hot_thread` and
  `cache::Padded` enforce (why pin, why `mlock`, how to find
  false sharing).

## License

Internal-use crate within the wider rsx exchange project.
Licensed under `MIT OR Apache-2.0`. Not published to
crates.io; distribution is the maintainer's decision.
