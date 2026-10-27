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
- `md.rs` -- `md snapshot` and `md follow`: a tokio WebSocket client of the
  marketdata feed, `rsx_marketdata::wire::decode`, and the JSON and text
  rendering of a decoded frame.
- `token.rs` -- `token mint`: `rsx_messages::mint_jwt` over
  `Claims::for_user`; the one token mint tools and tests call.
- `gateway_client.rs` -- `Connection`: an authenticated order-channel
  WebSocket that sends `rsx_messages::WsFrame`s, parses what it reads with
  `rsx_messages::parse` and echoes every gateway heartbeat.
- `symbols.rs` -- the gateway's `/v1/symbols` catalog, the decimal to raw unit
  conversion, and the minimum notional from risk's `symbol_config_schedule`.
- `outcome.rs` -- `PlaceTracker` and `CancelTracker`: the frames that answer
  one order or cancel folded into a `Verdict`, its `FailureReason` and
  `CANCEL_REASON_*` names, and the JSON and text rendering.
- `order.rs` -- `order place`, `cancel` and `batch`: input to raw `N` frames,
  `CidMint` cids, one connection per user.
- `scenario.rs` -- the scenario table (symbol count, replication, gateways,
  stress load) and `SYMBOL_ORDER`, the names scenarios take symbols by.
- `catalog.rs` -- the symbols of a cluster from Postgres: applies risk's
  migrations, reads `symbol_static` and the active `symbol_config_schedule`
  row; `redact` for any url it prints.
- `plan.rs` -- `build`: the pure scenario + symbols + options to `Plan`, the
  env contract of every daemon, the maker and the auth sidecar. Pinned
  against `tests/golden/cluster/*.json`, one file per scenario.
- `procs.rs` -- spawn (detached, own process group, output appended to
  `log/<name>.log`), `Record` (pid + kernel start time), `stop`
  (SIGTERM, grace, SIGKILL; only a record whose start time still matches),
  the `Store` under `tmp/cluster/`, port and binary checks.
- `probe.rs` -- `GET /ready`, `/health` and `/metrics` against a daemon's
  health server (`rsx-health`).
- `cluster.rs` -- `up`, `down`, `ps`, `restart` over the above: prepare
  (skip the running, refuse a missing binary, and a busy port on a cold
  start, certs, WAL dirs), spawn the daemons, wait for readiness, then the
  sidecars.
- `reset.rs` -- `reset`: `down`, then truncate risk's Postgres state, then
  delete the WAL, the replay tips and nothing else under `tmp/`.
- `runtime_utils.rs` -- `block_on`, the current-thread runtime the async
  commands run on.
- `tail.rs` -- `Tail`: the live reader behind `wal dump --follow`.
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

`--follow` keeps the segment being written open (`tail.rs`, `Tail`) and polls
it every 100 ms: a poll reads only the bytes appended since, and checks the
CRC of each record it reads. A record cut short at the end of the live
segment, or a bad one with nothing after it, is waited out. When the writer
rotates (the open file is renamed and a new active file appears) the old
segment is drained and the next one opened. Records below the next sequence
number are skipped, so each record is printed once; `from_seq` is the first
one printed. A bad record that has data after it, or a partial or bad record
in a segment the writer has left, ends the follow with exit 1, as in the
dump.

### wal stats

The same reader and filters as `wal dump`, printing a count per record type
and a total.

### wal types

Prints `RECORD_NAMES`, the one list behind `--type`, the JSON `type` field and
the round-trip test in `record_test.rs`.

### wal ls

Lists the numeric subdirectories of a WAL directory (or the `-s` streams).
Per stream: segment count and bytes (with each segment's name, size,
modification time and whether it is the active one), first and last sequence
number and the last `ts_ns`. The first record comes from the first segment that has an
intact one, the last from a scan of the last such segment; `--no-seq` skips
both reads. A segment renamed or deleted between the directory listing and the
stat is left out.

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
| `wal ls` | `{"wal_dir", "streams": [{"stream_id", "files", "bytes", "segments": [{"name", "bytes", "modified_ms", "is_active"}], "first_seq", "last_seq", "last_ts_ns"}]}` |
| `wal verify` | `{"status": "ok"\|"corrupt", "streams": [{"stream_id", "records", "first_seq", "last_seq", "crc_errors", "seq_gaps": [[from, to]], "torn_tail", "error"}]}` |
| `wal stats` | `{"stream_id", "records", "by_type": {"FILL": n, ...}}` |
| `wal types` | `{"types": ["FILL", "BBO", ...]}` |
| `md snapshot` | `{"symbol_id", "bbo": {"bid_px", "bid_qty", "ask_px", "ask_qty", "seq", "ts_ns"}\|null, "bids": [[px, qty]], "asks": [[px, qty]]}`; levels best first, `bbo` from the best level of each side (0 for an empty side), `null` for an empty book |
| `md follow` | JSON lines, `kind` one of `bbo`, `l2_snapshot`, `l2_delta`, `trade`, `heartbeat`: `bbo` has `symbol_id`, `bid_px`, `bid_qty`, `bid_count`, `ask_px`, `ask_qty`, `ask_count`, `ts_ns`, `seq`; `l2_snapshot` has `symbol_id`, `bids` and `asks` as `[px, qty, count]`, `ts_ns`, `seq`; `l2_delta` has `symbol_id`, `side` (0 bid, 1 ask), `px`, `qty` (0 removes the level), `count`, `ts_ns`, `seq`; `trade` has `symbol_id`, `px`, `qty`, `taker_side` (0 buy, 1 sell), `ts_ns`, `seq`; `heartbeat` has `ts_ms` |
| `token mint` | `{"token", "user_id", "exp"}`; text mode prints the bare token |
| `order place` | `{"cid", "status", "reason_code", "reason", "reply", "order_id", "fills": [{"px", "qty", "ts_ns"}], "frames": [...], "rtt_us"}`; `status` is `filled`, `partial`, `resting`, `cancelled`, `rejected` or `timeout`; `reply` is `"U"`, `"E"` or `null` (local timeout); `frames` are the order's gateway frames as received, heartbeats excluded |
| `order cancel` | the `order place` keys without `fills`; `status` is `cancelled`, `rejected` or `timeout`; `cid` is `null` for an order id |
| `order batch` | JSON lines, one `order place` object per input line, in input order |
| `cluster plan` | `{"scenario", "processes": [{"name", "binary", "args", "cwd", "env": {}, "ports": []}]}`; `binary` and `cwd` are relative to the root; `ports` are the bare `host:port` ports (1024 and up) in `env` |
| `cluster up`, `restart`, `down` | `{"started": [], "stopped": [], "skipped": [{"name", "reason"}]}`; `up` and `restart` add `waited_ms` |
| `cluster ps` | `{"processes": [{"name", "pid", "state": "running"\|"exited"\|"absent", "started_ms", "log"}]}`; `pid` and `started_ms` are `null` for `absent` |
| `cluster scenarios` | `{"scenarios": [{"name", "symbols", "replication": "none"\|"replica"\|"spare", "gateways", "load": {"rate", "duration", "type"}\|null}]}` |

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
