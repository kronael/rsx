# rsx-cli

The command-line tool of the exchange. It inspects WAL files, checks the
exchange invariants, starts and stops the processes, places orders, drives
load, injects faults, and reads and schedules the symbol config and the
accounts. Every command prints JSON under `--json`. The playground, the demo
scripts and the tests call `rsx-cli` and render what it prints; they hold no
exchange logic of their own. Rules for editing the crate: [CLAUDE.md](CLAUDE.md).

## Commands

| Command | What it does | Needs |
|---|---|---|
| `wal ls\|verify\|dump\|stats\|types\|file` | read and check WAL files | WAL directory |
| `verify` | check the exchange invariants | WAL directories, Postgres for some checks |
| `md snapshot\|follow` | read the public market-data feed | marketdata |
| `token mint` | print a gateway JWT | JWT secret |
| `order place\|cancel\|batch` | send orders through the gateway | gateway, JWT secret |
| `stress` | drive load, a latency probe or the reject path | gateway, marketdata |
| `cluster scenarios\|plan\|up\|down\|ps\|restart\|reset` | start, stop and inspect the processes of a scenario | repository root, Postgres |
| `fault kill\|net\|wal-corrupt\|dup-order` | inject a fault | the cluster, `sudo tc`, or a WAL |
| `config show\|schedule` | read and extend the symbol config | Postgres |
| `account seed\|show` | seed and read accounts | Postgres |
| `risk overview` | read risk's persisted state | Postgres |

`rsx-cli --help` and `rsx-cli <command> --help` list every flag; the sections
below give the behaviour and the JSON each command prints, and name a flag by
its short form where it has one. The `cargo run` lines show the arguments; installed, the binary is `target/debug/rsx-cli` or
`target/release/rsx-cli`.

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
`ORDER_CANCELLED` adds `reason_text`, and `ORDER_DONE` adds `cancel_reason`
and `reason_text` (null unless the order was cancelled).

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
first and last sequence number, last `ts_ns`. A stream directory lists the
WAL writer's segments and the recorder's `{stream_id}_{YYYY-MM-DD}.wal` files;
the newest dated file is the active one. `wal verify`, `wal dump`, `wal stats`
and the ledger scan of `verify` read the dated files too; the book replay of
`verify` is the engine's own recovery and reads the WAL writer's segments only.

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

### verify

Checks the exchange invariants (root `CLAUDE.md` § Correctness invariants)
over the WAL directories and, with a database, over risk's tables.

```
cargo run -p rsx-cli -- verify <wal_dir>... [-s <stream_id>]... [--db <url>] [--json]
```

Each WAL directory holds numeric stream directories, as `wal ls` lists them;
`-s` limits the run to those streams, and a stream found under no directory
exits 1. `--db` is `DATABASE_URL`: without a url the checks that read
Postgres report `skip`, and an unreachable server fails "Postgres is
reachable" and skips them.

The report has 15 checks in this order. Invariant numbers are those of the
root list.

| Invariant | Check | Reads |
|---|---|---|
| none | WAL records are intact | WAL |
| none | Postgres is reachable | Postgres |
| 1 | Fills precede ORDER_DONE | WAL |
| 2 | Every order completes exactly once | WAL |
| 3 | Fills follow time priority within a price level | WAL |
| 3 | Replayed book matches the WAL | WAL, symbol config |
| 4 | Position equals the sum of fills | WAL, `positions` |
| 5 | WAL seqs strictly increase | WAL |
| 5 | Risk tips stay within the ME WAL | WAL, `tips` |
| 6 | BBO records are not crossed | WAL |
| 6 | Replayed book is not crossed | WAL, symbol config |
| 7 | SPSC rings preserve FIFO | nothing, always `skip` |
| 8 | Slab has no leak | WAL, symbol config |
| 9 | Funding sums to zero per symbol and interval | Postgres |
| 10 | At most one holder per shard lock | Postgres |

A check is `fail` on any violation, `skip` when it examined nothing (a check
that examined nothing proved nothing) and `pass` otherwise. The report
`status` is `fail` when any check failed, `pass` when one passed and none
failed, and `skip` otherwise. The replay checks rebuild each matching
engine's book from its WAL with the symbol config in Postgres.

`--json` prints `{"status": "pass"|"fail"|"skip", "checks": [{"invariant",
"name", "status", "detail", "checked", "violation_count", "violations"}]}`.
`invariant` is `null` for the two checks without a number. `violations` holds
at most 10 records, each naming its stream and the ids that locate it;
`violation_count` counts all. The report is printed either way. A
failed check also exits 1 with `verify failed: <n> of <total> checks` on
stderr. `wal verify` checks only the files: CRC, sequence continuity and the
tail.

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
status-2 `U`, or the message of an
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

### stress

Drives load at a gateway.

```
cargo run -p rsx-cli -- stress [-m load|probe|reject] [-g <ws_url>] [--md <ws_url>]
    [--cross-bps <bps>] [-s <ids>] [-u <users>] [-c <connections>] [-r <orders/s>] [-d <seconds>]
    [-n <orders>] [-w <warmup>] [-t <timeout_ms>] [--min-notional <raw>] [--db <url>]
    [-o <file>] [--dry-run] [--json]
```

- `load` (default) sends crossing IOC orders open-loop at `-r` orders per
  second for `-d` seconds over `-c` connections (default one per user), users
  `1..-u` (never 99, the maker). Order `i` of a connection is due at a fixed
  time; latency is measured from that time, not from when the send happened,
  so a client that falls behind its schedule shows the queueing (no
  coordinated omission). `send_lag_us` reports how late sends were.
- `probe` sends one crossing IOC order at a time on one connection to the
  first symbol, `-w` warmup orders discarded, then `-n` measured.
- `reject` sends the same closed loop for a symbol the gateway does not serve
  and measures the gateway's own reject (`E1007`).

Orders buy `--cross-bps` (default 100, 1%) above the best ask and sell that
far below the best bid, which come from marketdata (`--md`, default
`MARKETDATA_WS`), so a book that moves a little during the run still fills
them at the resting prices; each is sized to the symbol's
minimum order size at its price: `--min-notional`, else the symbol's active
row in `--db` (`DATABASE_URL`), else the minimum the gateway serves in
`/v1/symbols`, else 1 USD from its decimals. Every cid comes from one
`CidMint`, so no cid repeats within a run or across runs. Replies do not echo
the cid, so the first frame naming a new order id binds it to the oldest
unanswered order of its connection; all orders are IOC, so the terminal `U` is
an order's last frame. `--dry-run` prints the plan (best prices, quantities,
lots per order) and sends nothing.

`--json` prints the report (also written to `-o`); text mode prints a
summary. Exit 1 after printing when the run is not sound (an accounting that
does not close, a crossing order that did not fill, bad server timestamps,
fewer than 95% of orders answered); rejects are counted by reason name and do
not fail a load. SIGINT and SIGTERM stop sending, let in-flight orders finish
and report with `interrupted` true.

### cluster

The one launcher of the exchange processes: it builds the spawn plan of a
scenario, starts each process detached, records its pid, waits for it to be
ready and stops it by pid.

```
cargo run -p rsx-cli -- cluster scenarios [--json]
cargo run -p rsx-cli -- cluster plan [-s <scenario>] [--json]
cargo run -p rsx-cli -- cluster up [-s <scenario>] [-o <name>]... [-b] [-w <secs>] [--json]
cargo run -p rsx-cli -- cluster ps [--json]
cargo run -p rsx-cli -- cluster restart <name> [-s <scenario>] [-w <secs>] [--json]
cargo run -p rsx-cli -- cluster down [-o <name>]... [--json]
cargo run -p rsx-cli -- cluster reset [--db <url>] [--json]
```

Shared flags: `-C <root>` (`RSX_ROOT`, default `.`) is the repository root:
binaries are `<root>/target/{debug,release}/rsx-*`, daemons run with it as
their working directory, logs go to `<root>/log/<name>.log`, and pids to
`<root>/tmp/cluster/`. `--db` (`DATABASE_URL`) is risk's Postgres, `--secret`
(`RSX_GW_JWT_SECRET`) the gateway and sidecar JWT secret, `-r/--release` the
release binaries, `--pin-cores` each daemon's `RSX_<X>_CORE_ID`, `--no-maker`
leaves the maker out.

The symbols come from Postgres only: `symbol_static` joined to the active
`symbol_config_schedule` row. `plan` and `up` apply risk's migrations first (as
risk does at boot, which seeds a fresh database), then take the scenario's
symbols in the order PENGU, SOL, BTC, ETH, WIF, BONK, PEPE, DOGE. An empty
`symbol_static`, or a missing scenario symbol, exits 1 naming it. The scenario
names and shapes are in `scenario.rs` (`cluster scenarios` prints them).

`up` starts the matching engines, mark, risk (and its replicas), gateways,
marketdata and recorder, waits up to `-w` seconds (default 120) until every
daemon answers `GET /ready` with 200 (a warm risk replica: `/health`), then
starts the maker, which every scenario includes, and the auth sidecar when
`rsx-auth/.venv` exists and `RSX_AUTH_GITHUB_CLIENT_ID` and
`RSX_AUTH_GITHUB_CLIENT_SECRET` are set. Risk answers `/ready` only once it is
the live shard (the advisory lock held and warm catch-up done), so `up` returns
when the order path answers; `waited_ms` is the real wait, including the time a
previous risk session takes to release its lock. A process that exits, or one
still not ready after the wait, fails `up` with exit 1 naming it (and the state
the daemon reports); the processes stay running for inspection and `down`
stops them. A process already running is skipped, and a process running outside
the scenario asks for `down` first. A cold start (nothing of the cluster
running, no `--only`) fails before anything starts when something holds a port
a process binds (`plan` lists them as `ports`); a partial start leaves it to
the daemon, whose failed bind exits it during startup, since siblings hold some of the ports a process names. `--only` starts or stops just the named processes
(`maker`, `auth`, `risk-0`, ...); `-b` runs `cargo build` for the daemons
first. If the CLI or its parent is killed (SIGTERM, SIGINT or parent death)
while `up` waits, it stops what it started and exits 1; once `up` returns the
processes run on their own.

`down` stops, in reverse start order, only the processes it recorded: SIGTERM,
five seconds of grace, then SIGKILL. A record is trusted only while the pid
still has the kernel start time it was stored with, so a recycled pid is never
signalled. `ps` lists the processes of the last `up` and every record:
`running`, `exited` (the pid is gone) or `absent` (planned, never started).
`restart` stops one process and starts it again from the plan rebuilt for the
recorded scenario, then waits for it.

`reset` runs `down` for every recorded process, then truncates risk's
per-run Postgres state (`tips`, `positions`, `fills`, `funding`,
`funding_intervals`, `insurance_fund`, `liquidations`, `frozen_orders`;
`accounts`, `users` and the symbol tables stay) and deletes everything under
`<root>/tmp/wal` (the directory stays), `tmp/md.tip` and `tmp/recorder-tip-*`,
so the next `up` boots every component at seq 0 together. A tip left in
Postgres next to a deleted WAL makes risk resume past the new WAL's seqs.
Postgres is truncated first: when it is unreachable the command exits 1, the
processes are already stopped and the WAL is untouched. Processes the cluster
did not start are not signalled. `--db` is `DATABASE_URL`.

`--json` of `plan`: `{"scenario", "processes": [{"name", "binary", "args",
"cwd", "env", "ports"}]}`. `up`, `down`, `restart` and `reset` print
`{"started": [], "stopped": [], "skipped": [{"name", "reason"}]}`, with
`waited_ms` on `up` and `restart` and `cleared` (`risk-state`, `wal`, `tips`)
on `reset`. `ps` prints `{"scenario", "processes": [{"name", "pid", "state",
"started_ms", "log"}]}`; `scenario` is the one of the last `up` (`null`
before any), and `pid` and `started_ms` are `null` for `absent`. `scenarios`
prints `{"scenarios": [{"name", "symbols", "replication", "gateways", "load"}]}`.

### fault

Injects one fault into a running cluster or a WAL.

```
cargo run -p rsx-cli -- fault kill <name> [-C <root>] [--json]
cargo run -p rsx-cli -- fault net [-i <iface>] [--delay-ms <ms>] [--loss-pct <pct>] [--clear] [--json]
cargo run -p rsx-cli -- fault wal-corrupt <stream_id> [--wal-dir <dir>] [--offset <byte>] [-C <root>] [--json]
cargo run -p rsx-cli -- fault dup-order [-C <root>] [-r] [--dir <dir>] [--json]
```

- `kill` sends SIGKILL to a process that `cluster up` recorded under
  `<root>/tmp/cluster/pids`, and only to that process: the record must still
  match the pid's kernel start time. A name without a record, or a process
  that already exited, exits 1 and signals nothing. The record stays, so
  `cluster ps` shows the process as `exited` (a crash) until something starts
  it again: `cluster restart <name>`, or the playground watcher. `--json`
  prints `{"name", "pid", "signal"}`.
- `net` runs `sudo -n tc qdisc add dev <iface> root netem delay <ms>ms loss
  <pct>%` (defaults: `lo`, 50 ms, 10%); `--clear` runs `tc qdisc del dev
  <iface> root`. It shapes the whole interface for every process on the host.
  `add` fails when the interface already has a root discipline and `--clear`
  fails when it has none; both print `tc`'s message and exit 1. Bad
  interface names, a loss outside 0 to 100 and a fault with neither delay nor
  loss exit 2 before `tc` runs. `--json` prints `{"iface", "delay_ms",
  "loss_pct", "active"}`, with `null` delay and loss and `active` false for
  `--clear`.
- `wal-corrupt` flips every bit of one byte (default offset 24, the 9th
  payload byte of the first record) in the newest segment of a stream, so
  `wal verify` counts one CRC error and reports the stream `corrupt`. The
  directory is `--wal-dir`, else `<root>/tmp/wal`; a matching engine writes
  under `tmp/wal/<symbol>` (for example `--wal-dir tmp/wal/pengu`). An offset
  that is not inside the payload of a complete record exits 1 and changes
  nothing. A running daemon notices on its next read of the segment, a
  restart or a replay. `--json` prints `{"stream_id", "file", "offset",
  "old_byte", "new_byte"}`.
- `dup-order` starts a scratch `rsx-matching` (`<root>/target/{debug,release}/`)
  on its own ports, symbol 999 and WAL directory (`--dir`, default
  `<root>/tmp/me-demo`, reused across runs), with nothing inherited from the
  environment. It casts the same `OrderMessage` twice on a `CastSender`, the
  way risk does, waits for the engine's `rejects_duplicate` counter, stops it
  and prints the WAL records of that order: one ORDER_ACCEPTED, then one
  ORDER_FAILED with reason 3 (duplicate order id). Any other outcome exits
  1. The live engines are never touched; the gateway mints a fresh order id
  per order, so only a direct cast can repeat one. The scratch engine dies
  with the CLI (SIGTERM, SIGINT or the kernel's parent-death signal).
  `--json` prints `{"sent": 2, "order_id", "records": [...]}`, the records as
  `wal dump --json` prints them.

### config show, config schedule

Reads and extends the symbol config in risk's Postgres (`symbol_static`,
`symbol_config_schedule`, `symbol_config_applied`). The url is `--db` or
`DATABASE_URL` (default `postgres://rsx:rsx@127.0.0.1:5432/rsx`); a value
Postgres cannot parse exits 2 before any connection, an unreachable server
exits 1, and both messages leave the password out. Every command applies
risk's migrations first, as the daemons do at boot.

```
cargo run -p rsx-cli -- config show [-s <symbol>] [--db <url>] [--json]
cargo run -p rsx-cli -- config schedule -s <symbol> --set <column>=<value>... [--effective-at-ms <ms>] [--db <url>] [--json]
```

- `show` lists every symbol with all its scheduled versions and the row the
  matching engine applied last (`null` before it applied one). `--json`
  prints `{"symbols": [{"symbol_id", "name", "active_version", "scheduled":
  [row], "applied": row|null}]}`; a row has the columns of its table, and
  `active_version` is the highest version already effective, the one risk
  runs on. `-s` of a symbol with no config exits 1.
- `schedule` inserts the next version of a symbol: a copy of its highest
  version with the `--set` columns changed, effective at `--effective-at-ms`
  (default now). No existing row is updated. Risk and the matching engine
  apply the row once it is effective; the engine polls every
  `RSX_ME_CONFIG_POLL_S` seconds (default 10) and Risk follows its
  `CONFIG_APPLIED` record. `--set` takes `tick_size`, `lot_size`, `min_notional`,
  `max_order_qty`, `maker_fee_bps`, `taker_fee_bps`,
  `initial_margin_rate_bps`, `maintenance_margin_rate_bps`, `max_leverage`,
  `funding_interval_sec`, `funding_rate_min_bps` and `funding_rate_max_bps`;
  decimals and status stay as they are. Quantities must be positive (fees
  and funding bounds may be negative), maintenance margin must be below
  initial margin, the row must read back as a valid risk config, and
  `--effective-at-ms` is at or after the highest version's (a version dated
  before it would never apply). A rejected change exits 2 and writes
  nothing; a symbol with no config row, or a writer that took the version
  first, exits 1. `--json` prints
  `{"symbol_id", "config_version", "effective_at_ms"}`.

### account seed, account show, risk overview

Seeds and reads the accounts risk persists. Same `--db` and `DATABASE_URL`
as `config`.

```
cargo run -p rsx-cli -- account seed [-c <count>] [--collateral <raw>] [--db <url>] [--json]
cargo run -p rsx-cli -- account show -u <user> [--mark <symbol>=<px>]... [--db <url>] [--json]
cargo run -p rsx-cli -- risk overview [--mark <symbol>=<px>]... [--db <url>] [--json]
```

- `seed` upserts an account for users 1 to `count` (default 1001) with
  `collateral` raw units (default 1e17): an existing account gets the
  collateral and keeps its version. It also deletes those users' frozen
  orders, so a crashed run leaves no reservation that rejects new orders for
  margin. Run it before risk starts: risk reads `frozen_orders` once at cold
  start, so a seed under a live risk leaves its memory ahead of Postgres.
  `--json` prints `{"accounts", "collateral"}`.
- `show` prints one user's collateral, open positions (net quantity not
  zero) and frozen orders; a user without an `accounts` row exits 1. `--json`
  prints `{"user_id", "collateral", "positions": [{"symbol_id", "net_qty",
  "avg_px", "upnl"}], "frozen": [{"order_id", "symbol_id", "amount"}]}`.
  Risk persists no mark price, so `upnl` is `null` unless `--mark` gives the
  symbol's raw price.
- `overview` prints every account in the shape of `show`, the insurance fund
  and the last 50 liquidations: `{"users": [account], "insurance": {"funds":
  [{"symbol_id", "balance", "version"}], "total"}, "liquidations": [row]}`,
  a liquidation row having the columns of `liquidations`, newest first.

## Exit codes

| Code | Meaning                                       |
|------|-----------------------------------------------|
| 0    | success                                       |
| 1    | runtime error (I/O, WAL corruption, a failed check or order, an unreachable service) |
| 2    | CLI misuse (unknown record type, bad flags, a value that cannot be represented) |

A failure prints `Error: <message>` on stderr, or one `{"error": "<message>"}`
line under `--json`.

## Building

```
cargo build -p rsx-cli
```

The crate builds a second binary, `bench-match-rt` (`--n <orders>`,
`--warmup <orders>`): one matching round trip in process, timed per stage.
It is a benchmark, not an operator tool.

## Dependencies

- `clap` -- CLI argument parsing
- `ctrlc` -- SIGINT handling for `wal dump --follow`
- `serde_json`, `serde` -- JSON output
- `tokio`, `futures-util` -- the current-thread runtime and stream helpers of the async commands
- `tokio-tungstenite` -- WebSocket client for `md`, `order` and `stress`
- `tokio-postgres` -- the Postgres client of `verify`, `cluster`, `config`, `account` and `risk`
- `libc` -- `kill(2)` and the parent-death signal for `cluster` and `fault`
- `rustc-hash` -- integer-keyed maps in `verify`
- `rsx-cast` -- WAL reader, header decoding, `CastSender` for `fault dup-order`
- `rsx-messages` -- record type constants and payload structs, the gateway frames, `mint_jwt`
- `rsx-types` -- `FailureReason`, `CidMint`
- `rsx-book` -- the orderbook `verify` replays into
- `rsx-marketdata` -- `wire::decode` for the market-data feed
- `rsx-matching` -- recovery and config loading for `verify`, `failure_reason_code` for `fault dup-order`
- `rsx-risk` -- migrations, the config writer and readers behind `cluster`, `verify`, `order`, `config`, `account` and `risk`
- `rsx-gateway` -- dev-dependency: the round-trip test of `token mint`

## How to read this crate

- **What** — this README: commands, exit codes, examples.
- **How** — [ARCHITECTURE.md](ARCHITECTURE.md): modules, the JSON contracts,
  command internals.
- **Rules** — [CLAUDE.md](CLAUDE.md): what an editor must not break.
- **Why** — [`notes/`](notes/): design rationale for the Parquet query tool
  that pairs with `wal file`.
