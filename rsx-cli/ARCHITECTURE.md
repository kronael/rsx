# rsx-cli Architecture

How `rsx-cli` is built. What each command does and its flags: [README.md](README.md).
Rules for editing the crate: [CLAUDE.md](CLAUDE.md).

## Layout

`main.rs` holds the clap tree and the dispatch: one `run_<group>` function
per command group turns parsed flags into a library call and returns its
`Outcome`; `main` prints the failure, if any, and sets the exit code. No
command logic lives there. The library (`lib.rs`) is flat, one module per
concern:

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
- `stress.rs` -- `stress --mode load|probe|reject`: the order plan from
  gateway, marketdata and (optionally) Postgres, one `Run` per connection
  (open-loop schedule or one order at a time), SIGINT and SIGTERM handling.
- `stress_tally.rs` -- `Tracker`: binds replies to orders by arrival order,
  measures from the intended send time, folds the five server clocks into
  legs; `Tally` and `Samples` (exact sorted samples, nearest-rank percentiles).
- `stress_report.rs` -- the report object and the correctness checks that
  make a run fail.
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
- `fault.rs` -- `kill` (SIGKILL of a recorded pid, through `procs::kill`),
  `shape_net` (`sudo -n tc qdisc add|del`, arguments built by `tc_args`) and
  `corrupt_wal` (one byte of the newest segment of a stream, checked against
  the record boundaries `SegmentReader` reports).
- `dup_order.rs` -- `duplicate_order`: the scratch matching engine (`Engine`,
  spawned with an empty environment and a parent-death signal), the two
  `CastSender::send_raw` calls, the wait on the `rejects_duplicate` counter,
  and the check of the WAL records of that order.
- `database.rs` -- `open`: the Postgres client of `config`, `account` and
  `risk`; `check_url` rejects an unparsable url as misuse before any
  connection, then it connects and applies risk's migrations.
- `config.rs` -- `config show` (`SymbolView`: the schedule and applied rows
  as `row_to_json`, so a row carries its table's columns) and `config
  schedule` over `rsx_risk::config::schedule_symbol_config`, which owns the
  insert and its validation.
- `account.rs` -- `account seed|show` and `risk overview`: `Account` (the
  `accounts`, open `positions` and `frozen_orders` of a user, rows decoded by
  `pg::position_from_row`), the unrealized PnL at a caller-given mark, the
  insurance fund and the last liquidations.
- `runtime_utils.rs` -- `block_on`, the current-thread runtime the async
  commands run on.
- `tail.rs` -- `Tail`: the live reader behind `wal dump --follow`.
- `stream_reader.rs` -- `StreamReader`: every record of a stream in file order
  across the segments `inspect::list_segments` names, the recorder's dated
  archive files included. The reader behind `wal dump`, `wal stats` and the
  ledger scan of `verify`; a torn tail of the last segment ends the stream, as
  in `WalReader`.
- `inspect.rs` -- `list_stream` and `verify_stream` over a stream directory,
  built on `SegmentReader` and `rsx-cast`'s `list_wal_files_across` plus the
  recorder's dated archive files, which `rsx-cast` does not list; the
  `StreamListing` and `StreamVerdict` reports and their JSON.
- `verify.rs` -- `verify`: finds the streams under the WAL directories, scans
  each into a `Ledger`, replays the streams that carry order flow, loads
  `Persisted` when a database is given, and assembles the 15 `Check`s.
- `check.rs` -- `Tally` (checked count plus at most `MAX_VIOLATIONS`
  offending records), `Check` (pass, fail or skip, and the check names), and
  `Report` with its JSON and text.
- `ledger.rs` -- `Ledger`: one pass over a stream's records that feeds the
  WAL-only invariants (fills before done, one completion per order, FIFO,
  increasing seqs, BBO not crossed) and keeps the fills the position check
  compares with Postgres.
- `replay.rs` -- rebuilds the book of a stream with `rsx_matching`'s recovery
  and judges it against the ledger: the book matches what rests, is not
  crossed, and its slab has no leak.
- `persisted.rs` -- `Persisted`: risk's `positions` and `tips` rows; the
  position and tip checks; the funding and lock tallies.
- `pg.rs` -- `Pg`: a Postgres connection with the current-thread runtime that
  drives it, every call blocking; the reads of `verify` (positions, tips,
  funding, locks, the symbol config for a replay).
- `bin/bench_match_rt.rs` -- the `bench-match-rt` benchmark binary.

## Commands

### Output and exit codes

Every command returns `Outcome = Result<(), Failure>`. `Failure::Runtime`
exits 1 and `Failure::Misuse` exits 2; `output::report` prints `Error:
<message>` on stderr, or one `{"error": "<message>"}` line when the command
ran with `--json`. A command that has a verdict to report, such as `verify`,
`wal verify` and `stress`, prints its report on stdout first and then
returns the failure, so a failed run still has its JSON.

### wal dump

Reads the stream through `StreamReader` over every WAL writer and recorder
archive segment. Iterates all records sequentially, printing sequence number,
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

### verify

`verify::run` does the work and `verify::verify` prints it. It lists the
numeric stream directories under each WAL directory, then for each stream:

1. `scan_stream` reads the stream into a `Ledger` with `StreamReader`. A read
   error is noted in the ledger, not raised, so the other streams still run.
2. A stream with order flow, and a connected database, is replayed:
   `Pg::load_boot_config` gives the symbol config, `replay::recover_book`
   runs `rsx_matching`'s recovery over the WAL, and `replay::judge` compares
   the book with the ledger. When the replay stops short of the ledger's last
   seq, the ledger is rescanned up to that seq so both sides cover the same
   records. A stream with no config row skips the replay checks with a
   reason; a failed replay is one violation.
3. With a database, `Pg::load_persisted` reads `positions` and `tips` once;
   the position check compares them with the ledger's fills, the tip check
   with each stream's last seq. `Pg::check_funding` and `check_locks` run
   their SQL.

`Check::from_tally` turns a tally into a verdict: any violation fails, no
record examined skips. `check_or_explain` gives a skipped replay check the
reasons the replay did not run. Invariant 7 is always `skip`: the ring is
in-process and nothing outside the process records its order.

### JSON contracts

`--json` prints one object on stdout (`wal dump` prints JSON lines). Keys are
pinned by `tests/json_contract_test.rs`.

| Command | Output |
|---|---|
| `wal ls` | `{"wal_dir", "streams": [{"stream_id", "files", "bytes", "segments": [{"name", "bytes", "modified_ms", "is_active"}], "first_seq", "last_seq", "last_ts_ns"}]}` |
| `wal verify` | `{"status": "ok"\|"corrupt", "streams": [{"stream_id", "records", "first_seq", "last_seq", "crc_errors", "seq_gaps": [[from, to]], "torn_tail", "error"}]}` |
| `wal stats` | `{"stream_id", "records", "by_type": {"FILL": n, ...}}` |
| `wal types` | `{"types": ["FILL", "BBO", ...]}` |
| `verify` | `{"status": "pass"\|"fail"\|"skip", "checks": [{"invariant", "name", "status", "detail", "checked", "violation_count", "violations": []}]}`; 15 checks in a fixed order, `invariant` `null` for the two without a number, at most 10 violations per check, each carrying its own keys |
| `md snapshot` | `{"symbol_id", "bbo": {"bid_px", "bid_qty", "ask_px", "ask_qty", "seq", "ts_ns"}\|null, "bids": [[px, qty]], "asks": [[px, qty]]}`; levels best first, `bbo` from the best level of each side (0 for an empty side), `null` for an empty book |
| `md follow` | JSON lines, `kind` one of `bbo`, `l2_snapshot`, `l2_delta`, `trade`, `heartbeat`: `bbo` has `symbol_id`, `bid_px`, `bid_qty`, `bid_count`, `ask_px`, `ask_qty`, `ask_count`, `ts_ns`, `seq`; `l2_snapshot` has `symbol_id`, `bids` and `asks` as `[px, qty, count]`, `ts_ns`, `seq`; `l2_delta` has `symbol_id`, `side` (0 bid, 1 ask), `px`, `qty` (0 removes the level), `count`, `ts_ns`, `seq`; `trade` has `symbol_id`, `px`, `qty`, `taker_side` (0 buy, 1 sell), `ts_ns`, `seq`; `heartbeat` has `ts_ms` |
| `token mint` | `{"token", "user_id", "exp"}`; text mode prints the bare token |
| `order place` | `{"cid", "status", "reason_code", "reason", "reply", "order_id", "fills": [{"px", "qty", "ts_ns"}], "frames": [...], "rtt_us"}`; `status` is `filled`, `partial`, `resting`, `cancelled`, `rejected` or `timeout`; `reply` is `"U"`, `"E"` or `null` (local timeout); `frames` are the order's gateway frames as received, heartbeats excluded |
| `order cancel` | the `order place` keys without `fills`; `status` is `cancelled`, `rejected` or `timeout`; `cid` is `null` for an order id |
| `order batch` | JSON lines, one `order place` object per input line, in input order |
| `stress` | `{"mode", "status": "passed"\|"failed", "failures": [], "interrupted", "config": {"gateway", "symbols", "users", "connections", "target_rate", "duration_s", "count", "warmup", "timeout_ms", "measures"}, "offered", "submitted", "send_errors", "accepted", "rejected", "timeouts", "errors", "completed", "fills", "filled_orders", "rejected_by_reason": {name: n}, "errors_by_reason", "timestamp_errors", "elapsed_ms", "latency_us", "send_lag_us", "server_latency_ns": {leg: summary}, "per_symbol": {id: {counters, "latency_us", "server_latency_ns"}}}`; a summary is `{"n", "min", "p50", "p95", "p99", "p99_9", "max"}` (null when `n` is 0); `measures` is `accepted` (load, probe) or `rejected` (reject) and says which orders `latency_us` samples; the legs are `internal`, `gateway_to_risk`, `risk_to_me`, `matching`, `match_done_to_gateway`; `scripts/latency-publish.sh` turns this into the `e2e_us` block `make perf-e2e-gate` reads. `--dry-run` prints `{"mode", "symbols": [{"symbol_id", "lot_size", "bid_px", "ask_px", "bid_qty", "ask_qty", "buy_px", "sell_px", "buy_qty", "sell_qty", "buy_lots", "sell_lots"}]}` |
| `cluster plan` | `{"scenario", "processes": [{"name", "binary", "args", "cwd", "env": {}, "ports": []}]}`; `binary` and `cwd` are relative to the root; `ports` are the ports the process binds, ascending; an address it only sends to is not listed |
| `cluster up`, `restart`, `down` | `{"started": [], "stopped": [], "skipped": [{"name", "reason"}]}`; `up` and `restart` add `waited_ms` |
| `cluster ps` | `{"scenario": name\|null, "processes": [{"name", "pid", "state": "running"\|"exited"\|"absent", "started_ms", "log"}]}`; `pid` and `started_ms` are `null` for `absent` |
| `cluster scenarios` | `{"scenarios": [{"name", "symbols", "replication": "none"\|"replica"\|"spare", "gateways", "load": {"rate", "duration", "type"}\|null}]}` |
| `fault kill` | `{"name", "pid", "signal"}`; `signal` is `"SIGKILL"` |
| `fault net` | `{"iface", "delay_ms", "loss_pct", "active"}`; `delay_ms` and `loss_pct` are `null` and `active` false after `--clear` |
| `fault wal-corrupt` | `{"stream_id", "file", "offset", "old_byte", "new_byte"}` |
| `fault dup-order` | `{"sent": 2, "order_id", "records": [...]}`; `records` are `wal dump --json` objects of the scratch stream (symbol 999) whose `oid` is `order_id` |
| `cluster reset` | the `cluster down` object plus `"cleared": ["risk-state", "wal", "tips"]` |
| `config show` | `{"symbols": [{"symbol_id", "name", "active_version", "scheduled": [row], "applied": row\|null}]}`; a row has the columns of its table |
| `config schedule` | `{"symbol_id", "config_version", "effective_at_ms"}` |
| `account seed` | `{"accounts", "collateral"}` |
| `account show` | `{"user_id", "collateral", "positions": [{"symbol_id", "net_qty", "avg_px", "upnl"}], "frozen": [{"order_id", "symbol_id", "amount"}]}`; `upnl` is `null` without a `--mark` for the symbol |
| `risk overview` | `{"users": [account], "insurance": {"funds": [{"symbol_id", "balance", "version"}], "total"}, "liquidations": [row]}` |

`first_seq`, `last_seq`, `last_ts_ns` and `error` are `null` when absent.
`records` in `wal verify` counts the records whose CRC matched. An error
under `--json` is one `{"error": "<message>"}` line on stderr.

### wal file

Parses the 16-byte headers of one raw file in memory and prints each record
as JSON. It stops at the first partial record and does not check CRCs.

## Architectural Decisions

**Runtime: tokio, current-thread, only where a command waits on a socket.**
`md`, `order`, `stress`, `cluster`, `reset`, `config`, `account`, `fault
dup-order` run on the current-thread runtime that
`runtime_utils::block_on` builds per call, and the Postgres reads of
`verify` on the one that `Pg` owns. `wal` and `fault
kill|net|wal-corrupt` are plain synchronous code. The CLI runs offline, not
on the GW→ME→GW critical path, so it has no hot loop and no latency budget.
See [`../docs/concepts/02-tiles-and-pinning.md`](../docs/concepts/02-tiles-and-pinning.md)
for where tiles or monoio apply instead.

**One crate for every operator tool.** The WAL decoder, the invariant
checks, the gateway client, the launcher and the fault injectors share the
record layouts of `rsx-messages` and the migrations of `rsx-risk`, so a
second copy in another language drifts from the wire. The playground and the
demo scripts call the binary and render its JSON.
