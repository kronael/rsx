# rsx-messages

The RSX exchange's application wire records — fixed
`#[repr(C, align(64))]` structs that flow over the `rsx-cast`
transport with no serialization step.

Eleven record types cover the order lifecycle, fills, BBO,
marks, and liquidations. Each implements `rsx_cast::CastRecord`,
so the same bytes travel over casting/UDP (live), replication/TCP
(replay), and the WAL (disk) unchanged — wire = stream = disk.
This crate is RSX-specific: it is reusable only alongside
`rsx-cast` (the transport) and `rsx-types` (the `Price`/`Qty`
newtypes the records embed). It is not a general-purpose message
library; it is the concrete record catalog one exchange happens
to send.

## Record catalog

```
FillRecord            fill happened (taker × maker)        128 B
BboRecord             best bid/offer snapshot              128 B
OrderInsertedRecord   order entered the book                64 B
OrderCancelledRecord  order left the book                   64 B
OrderDoneRecord       terminal: filled/resting/cancelled    64 B
OrderAcceptedRecord   risk pre-trade check passed          128 B
OrderFailedRecord     risk pre-trade check rejected         64 B
ConfigAppliedRecord   symbol-config version applied         64 B
MarkPriceRecord       aggregated mark price update          64 B
LiquidationRecord     forced reduce/close                   64 B
CancelRequest         user cancel command                   64 B
```

Every struct is `#[repr(C, align(64))]` with `_pad` fields to a
64- or 128-byte size, asserted at compile time
(`const _: () = assert!(size_of::<T>() == N)`). The first field
of each is `seq: u64`, which `CastRecord` reads and stamps.

## Public API

- The eleven record structs above.
- `RECORD_*: u16` — the record-type discriminants (`RECORD_FILL`
  = 0 … `RECORD_LIQUIDATION` = 13). These are the domain-layer
  type space; transport-level constants live in
  `rsx_cast::protocol`. A few discriminants
  (`RECORD_ORDER_REQUEST`, `RECORD_ORDER_RESPONSE`) are reserved
  here for records the gateway/risk tiles define, so the numeric
  space stays coordinated across crates.
- `CANCEL_REASON_*: u8` — cancel-reason codes carried in
  `OrderCancelledRecord.reason`.
- `encode_<record>(&r) -> Vec<u8>` — prepend the 16-byte
  `rsx-cast` header for a record (wraps `rsx_cast::encode_record`
  with the right `record_type`). Allocating; off the hot path.
- `decode_<record>(&[u8]) -> Option<T>` — read a record back out
  of a payload via `ptr::read_unaligned`; `None` if the slice is
  too short.

## Adding a record type

Local to this crate, no `rsx-cast` change:

1. Define a `#[repr(C, align(64))]` struct whose first field is
   `seq: u64`; pad to a 64-byte multiple and assert the size.
2. Pick an unused `RECORD_*` u16.
3. `impl CastRecord` (`seq`, `set_seq`, `record_type`).

The transport moves any `CastRecord`; it makes no assumptions
about these structs.

## Invariants

- **Wire = disk = stream.** No record is ever reformatted
  between the UDP frame, the TCP replay stream, and the WAL. The
  `#[repr(C)]` layout *is* the format.
- **Compile-time size lock.** Each record's size and alignment
  are asserted with `const _`, so an accidental field change that
  shifts the layout fails the build, not production.
- **`seq` first.** `CastRecord::seq`/`set_seq` assume `seq` is
  the leading `u64`; the transport stamps it in place.

## Dependencies

- `rsx-cast` — the transport + `CastRecord` trait + header codec.
- `rsx-types` — `Price`/`Qty` embedded in the record fields.

Nothing else. No serde (the layout is the format), no runtime.

## Testing

```
cargo test -p rsx-messages
```

Round-trip encode/decode + size/alignment tests in
`tests/records_test.rs`.

## MSRV

Rust 1.78+ on stable, edition 2021. No nightly features.

## See also

- ARCHITECTURE.md — layout rules, padding, and the `CastRecord`
  contract.
- The `rsx-cast` crate's README documents the transport and the
  16-byte header these records ride on.
- `specs/2/18-messages.md` in the wider rsx repo is the
  record-by-record field reference.

## License

Internal-use crate within the wider rsx exchange project.
Licensed under the MIT license. Not published to crates.io;
distribution is the maintainer's decision.
