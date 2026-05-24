# rsx-messages

RSX exchange application-level wire records on top of the
`rsx-dxs` transport.

Eleven `#[repr(C, align(64))]` records covering order events,
fills, BBO, marks, liquidations. Each implements
`rsx_dxs::CmpRecord` so it can flow over casting/UDP and replication/TCP
without serialization.

```
FillRecord                 fill happened (taker × maker)
BboRecord                  best bid/offer snapshot
OrderInsertedRecord        order entered the book
OrderCancelledRecord       order left the book
OrderDoneRecord            terminal: filled/resting/cancelled
OrderAcceptedRecord        risk pre-trade check passed
OrderFailedRecord          risk pre-trade check rejected
MarkPriceRecord            aggregated mark price update
LiquidationRecord          forced reduce/close
ConfigAppliedRecord        symbol-config version applied
CancelRequest              user cancel command
```

The transport (`rsx-dxs`) makes no assumptions about these —
they're consumer-defined records that happen to be the ones
RSX uses. Adding a new record type is local: define a
`#[repr(C, align(64))]` struct, pick a `RECORD_*` u16,
implement `CmpRecord`. No edit to `rsx-dxs` required.

## Architectural Decisions

**Runtime: none — wire records only.** `rsx-messages` is a
library of `#[repr(C, align(64))]` structs and the
`CmpRecord` impl for each. No runtime, no I/O, no threading.
The records travel through `rsx-dxs` transport (streaming
protocol casting over UDP, replay protocol replication over TCP, WAL on
disk) without being aware of which path they're on. See
[`../notes/tiles.md`](../notes/tiles.md) for the runtime
choices made by the producers and consumers of these records.

## See also

- `rsx-dxs/README.md` — transport
- `specs/2/18-messages.md` — record-by-record reference
