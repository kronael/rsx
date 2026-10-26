# rsx-cli

CLI tools for RSX exchange operations.

## Commands

All WAL commands live under `wal`.

### wal dump

Dump the records of a stream.

```
cargo run -p rsx-cli -- wal dump <stream_id> <wal_dir> [from_seq] [flags]
```

`from_seq` opens the reader at the segment that holds that sequence number;
records before it in the same segment are still printed.

Flags:
- `--json` -- emit JSON lines instead of human text
- `--type <NAME>` -- filter by record type (repeatable, OR logic):
  `FILL`, `BBO`, `ORDER_INSERTED`, `ORDER_CANCELLED`, `ORDER_DONE`,
  `CONFIG_APPLIED`, `CAUGHT_UP`, `ORDER_ACCEPTED`, `MARK_PRICE`,
  `ORDER_REQUEST`, `ORDER_RESPONSE`, `CANCEL_REQUEST`,
  `ORDER_FAILED`, `LIQUIDATION`
- `--symbol <U32>` / `--user <U32>` -- filter by symbol or user
- `--from-ts <NS>` / `--to-ts <NS>` -- keep records with ts_ns in the window
- `--follow` -- after the last record keep polling for new ones, re-opening
  the reader on EOF (Ctrl-C to exit)
- `--tick-size <F64>` / `--lot-size <F64>` -- divide raw i64
  prices/quantities for text display (JSON always raw)

Records go to stdout; `total: N records` goes to stderr.

### wal stats

Per-type record counts and a total, for the records that pass the filters.

```
cargo run -p rsx-cli -- wal stats <stream_id> <wal_dir> [from_seq] [filter flags]
```

Takes the same `--type`, `--symbol`, `--user`, `--from-ts` and `--to-ts`
flags as `wal dump`.

### wal file

Dump a single raw WAL file as JSON lines (no stream reader required). It
stops at the first partial record and does not check CRCs.

```
cargo run -p rsx-cli -- wal file <file>
```

## Exit codes

| Code | Meaning                                       |
|------|-----------------------------------------------|
| 0    | success                                       |
| 1    | runtime error (I/O, WAL corruption)           |
| 2    | CLI misuse (unknown record type, bad flags)   |

A failure prints `Error: <message>` on stderr, or one `{"error": "<message>"}`
line under `wal dump --json`.

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

## How to read this crate

- **What** — this README: commands, exit codes, examples.
- **How** — [ARCHITECTURE.md](ARCHITECTURE.md): command
  internals and the bench-probe binary.
- **Why** — [`notes/`](notes/): design rationale for the
  one non-obvious data structure choice (priority queue in
  the bench probe).

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) -- command internals
