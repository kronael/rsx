# rsx-types Architecture

Foundation crate: fixed-point numeric types, order enums,
symbol configuration, panic handling, and the CPU/cache helpers
every pinned tile needs. Everything in the project depends on
this crate; this crate depends only on `core_affinity` and
`libc` (both used solely by the hot-thread setup).

## Key types

- `Price(pub i64)` — price in tick units, `#[repr(transparent)]`.
- `Qty(pub i64)` — quantity in lot units, `#[repr(transparent)]`.
- `Side` — `Buy = 0`, `Sell = 1`, `#[repr(u8)]`.
- `TimeInForce` — `GTC = 0`, `IOC = 1`, `FOK = 2`.
- `FinalStatus` — `Filled = 0`, `Resting = 1`, `Cancelled = 2`.
- `OrderStatus` — `Filled`, `Resting`, `Cancelled`, `Failed`.
- `FailureReason` — the rejection catalog (invalid tick/lot,
  symbol-not-found, dedup, margin, rate limit, wrong shard, …),
  `#[repr(u8)]` with explicit discriminants.
- `SymbolConfig` — `symbol_id`, `price_decimals`, `qty_decimals`,
  `tick_size`, `lot_size`.
- `NONE` — `u32::MAX`, the sentinel for empty slab / level
  linked-list pointers.

## Module layout

| File | Purpose |
|------|---------|
| `lib.rs` | `Price`/`Qty` newtypes, order enums, `SymbolConfig`, `validate_order`, `NONE`, module re-exports |
| `cpu.rs` | `setup_hot_thread`, `HotSetup`, `pin_current`, `mlock_all`, `warm_stack`, `core_is_isolated`, `parse_cpu_list` |
| `cache.rs` | `Padded<T>`, `LINE` (64), `PAD` (128) — false-sharing layout primitives |
| `macros.rs` | `install_panic_handler()` — panic-hook installer used by every binary |
| `time_utils.rs` | `time_ns()`, `time_ms()`, `time()` — epoch timestamps |

## Design decisions

- **`#[repr(transparent)]` on `Price`/`Qty`.** Zero-cost
  newtypes with the exact layout of `i64`, so they drop into
  `#[repr(C)]` wire records and cast over the transport without
  a conversion step.
- **Explicit enum discriminants.** Every order enum fixes its
  integer values so the wire encoding is stable across builds
  and across processes. A discriminant change is a wire break.
- **`validate_order` at the entry boundary.** Checks
  `price > 0 && qty > 0 && price % tick_size == 0 && qty % lot_size == 0`.
  It is the one alignment gate; the matching engine downstream
  assumes its inputs are already validated.
- **`install_panic_handler` = fail-fast.** Replaces the default
  hook to print and `exit(1)` on any thread panic. A pinned tile
  that panics must take the process down, not leave a dead
  thread and a half-live daemon.

## Hot-thread setup (cpu.rs)

`setup_hot_thread(core)` is the one place that prepares a thread
for a pinned busy-loop, in order:

1. `pin_current(core)` — set core affinity.
2. `warm_stack()` — write one byte per page `STACK_WARM_KIB`
   (256) deep to pre-fault the stack (write, not read, to break
   copy-on-write and the shared zero page).
3. `mlock_all()` — `mlockall(MCL_CURRENT | MCL_FUTURE)` to pin
   the now-mapped pages and pre-fault future ones.
4. `core_is_isolated(core)` — read
   `/sys/devices/system/cpu/isolated` and report whether the
   core is in the kernel's isolated set.

It returns a `HotSetup` recording what it achieved (`pinned`,
`mlocked`, `isolated`, `stack_warm_kb`). Each step is
best-effort: denial degrades the flag, never aborts the caller.
rsx-types does no logging itself — the caller logs the
`HotSetup` (its `Display` impl is one line), which is why the
crate needs no logging dependency.

## Cache-line layout (cache.rs)

`Padded<T>` is `#[repr(align(128))]`. `LINE` (64) is the real
cache-line size for reasoning about which fields co-locate;
`PAD` (128) is the anti-false-sharing alignment — 128 rather
than 64 because Intel's adjacent-line prefetcher pulls lines in
pairs, so the unit of destructive interference is two lines.
`crossbeam-utils`' `CachePadded` picks the same 128 on
x86-64/aarch64. Wrap a datum in `Padded` when two threads each
write their own value that would otherwise land on one line.

## Runtime

None. rsx-types defines wire-stable primitives and best-effort
setup helpers; it has no async runtime, no threading of its own,
and no I/O beyond the single `/sys` read in `core_is_isolated`.
The primitives travel unchanged through tiles, monoio reactors,
and tokio sidecars alike. The runtime choices made downstream
are documented by the consuming crates, not here.
