---
status: shipped
---

# CLI.md — rsx-cli, the exchange command-line tool

## Purpose

`rsx-cli` is the one tool operators, developers, the playground and the
tests use to inspect and drive the exchange. It reads WAL files, checks the
exchange invariants, starts and stops the processes of a scenario, places
orders, drives load, injects faults, and reads and extends the symbol config
and the accounts. The playground and the demo scripts call it and render what
it prints; they hold no exchange logic.

The flags, exit codes and JSON shapes of every command are in
[`rsx-cli/README.md`](../../rsx-cli/README.md). This spec states what the
tool must do.

---

## Commands

| Command | Requirement |
|---|---|
| `wal ls\|verify\|dump\|stats\|types\|file` | Read WAL files offline. Never write a WAL file. |
| `verify` | Check the ten correctness invariants over WAL directories and, with a database, over risk's tables. Report `skip` for a check that examined nothing. |
| `md snapshot\|follow` | Read the public market-data feed and decode it with `rsx_marketdata::wire::decode`. |
| `token mint` | Print a gateway JWT. |
| `order place\|cancel\|batch` | Send orders through the gateway WebSocket and report how each order ended. |
| `stress` | Drive crossing IOC orders open-loop, a closed-loop latency probe or the reject path at a gateway. |
| `cluster scenarios\|plan\|up\|down\|ps\|restart\|reset` | The one launcher of the exchange processes. |
| `fault kill\|net\|wal-corrupt\|dup-order` | Inject one fault into a running cluster or a WAL. |
| `config show\|schedule` | Read the symbol config and insert the next version of a symbol. Never update an existing row. |
| `account seed\|show`, `risk overview` | Seed and read the accounts risk persists. |

Every command prints text by default and one JSON document, or one JSON object
per record for a WAL stream, under `--json`.

---

## WAL commands

### Output

`wal dump` prints one line per record. Header fields always present: `seq`,
`type`, `len`, `crc`; the record's own fields follow on the same line.

```
seq=1042     type=FILL               len=128  crc=0x00abcd12 sym=1 user=5 ...
```

Under `--json` a record is one JSON object with `seq`, `type`, `ts_ns`, `len`,
`crc32` and the record's own fields. Order ids are rendered as hex strings.
Text mode may scale raw prices and quantities for display (`--tick-size`,
`--lot-size`); JSON always carries raw values.

### Record types decoded

`wal types` lists the names. There are 14:

`FILL`, `BBO`, `ORDER_INSERTED`, `ORDER_CANCELLED`, `ORDER_DONE`,
`ORDER_ACCEPTED`, `ORDER_FAILED`, `ORDER_REQUEST`, `ORDER_RESPONSE`,
`CANCEL_REQUEST`, `CONFIG_APPLIED`, `CAUGHT_UP`, `MARK_PRICE`, `LIQUIDATION`.

### Filters, stats and follow

`wal dump` and `wal stats` take `--type` (repeatable, OR'd), `--symbol`,
`--user`, `--from-ts` and `--to-ts`. `wal stats` prints a count per record type
and a total. `wal dump --follow` keeps polling for new records after the last
one, prints each once and exits on SIGINT.

---

## Acceptance Criteria

- `wal dump` with no filters prints all records in seq order
- `--type FILL --symbol 1` prints only FILL records for symbol 1
- `wal stats` prints per-type counts and a total, and exits 0
- `--follow` does not exit after the last record; it prints new ones
- `--json` output is valid NDJSON parseable by `jq`
- All 14 record types decode without a panic on valid WAL data
- `from_seq` 0 and an absent `from_seq` behave identically
- `wal verify` exits 1 on a CRC error or a sequence gap
- `verify` exits 1 when a check fails
- A bad flag exits 2; a runtime failure exits 1
