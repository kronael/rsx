# rsx-cli Architecture

CLI tools for RSX exchange operations.

## Layout

`main.rs` holds the clap tree and dispatches to library functions; the
library (`lib.rs`) is flat:

- `output.rs` -- `Failure` (runtime exits 1, misuse exits 2), `report`
  (the stderr error line, plain or `{"error": ...}`), and `print_record`
  for the text or JSON `Format`.
- `record.rs` -- WAL record to JSON object or text line, for every record
  type the CLI decodes; record type names; `DisplayScale`.
- `wal.rs` -- `dump`, `follow`, `stats` and `dump_file`, and the record
  `Filters`.

## Commands

### wal dump

Reads WAL files via `WalReader::open_from_seq()` from rsx-cast.
Iterates all records sequentially, printing sequence number,
record type name, payload length, and CRC32C. A bad record that has data
after it ends the dump with exit 1.

Recognizes the record types FILL, BBO, ORDER_INSERTED, ORDER_CANCELLED,
ORDER_DONE, CONFIG_APPLIED, CAUGHT_UP, ORDER_ACCEPTED, MARK_PRICE,
ORDER_REQUEST, ORDER_RESPONSE, CANCEL_REQUEST, ORDER_FAILED, LIQUIDATION.

Unknown record types are printed as `UNKNOWN` with the header fields only.

`--follow` polls the stream at 100 ms. At EOF it re-opens the reader at the
last seen sequence number plus one and skips records below it, so each
record is printed once. `from_seq` is the first sequence number printed.

### wal stats

The same reader and filters as `wal dump`, printing a count per record type
and a total.

### wal file

Parses the 16-byte headers of one raw file in memory and prints each record
as JSON. It stops at the first partial record and does not check CRCs.

## Architectural Decisions

**Runtime: tokio.** The CLI does sequential file I/O and
occasional async work (Postgres queries, `--follow` polling
on EOF). tokio is the boring choice and matches what the
inspected components already use. There is no hot loop here
and no latency budget — the CLI runs offline, not on the
GW→ME→GW critical path. See
[`../docs/concepts/tiles-and-pinning.md`](../docs/concepts/tiles-and-pinning.md) for when tiles or
monoio apply instead.
