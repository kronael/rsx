# rsx-cli Architecture

CLI tools for RSX exchange operations.

## Commands

### wal-dump

Reads WAL files via `WalReader::open_from_seq()` from rsx-dxs.
Iterates all records sequentially, printing sequence number,
record type name, payload length, and CRC32.

Uses clap for argument parsing. Recognizes all standard record
types: FILL, BBO, ORDER_INSERTED, ORDER_CANCELLED, ORDER_DONE,
CONFIG_APPLIED, CAUGHT_UP, ORDER_ACCEPTED, MARK_PRICE,
ORDER_REQUEST, ORDER_RESPONSE, CANCEL_REQUEST, ORDER_FAILED.

Unknown record types are printed as `UNKNOWN(N)`.

## Architectural Decisions

**Runtime: tokio.** The CLI does sequential file I/O and
occasional async work (Postgres queries, `--follow` polling
on EOF). tokio is the boring choice and matches what the
inspected components already use. There is no hot loop here
and no latency budget — the CLI runs offline, not on the
GW→ME→GW critical path. See
[`../notes/tiles.md`](../notes/tiles.md) for when tiles or
monoio apply instead.
