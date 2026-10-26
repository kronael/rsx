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
- `wal.rs` -- the `wal` commands `ls`, `verify`, `dump`, `follow`, `stats`
  and `dump_file`, and the record `Filters`.
- `segment.rs` -- `SegmentReader`: sequential frames of one segment file with
  a per-record CRC verdict. Unlike `WalReader` it keeps reading past a bad
  CRC, because the header length still locates the next record.
- `inspect.rs` -- `list_stream` and `verify_stream` over a stream directory,
  built on `SegmentReader` and `rsx-cast`'s `list_wal_files_across`; the
  `StreamListing` and `StreamVerdict` reports and their JSON.

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

`--follow` polls the stream at 100 ms. At EOF it compares the stream's
segment files (names and sizes) with the ones the reader was opened on; an
idle poll that sees no change reads nothing. After a change it re-opens the
reader at the last seen sequence number plus one and skips records below it,
so each record is printed once. `from_seq` is the first sequence number printed. A
bad record that has data after it ends the follow with exit 1, as in the dump.

### wal stats

The same reader and filters as `wal dump`, printing a count per record type
and a total.

### wal ls

Lists the numeric subdirectories of a WAL directory (or the `-s` streams).
Per stream: segment count, bytes, first and last sequence number and the
last `ts_ns`. The first record comes from the first segment that has an
intact one, the last from a scan of the last such segment.

### wal verify

Reads every segment of each stream with `SegmentReader` and checks:

- the CRC of every record (`crc_errors`; the record's own sequence number is
  not trusted, so the slot counts as present and no gap is reported for it);
- sequence continuity across the segments in order (`seq_gaps`, as
  inclusive `[from, to]` pairs); the first retained record may start above 1;
- a sequence number that does not increase, an unknown header version, or a
  partial record in a segment other than the last (`error`, the first one);
- a partial record at the end of the last segment (`torn_tail`): what a crash
  or a read of a live segment mid-write leaves. It is reported and is not
  corruption. A complete record with a bad CRC is a `crc_errors` entry even
  at the end of the last segment, where `WalReader` would read it as a torn
  tail.

The stream is corrupt when `crc_errors > 0`, `seq_gaps` is not empty or
`error` is set. The report goes to stdout whether or not any stream is
corrupt; a corrupt stream also exits 1 with the error line on stderr.

### JSON contracts

`--json` prints one object on stdout (`wal dump` prints JSON lines). Keys are
pinned by `tests/json_contract_test.rs`.

| Command | Output |
|---|---|
| `wal ls` | `{"wal_dir", "streams": [{"stream_id", "files", "bytes", "first_seq", "last_seq", "last_ts_ns"}]}` |
| `wal verify` | `{"status": "ok"\|"corrupt", "streams": [{"stream_id", "records", "first_seq", "last_seq", "crc_errors", "seq_gaps": [[from, to]], "torn_tail", "error"}]}` |
| `wal stats` | `{"stream_id", "records", "by_type": {"FILL": n, ...}}` |

`first_seq`, `last_seq`, `last_ts_ns` and `error` are `null` when absent.
`records` in `wal verify` counts the records whose CRC matched. An error
under `--json` is one `{"error": "<message>"}` line on stderr.

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
