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
