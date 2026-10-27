# rsx-cli

CLI tools for RSX exchange operations.

## Commands

All WAL commands live under `wal`.

### wal dump

Dump the records of a stream.

```
cargo run -p rsx-cli -- wal dump <stream_id> <wal_dir> [from_seq] [flags]
```

`from_seq` is the first sequence number printed; earlier records in the
same segment are skipped.

Flags:
- `--json` -- emit JSON lines instead of human text
- `--type <NAME>` -- filter by record type (repeatable, OR logic):
  `FILL`, `BBO`, `ORDER_INSERTED`, `ORDER_CANCELLED`, `ORDER_DONE`,
  `CONFIG_APPLIED`, `CAUGHT_UP`, `ORDER_ACCEPTED`, `MARK_PRICE`,
  `ORDER_REQUEST`, `ORDER_RESPONSE`, `CANCEL_REQUEST`,
  `ORDER_FAILED`, `LIQUIDATION`
- `--symbol <U32>` / `--user <U32>` -- filter by symbol or user
- `--from-ts <NS>` / `--to-ts <NS>` -- keep records with ts_ns in the window
- `--follow` -- after the last record keep polling for new ones, printing
  each record once (Ctrl-C to exit)
- `--tick-size <F64>` / `--lot-size <F64>` -- divide raw i64
  prices/quantities for text display (JSON always raw)

Records go to stdout; `total: N records` goes to stderr. A JSON record
carries `seq`, `type`, `ts_ns`, `len`, `crc32` and the record's own fields;
`ORDER_CANCELLED` adds `reason_text`.

### wal stats

Per-type record counts and a total, for the records that pass the filters.

```
cargo run -p rsx-cli -- wal stats <stream_id> <wal_dir> [from_seq] [filter flags]
```

Takes the same `--type`, `--symbol`, `--user`, `--from-ts` and `--to-ts`
flags as `wal dump`. `--json` prints one object:
`{"stream_id", "records", "by_type": {"FILL": n, ...}}`.

### wal types

The record type names `--type` accepts and the JSON `type` field carries.

```
cargo run -p rsx-cli -- wal types [--json]
```

`--json` prints `{"types": ["FILL", "BBO", ...]}`.

### wal ls

One row per stream directory under a WAL directory: segment count, bytes,
first and last sequence number, last `ts_ns`.

```
cargo run -p rsx-cli -- wal ls <wal_dir> [-s <stream_id>]... [--json] [--no-seq]
```

`--no-seq` reads no record, so the sequence numbers and `last_ts_ns` are
absent (`null`); the segment sizes and times stay. Finding the last record
reads the last segment whole, which `--no-seq` avoids.

`--json` prints `{"wal_dir", "streams": [{"stream_id", "files", "bytes",
"segments": [{"name", "bytes", "modified_ms", "is_active"}], "first_seq",
"last_seq", "last_ts_ns"}]}`.

### wal verify

Checks the CRC of every record of every segment, the sequence continuity
across segments, and the tail of the last segment.

```
cargo run -p rsx-cli -- wal verify <wal_dir> [-s <stream_id>]... [--json]
```

`--json` prints `{"status": "ok"|"corrupt", "streams": [{"stream_id",
"records", "first_seq", "last_seq", "crc_errors", "seq_gaps": [[from, to]],
"torn_tail", "error"}]}`. A stream with a CRC error, a sequence gap or an
`error` is corrupt; a torn tail alone is not. The report is printed either
way; a corrupt WAL also exits 1.

### wal file

Dump a single raw WAL file as JSON lines (no stream reader required). It
stops at the first partial record and does not check CRCs.

```
cargo run -p rsx-cli -- wal file <file>
```

### md snapshot, md follow

Read the public market-data feed over WebSocket and decode it with
`rsx_marketdata::wire::decode`.

```
cargo run -p rsx-cli -- md snapshot <symbol_id> [-u <ws_url>] [-t <timeout_ms>] [--json]
cargo run -p rsx-cli -- md follow <symbol_id>... [-u <ws_url>] [-c <channels>] [-n <count>] [--json]
```

The url comes from `-u`, then `MARKETDATA_WS`, then `ws://127.0.0.1:8180`.
`snapshot` subscribes to depth and prints the first snapshot for the symbol;
an empty book is a snapshot with empty sides, and no answer within the
timeout (default 2000 ms) exits 1. `follow` subscribes to every symbol with
the channel bitmask (1 BBO, 2 depth, 4 trades; default 7), prints one line
per frame, heartbeats included, and sends the heartbeat marketdata requires.
It stops after `-n` frames or on SIGINT or SIGTERM (exit 0); a closed
connection or an undecodable frame exits 1.

### token mint

Prints a gateway JWT for a user, signed with `rsx_messages::mint_jwt`.

```
RSX_GW_JWT_SECRET=<secret> cargo run -p rsx-cli -- token mint -u <user_id> [-t <ttl_secs>] [-s <secret>] [--json]
```

The secret comes from `-s`, then `RSX_GW_JWT_SECRET`; none exits 2. The token
expires `-t` seconds from now (default 3600) and carries `aud` `rsx-gateway`,
`iss` `rsx-auth`, `user_id` and `sub`. Text mode prints the bare token;
`--json` prints `{"token", "user_id", "exp"}`.

### order place, order cancel, order batch

Send orders over the gateway WebSocket (`specs/2/49-webproto.md`) as a user
and print how each one ended. The token is minted with `rsx_messages::mint_jwt`.

```
cargo run -p rsx-cli -- order place -u <user_id> -s <symbol_id> --side buy|sell \
    (-p <price> | --px <ticks>) (-q <qty> | --qty-raw <lots> | --min-size) \
    [--tif gtc|ioc|fok] [--reduce-only] [--post-only] [--cid <cid>] [--json]
cargo run -p rsx-cli -- order cancel -u <user_id> <cid|oid> [--json]
cargo run -p rsx-cli -- order batch [<file>] [--json]
```

All three take `-g <ws_url>` (then `GATEWAY_URL`, then `ws://127.0.0.1:8088`),
`--secret` (then `RSX_GW_JWT_SECRET`; none exits 2) and `-t <timeout_ms>`
(default 5000, per order).

`-p` and `-q` are human decimals converted with the symbol's decimals from the
gateway's `/v1/symbols`; a value with more decimals than the symbol has exits
2. `--px` and `--qty-raw` are raw ticks and lots and are sent as given, so the
gateway answers a misaligned one (`E[1008]`, `E[1009]`). `--min-size` sends
the smallest lot multiple whose `price * qty` reaches the symbol's minimum
notional: `--min-notional`, else the active `symbol_config_schedule` row at
`--db` (`DATABASE_URL`), else 1 USD from the decimals. Without `--cid` the
order gets a fresh id from `rsx_types::client_order::CidMint`: a random
session part drawn per run plus a counter, so runs never share ids. A given
`--cid` is 1 to 20 characters of `0-9 A-Z a-z - _`. To retry an unanswered
order, resend it with the same `--cid`.

`place` ends on the first of: `U` status 0 (`filled`), 1 (`resting`, or
`partial` after fills), 2 (`cancelled`, or `partial` after fills), 3
(`rejected`, or `timeout` for reason TIMEOUT), an `E` frame (`rejected`), or
the timeout (`timeout`, no `reply`). A `timeout` means the order's fate is
unknown: it may still rest. `reason` is the `FailureReason` name
(`specs/2/18-messages.md`) of a status-3 `U`, the `CANCEL_REASON_*` name of a
status-2 `U` that came from an ORDER_CANCELLED record, or the message of an
`E`, whose code is `reason_code`. `U` frames carry no cid, so the first `U` or
`F` after the send names the order; use a user with no other activity.
Heartbeats from the gateway are answered with one of the client's own.

`cancel` takes a 32-character order id or a client order id and ends on the
order's status-2 `U` (`cancelled`), a status-3 `U` or an `E` (for example
`E[1005]` order not found), or the timeout.

`batch` reads JSON lines from a file or standard input, validates all of them
before sending any, then sends the orders one at a time in input order, one
connection per user, and prints one outcome per order as it ends. A line has
`user_id`, `symbol_id`, `side` (`buy`|`sell`), `price` (human decimal, string
or number) or `px`, `qty` (human), `qty_raw` or `min_size` (true), and
optionally `tif`, `reduce_only`, `post_only`, `cid`. The gateway limits a user
to 10 new orders a second (`E[1006]`).

Exit 0 when every order is `filled`, `partial`, `resting` or `cancelled`; exit
1 after the output when any is `rejected` or `timeout`, or when the gateway
refuses the token (HTTP 401) or the connection closes; exit 2 for bad flags,
an unrepresentable value, a bad cid, a symbol the gateway does not serve or a
bad batch line.

## Exit codes

| Code | Meaning                                       |
|------|-----------------------------------------------|
| 0    | success                                       |
| 1    | runtime error (I/O, WAL corruption)           |
| 2    | CLI misuse (unknown record type, bad flags)   |

A failure prints `Error: <message>` on stderr, or one `{"error": "<message>"}`
line under `--json`.

## Building

```
cargo build -p rsx-cli
```

## Dependencies

- `clap` -- CLI argument parsing
- `ctrlc` -- SIGINT handling for `--follow`
- `serde_json` -- JSON output
- `rsx-cast` -- WAL reader, header decoding
- `rsx-messages` -- record type constants and payload structs, `mint_jwt`
- `rsx-marketdata` -- `wire::decode` for the market-data feed
- `tokio-tungstenite` -- WebSocket client for `md`

## How to read this crate

- **What** — this README: commands, exit codes, examples.
- **How** — [ARCHITECTURE.md](ARCHITECTURE.md): command
  internals and the bench-probe binary.
- **Why** — [`notes/`](notes/): design rationale for the
  one non-obvious data structure choice (priority queue in
  the bench probe).

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) -- command internals
