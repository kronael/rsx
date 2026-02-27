# CLI.md — rsx-cli WAL Inspection Tool

## Purpose

`rsx-cli` is an offline WAL debugging tool. It reads WAL files written
by `rsx-dxs` and prints records in human-readable or JSON format.
Intended for operators and developers inspecting exchange state.

---

## Current Commands

### wal-dump

Stream records from a WAL directory for a given stream.

```
rsx-cli wal-dump <stream_id> <wal_dir> [--from-seq <n>] [--json]
```

- `stream_id`: integer stream identifier (matches WAL file prefix)
- `wal_dir`: directory containing `.wal` segment files
- `--from-seq`: start at this sequence number (default: 0)
- `--json`: emit one JSON object per line instead of text

### dump

Dump a single WAL segment file.

```
rsx-cli dump <file>
```

---

## Record Types Decoded (14 total)

| Record Type       | Key Fields Printed                              |
|-------------------|-------------------------------------------------|
| FILL              | symbol_id, user_id, side, qty, price, oid       |
| BBO               | symbol_id, bid_px, bid_qty, ask_px, ask_qty     |
| ORDER_INSERTED    | symbol_id, user_id, side, qty, price, oid       |
| ORDER_CANCELLED   | symbol_id, user_id, oid                         |
| ORDER_DONE        | symbol_id, user_id, oid                         |
| ORDER_ACCEPTED    | symbol_id, user_id, oid, cid                    |
| ORDER_FAILED      | symbol_id, user_id, oid, reason                 |
| ORDER_REQUEST     | symbol_id, user_id, side, qty, price, oid       |
| ORDER_RESPONSE    | symbol_id, user_id, oid, status                 |
| CANCEL_REQUEST    | symbol_id, user_id, oid                         |
| CONFIG_APPLIED    | symbol_id                                       |
| CAUGHT_UP         | symbol_id, live_seq                             |
| MARK_PRICE        | symbol_id, price                                |
| LIQUIDATION       | symbol_id, user_id, status, side, round, qty,   |
|                   | price, slip_bps                                 |

---

## Output Formats

### Text (default)

One line per record:

```
seq=1042 type=FILL len=128 crc=0xabcd12 symbol_id=1 user_id=5 \
  side=BUY qty=100 price=50000000
```

Header fields always present: `seq`, `type`, `len`, `crc`.
Record-specific fields follow on the same line.

### JSON (`--json`)

One JSON object per line (NDJSON). Fields match text output names.

```json
{"seq":1042,"type":"FILL","len":128,"crc":2882396704,
 "symbol_id":1,"user_id":5,"side":"BUY","qty":100,"price":50000000}
```

UUIDs (oid) rendered as hex strings via `oid_hex()`.

---

## Proposed Improvements

### Filtering

```
--type <TYPE>       only emit records of this type (e.g. FILL)
--symbol <id>       filter by symbol_id
--user <id>         filter by user_id
--from-ts <ns>      skip records with ts_ns < value
--to-ts <ns>        stop after ts_ns > value
```

Multiple `--type` flags should be OR'd together.

### Stats Mode

```
--stats
```

Instead of printing records, print a count per record type:

```
FILL            12340
ORDER_INSERTED   8210
ORDER_CANCELLED   430
...
total           21000
```

### Tail / Follow Mode

```
--follow
```

After exhausting existing records, poll the WAL directory for new
segment files and continue streaming. Exit on SIGINT.

### Human-Readable Prices

```
--tick-size <f64>   divide raw i64 prices by this for display
--lot-size <f64>    divide raw i64 quantities by this for display
```

Example: `--tick-size 0.1 --lot-size 0.001` converts raw i64 fields
to human decimals. Only affects text output; JSON always emits raw.

---

## Implementation Plan

1. Add filter structs parsed from clap args; apply before printing.
2. Add `--stats` flag; accumulate counts, print on exit.
3. Add `--follow` flag; after last segment, `inotify`/poll loop.
4. Add `--tick-size` / `--lot-size` for display conversion.

All changes are additive. No WAL format changes required.

---

## Acceptance Criteria

- `wal-dump` with no filters prints all records in seq order
- `--type FILL --symbol 1` prints only FILL records for symbol 1
- `--stats` prints per-type counts, total, exits 0
- `--follow` does not exit after last record; streams new ones
- `--json` output is valid NDJSON parseable by `jq`
- All 14 record types decode without panic on valid WAL data
- `--from-seq 0` and absent `--from-seq` behave identically
