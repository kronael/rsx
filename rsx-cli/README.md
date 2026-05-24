# rsx-cli

CLI tools for RSX exchange operations.

## Commands

### wal-dump

Dump WAL records for a given stream.

```
cargo run -p rsx-cli -- wal-dump <stream_id> <wal_dir> [from_seq] [flags]
```

Flags:
- `--json` -- emit JSON lines instead of human text
- `--type <NAME>` -- filter by record type (repeatable, OR logic):
  `FILL`, `BBO`, `ORDER_INSERTED`, `ORDER_CANCELLED`, `ORDER_DONE`,
  `CONFIG_APPLIED`, `CAUGHT_UP`, `ORDER_ACCEPTED`, `MARK_PRICE`,
  `ORDER_REQUEST`, `ORDER_RESPONSE`, `CANCEL_REQUEST`,
  `ORDER_FAILED`, `LIQUIDATION`
- `--symbol <U32>` / `--user <U32>` -- filter by symbol or user
- `--from-ts <NS>` / `--to-ts <NS>` -- filter by ts_ns window
- `--stats` -- per-type counts only (mutually exclusive with --follow)
- `--follow` -- tail WAL for new records, re-opens reader on EOF
  (Ctrl-C to exit)
- `--tick-size <F64>` / `--lot-size <F64>` -- divide raw i64
  prices/quantities for text display (JSON always raw)

### dump

Dump a single raw WAL file as JSON lines (no stream reader required).

```
cargo run -p rsx-cli -- dump <file>
```

## Exit codes

| Code | Meaning                                       |
|------|-----------------------------------------------|
| 0    | success                                       |
| 1    | runtime error (`die()`: I/O, WAL corruption)  |
| 2    | CLI misuse (`misuse()`: unknown record type)  |

## Building

```
cargo build -p rsx-cli
```

## Dependencies

- `clap` -- CLI argument parsing
- `ctrlc` -- SIGINT handling for `--follow`
- `serde_json` -- JSON output
- `rsx-cast` -- WAL reader, header decoding
- `rsx-messages` -- record type constants and payload structs

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) -- command internals
