# 53.cli — exchange tooling into `rsx-cli`

Founder decision 2026-10-21 (PLAN.md § Founder decisions): the playground
reships nothing that exists elsewhere. Every tool useful for the exchange
lives in the Rust `rsx-cli` (or a Rust lib it uses). The playground is glue:
it calls `rsx-cli … --json` and renders. No PyO3, no Python CLI. The Go
maker stays `rsx-maker`; `rsx-playground/market_maker.py` is deleted.

Read-only audit at `badf1247`. Two read-only sub-agents surveyed
`server.py`/`pages.py` and the other modules, `rsx-cli`, the Rust libs and the
gates. These were re-checked by hand: `WAL_HDR` `server.py:1809`,
`parse_wal_records` `:1874`, `send_order_to_gateway` `:5830`, the JWT mints
`:5859, :8102, :8304`, `_run_invariant_checks` `:6288`, invariant 1 at
`:6355-6412`, `/api/orders/quick` `:6871` without an admin check,
`MAKER_SCRIPT` `:8798`, the snapshot magic `:2063`, `runtime.py:1-100`,
`signRsxJwt` `tests/test_helpers.ts:24`, the `rsx-risk` and `rsx-gateway`
dependencies, and the `bench-gate-e2e.sh` keys. Line numbers are per file at
that commit. Paths without a prefix are under `rsx-playground/`.

## 1. Inventory

Target column: the `rsx-cli` subcommand that owns the work. "stays" means
dashboard glue, with the reason given. LOC are approximate (from the survey).

### A. WAL (≈800 LOC in `server.py`)

| Playground | What it does | Target |
|---|---|---|
| `server.py:1809-1872` `WAL_HDR`, `*_FMT`, `RECORD_*`, `RECORD_TYPE_NAMES`, `CANCEL_REASON_TEXT` | copies of `rsx-messages/src/lib.rs` ids and layouts; omits types 9/10/11 | deleted; names come from `rsx-cli` JSON |
| `parse_wal_records` `:1874-2049` | Python decoder for 11 record types, **no CRC check** | `wal dump --json` (`--follow` for live views) |
| `_wal_stream_dirs` `:2050`, `scan_wal_streams` `:1711`, `scan_wal_files` `:1757` | stream and file listing, byte counts | `wal ls --json` |
| `_parse_snapshot_orders` + `_bbo_from_orders` + `_snap_to_bbo` `:2063-2186` | parses `snapshot.bin` with magic `0x5258534E` | **deleted.** That magic matches neither the Avro checkpoint (`Obj\x01`) nor the legacy one (`0x52535843`, `rsx-matching/src/checkpoint.rs:24-28`), so it reads nothing today. Checkpoint seq comes from ME `/metrics` (matching plan step 9) |
| `_latest_bbo_from_wal`, `parse_wal_bbo`, `parse_wal_fills*`, `parse_wal_mark_prices`, `parse_wal_liquidations` `:2187-2361` | latest-of / filter-by over decoded records | records from `wal dump --json`; selecting and rendering the newest rows stays |
| `parse_wal_book_stats` `:2272`; fills → `render_position_heatmap` / `render_margin_ladder` (`:4807-4816`) | positions and margin recomputed from WAL fills (risk logic in Python) | `account show --json` (risk's persisted positions, group H); the Python recomputation is deleted |
| `api_wal_verify` `:7591` | says "verified" after counting files; checks no CRC | `wal verify --json` |
| `api_wal_dump` `:7605-7696` | already shells out to `rsx-cli dump` (`:7647`) | `wal dump --json`; glue stays |
| `_collect_me_wal` + `api_matching_wal` `:4317-4369` | ME WAL newest-first | `wal dump --json --type …` |

### B. Invariants (≈590 LOC)

| Playground | What it does | Target |
|---|---|---|
| `_run_invariant_checks` `:6288-6750` | 10 checks (table below) | `verify --json` |
| `/api/verify/run*` `:6751`, `x_verify` `:5140`, `x_invariant_status` `:3953` | wrappers and the `verify_results` global | stays (renders `verify` output) |
| `x_reconciliation` `:4865-4938` | shadow book vs WAL BBO, book mid vs mark WAL | `verify` check "md book matches ME BBO" (needs group I's `md snapshot`) |

Today's checks against what `verify` does instead:

| # | Today (`server.py`) | `verify` |
|---|---|---|
| 1 Fills precede ORDER_DONE | `:6355-6412`: sorts the fill seqs, then counts inversions, so it **can never fail** | per order id over each WAL stream: no FILL (as taker or maker) follows that order's ORDER_DONE |
| 2 Exactly-one completion | counts the dashboard's own `recent_orders` | per ORDER_ACCEPTED: one ORDER_DONE, ORDER_FAILED or post-only ORDER_CANCELLED; resting is legal; a duplicate-reject ORDER_FAILED is not a completion; an order cut off at the live tail is not reported |
| 3 FIFO, 8 slab no-leak | hard-coded skip | FIFO: a FILL must hit the earliest ORDER_INSERTED still resting at its price (WAL only). Replay: `rsx_matching::recovery::recover` rebuilds the book from checkpoint or seq 1, and its resting orders must equal the WAL's open orders. Slab: `allocated = free + active` on the replayed book |
| 4 Position = sum of fills | SQL on `positions`/`fills` | each Postgres `positions` row equals `rsx_risk::Position::apply_fill` over the WAL fills up to the row's `last_fill_seq`; every position with a fill at or below the lowest `tips` row of its symbol has such a row |
| 5 Tips monotonic | WAL BBO seqs | every stream's seqs strictly increase, plus no risk `tips` row (`001_base_schema.sql:49`) passes the last seq of its ME stream |
| 6 No crossed book | `_book_snap` + WAL BBO | every two-sided BBO record has bid < ask, plus the replayed book's resting buys all below its resting sells |
| 7 SPSC FIFO | skip | `skip`: the ring is in-process and nothing outside the process records its order |
| 9 Funding zero-sum | PG `funding` | PG, per (symbol, interval) sum over the shards that all settled it is in `-(k-1) ..= 0` for `k` shards (0 for one shard; each shard floors its own payments); `funding_intervals` from migration 008 |
| 10 Advisory lock | duplicate pids from `tmp/pids` | `pg_locks`: at most one backend per shard lock key; skip when no lock is held |
| — WAL dir, processes, PG reachable | `:6293-6350` | `WAL records are intact` (CRC, gaps, short payloads) and `Postgres is reachable` are `verify` checks; process liveness comes from `cluster ps` |

### C. Token (≈75 LOC here, 9 more copies)

| Copy | Target |
|---|---|
| `_mint_user_token` `:5855-5872`; inline mints in the probes `:8096-8113`, `:8300-8316` | `token mint --json` |
| `stress_client.py:140-155, 489`, `market_maker.py:213`, `demo_scenario.py:204` | deleted with their modules, or call `token mint` (demos) |
| `tests/test_helpers.ts:24` `signRsxJwt` (about 20 specs), `tests/conftest.py:540`, `acceptance_test.py:256`, `stress_integration_test.py:41` | `token mint` via `execFileSync` / subprocess |
| `rsx-cli/src/bench_probe.rs:168-180`; `rsx-gateway` benches and tests (`ws_order_latency:118`, `jwt_validate_bench:40`, `gw_tail_bench:451`, `gateway_process_test:177`, `rest_contract:462`) | one `rsx_messages::mint_jwt` (D4; `validate_jwt` stays in `jwt.rs:33` and shares its `Claims`); all Rust callers use it |
| `_decode_user_token` `:1372-1391` | **stays**: the dashboard checks its own caller's JWT (auth glue, not tooling) |
| `rsx-maker/gateway.go:66`, `rsx-auth` jwt_util | **stay**: the Go maker stays (founder); rsx-auth is the issuer |

### D. Orders (≈835 LOC + `order_ids.py` 54)

| Playground | What it does | Target |
|---|---|---|
| `_send_ws_frame` `:5778-5829`, `send_order_to_gateway` `:5830`, `send_cancel_to_gateway` `:5844` | gateway WS client: one frame, wait 2 s for U/F/E | `order place` / `order cancel` |
| `_new_cid` `:5873`, `order_ids.py` | twin of `rsx_types::client_order::CidMint` | deleted; the CLI mints cids |
| `REJECT_REASONS` `:5897-5930`, `_classify_order_response` `:5931`, `FAILURE_REASON_NAMES` `:4083` | copies of `FailureReason` (`rsx-types/src/lib.rs:73`); `ORDER_TOO_SMALL` is spelled differently | the CLI reports `reason` by name |
| `_build_order_msg` `:5983-6120`; the pricing copy in `/api/orders/quick` `:6868-7061` | human → raw, tick/lot check, market sweep, overflow | `order place --price/--qty` (human, symbol config from PG) or `--px/--qty-raw`; `--market` takes the sweep price from the md BBO |
| `_execution_venue_error` `:5958` | refuses when gw-0/ME are down | `cluster ps` glue; stays as a UI precheck |
| `_finalize_order_submission` `:6121-6210` | HTML + `recent_orders` | stays (rendering, session state) |
| `/api/orders/batch` `:6820`, `/random` `:6842`, `/invalid` `:7498` | **fake**: append rows, never reach the gateway | `order batch` (stdin JSONL); `random`/`invalid` only generate params in the glue |
| `/api/gateway/duplicate-demo` `:4198-4253` | same cid twice | `order place --cid X` twice; glue stays |

### E. Stress and probe (≈690 LOC here + `stress.py` 512 + `stress_client.py` 800)

| Playground | What it does | Target |
|---|---|---|
| `stress.py`, `stress_client.py` | open-loop load, process pool, JWT, frames, percentiles, `stress-*.json` | `stress --json` |
| `do_stress_start` `:7075`, `/api/stress/*` `:7130-7285` | spawns `stress.py`, status | stress runs as a managed process (`cluster up --only stress` or the glue spawns `rsx-cli stress`); the routes stay |
| reports `:7286-7335`, `x_stress_reports_list` `:7415` | read `tmp/stress-reports/*.json` | stays (renders the CLI's report files) |
| `_run_latency_probe` `:8064-8269`, `_run_gw_only_probe` `:8299-8401` | GW→ME→GW fill probe; gateway-only reject probe | `stress --mode probe`, `stress --mode reject` |
| `_percentiles`, `api_latency` `:8030-8052` | percentiles over Python lists | stays only as the e2e latency demo card (browser→proxy ms); the exchange numbers come from `stress` |
| stage latency from logs `:8426-8532` | regex over `latency:` log lines | stays (log rendering); flagged in § 7 |
| `rsx-cli/src/bench_probe.rs` (518) | one-at-a-time IOC probe; fetches `/v1/symbols` and `/api/book` from the playground, which is circular | merged into `stress.rs`; the `bench-probe` bin goes; symbols come from PG, prices from `md snapshot` |
| `scripts/latency-publish.sh:316` (calls `stress.py`), `Makefile` `perf-load` | sustained-load publisher | calls `rsx-cli stress`; report keeps the keys `bench-gate-e2e.sh:17` reads (`p50 p95 p99 p99_9 max n accepted offered submitted`) |

### F. Cluster (≈480 LOC in `server.py` + `runtime.py` 364)

| Playground | What it does | Target |
|---|---|---|
| `runtime.py` `SYMBOLS`, `SCENARIOS`, `build_spawn_plan` `:188-364` | the env contract of every daemon; `SYMBOLS` is mirrored by `rsx-risk/migrations/006_seed_symbols.sql` ("mirror runtime.py exactly") | `cluster plan --scenario X --json`; scenarios in Rust; symbols read from PG `symbol_static` + `symbol_config_schedule` (decision D3) |
| `server.py:85-105` (inline `runtime` fallback), `pages.SYMBOL_CONFIG` `:5580`, `SYMBOL_NAMES` `:5573`, `:4431` | three more symbol tables | deleted; `config show --json` |
| `get_spawn_plan` `:638`, `start_all` `:1111-1241`, `stop_all` `:1242`, `_ensure_repl_certs` `:1064`, `do_build` `:925`, port and pid helpers `:446-534`, `spawn_process` `:680`, `stop_process` `:732`, `kill_process` `:771`, `restart_process` `:799`, `/api/processes/{name}/{action}` `:5318` (repeats the spawn) | build, certs, spawn, stop, ports, orphan reaping | `cluster up/down/restart/ps` (decision D1) |
| `scan_processes` `:1608-1710` | psutil table | `cluster ps --json` |
| `_process_watcher` `:816-924` | restart with backoff, circuit | **stays** as dashboard policy; it restarts via `cluster restart <name>` (D1) |
| `/api/reset` `:5254` | wipes WAL, truncates risk tables | `cluster reset` |
| `do_maker_start` `:988`, `do_auth_start` `:8677` | sidecars | `cluster up --only maker|auth` |

### G. Faults (≈225 LOC + `me_demo.py` 171)

| Playground | Target |
|---|---|
| `/api/fault/kill` `:5526` (SIGKILL, watcher restarts) | `fault kill <name>` |
| `/api/fault/net` `:5564` (`sudo tc netem` on `lo`) | `fault net --delay-ms --loss-pct`, `fault net --clear` |
| `/api/fault/wal-corrupt` `:5605` (flips byte 24 of the newest file) | `fault wal-corrupt <stream> [--offset]` |
| `/api/matching/crash/{name}` `:4525` | `fault kill me-<sym>`; the before/after compare stays as glue |
| `/api/matching/duplicate-demo` `:4583` + `me_demo.py` (hand-packed `OrderMessage`, copied crc32c table) | `fault dup-order`, built on the public `rsx-cast` `CastSender` and `rsx-messages` `OrderMessage`; no rsx-cast change |

### H. Config, accounts, risk reads (≈695 LOC of SQL)

| Playground | Target |
|---|---|
| config SQL `:3043-3084`, `/api/matching/config` `:4372`, `/v1/symbols` `:9373` | `config show --json` (scheduled + applied rows) |
| `POST /api/config/schedule` `:3105-3236` | `config schedule --symbol N --set field=value … [--effective-at-ms]` inserts a new version row; the writer goes in `rsx-risk/src/config.rs` beside `load_symbol_configs` (`:428`), since the schema is rsx-risk's |
| `seed_accounts` `:957` (1001 accounts, 1e17 collateral) | `account seed --count --collateral` |
| `/api/risk/overview` `:7733` (margin rates hard-coded `:7791`), `funding` `:7936`, `liquidations` `:7980`, `insurance` `:7999`, `_fetch_user_collateral` `:3005`, `/api/users/create` `:7511`, `x_risk_user` `:5081`, `x_liquidations` `:5112` | `account show`, `risk overview` (read-only, PG via `rsx_risk::db`) |
| stubs `deposit` `:7541`, `liquidate` `:7571`, `risk/users/{id}/{action}` `:7713` | **deleted**: they have no exchange effect (D5) |

### I. Marketdata wire

| Playground | Target |
|---|---|
| `md_wire.py` (116) + `_md_ws_subscriber` `:158-354` | **not confirmed as "stays".** It is the fourth decoder of a Rust-owned wire: `rsx-marketdata/src/wire.rs` encodes only; Go has `rsx-core/wire/md.go` and a copy in `rsx-maker/marketdata.go:78`. `stress --mode probe` and `order --market` need a Rust decoder anyway (`decode` beside `encode` in `wire.rs`). With it, the dashboard can run one long-lived `rsx-cli md follow --json` child and drop `md_wire.py` (D2) |

### Stays

| Item | Reason |
|---|---|
| `pages.py` rendering, `/x/*`, topology, docs loader `pages.py:6704-7280`, `cast_demo.py` | rendering and page content |
| logs (`read_logs` `:2384`, `/api/logs`), health/metrics scrapers `:3991-4170` | reads daemon output; nothing to share |
| auth proxy `:9433-9499`, roles `:1399-1458`, sessions `:9115`, idempotency `:1550`, run-id reaper `:583` | dashboard state |
| `notional.py` USD format/parse | display formatting; its sizing half (`min_order_qty`, `size_qty`) goes with the Python clients (`rsx_types::client_order` owns it) |
| `playground` launcher | the dashboard's entrypoint; `start-all`/`stop-all`/`stress` become thin calls into the server, which calls `rsx-cli` |
| `demo_scenario.py`, `demo/*`, `scripts/demo-trade.sh` | demo choreography stays, but every token, order and config action calls `rsx-cli` (D7) |

## 2. `rsx-cli` layout

Today: `main.rs` is 968 lines (arg parsing, filters, a 14-type decoder,
dumpers), there is no `lib.rs`, and there are three bins (`rsx-cli`,
`bench-probe`, `bench-match-rt`). The target is flat modules, a thin `main`,
and `src/<module>_test.rs` per module:

```
rsx-cli/src/
  main.rs          clap tree + dispatch to lib fns; nothing else
  lib.rs           pub mod list
  output.rs        --json vs text emit, the error line, exit codes 0/1/2 (die/misuse)
  record.rs        WAL record → JSON/text (main.rs:334-645 today); all rsx-messages types
  wal.rs           wal ls | dump | stats | verify (follow included)
  verify.rs        verify <wal_dir>... [-s stream] [--db url]: scan, replay, assemble the report
  check.rs         Check / Tally / Report, the check names, the JSON and text rendering
  ledger.rs        per-stream order lifecycle from the WAL records (verify 1, 2, 3, 5, 6)
  replay.rs        ME book rebuilt by rsx_matching::recovery, judged against the ledger (verify 3, 6, 8)
  persisted.rs     Postgres positions/tips/funding/locks judged against the ledger (verify 4, 5, 9, 10)
  pg.rs            the queries behind persisted.rs, the symbol config for the replay
  token.rs         token mint (rsx_messages::mint_jwt, D4)
  gateway_client.rs  WS connect, send N/C, read U/F/E via rsx_messages frames (D4)
  symbols.rs       symbol catalog + config from PG (rsx_risk::config)
  order.rs         order place | cancel | batch; human → raw; outcome classification
  md.rs            md snapshot | follow (rsx_marketdata::wire::decode)
  stress.rs        stress --mode load|probe|reject (bench_probe.rs + stress_client semantics)
  percentile_utils.rs  p50/p95/p99/p99_9 over samples
  cluster.rs       scenarios, plan, up/down/restart/ps/reset, pid files
  fault.rs         kill | net | wal-corrupt | dup-order
  config.rs        config show | schedule
  account.rs       account seed | show, risk overview
  bin/bench_match_rt.rs  stays (in-process bench, not a duplicated tool)
```

New dependencies: `rsx-risk` (PG connect + schema readers; already
`tokio-postgres`), `rsx-marketdata` (wire decode). The CLI does not depend on
`rsx-gateway` (D4): the client frames and the token mint come from
`rsx-messages` (steps 6–7). `rsx-cast` is used only through its existing
public API; it is never edited.

The CLI client becomes a real consumer of the server-frame parsers (moved to
`rsx-messages` at step 6, `records.rs:210-314` today), which settles BUGS
GW-SERVER-FRAME-PARSERS-UNUSED as "keep, used by the client".

Postgres: `DATABASE_URL` (the variable risk reads, `rsx-risk/src/config.rs:358`),
overridable with `--db`. Gateway: `--gateway ws://…` defaults to the `cluster plan` gw-0
address. JWT secret: `RSX_GW_JWT_SECRET`, as `bench_probe` reads today.

## 3. JSON contracts

Rules for every subcommand:

- `--json` on a finite command prints exactly one JSON object on stdout.
- A streaming command (`wal dump`, `--follow`, `md follow`, `order batch`)
  prints JSON Lines, one object per line, with no envelope.
- Errors: exit 1 (runtime) or 2 (misuse). With `--json`, stderr carries one
  line `{"error": "<message>"}` and stdout stays empty.
- `--stats --json` prints JSON. Today it prints text (`main.rs:675`); that is
  fixed.
- Prices, quantities and collateral are raw i64 JSON numbers. The consumer is
  Python, which keeps i64 exactly; collateral 1e17 is above 2^53, so a JS
  consumer must not parse these.
- Order ids are hex strings. Field names are the `rsx-messages` struct field
  names (`taker_user_id`, not Python's `taker_uid`).
- No version field: both sides ship in one repo and one commit. The contract
  is pinned by `rsx-cli/tests/json_contract_test.rs` (key sets per command)
  and by the gate-3 API tests on the routes that render them.

| Command | Output |
|---|---|
| `wal ls` | `{"wal_dir", "streams":[{"stream_id","files","bytes","first_seq","last_seq","last_ts_ns"}]}` |
| `wal dump` | per record `{"seq","type","len","crc32", …fields}` (today's shape, `main.rs:862-889`) |
| `wal stats` | `{"stream_id","records","by_type":{"FILL":n,…}}` |
| `wal verify` | `{"status":"ok\|corrupt","streams":[{"stream_id","records","first_seq","last_seq","crc_errors","seq_gaps":[[from,to]],"torn_tail","error"}]}` |
| `verify` | `{"status":"pass\|fail\|skip","checks":[{"invariant":1..10\|null,"name","status":"pass\|fail\|skip","detail","checked","violation_count","violations":[…≤10]}]}`; top-level `skip` means no check ran; a check that examined nothing is `skip`, never `pass`; a violation is an object with `message` and, for a WAL finding, `wal_dir`, `stream_id`, `seq`; exit 1 after the report when any check failed |
| `token mint` | `{"token","user_id","exp"}` |
| `order place` | `{"cid","status":"filled\|partial\|resting\|cancelled\|rejected\|timeout","reason_code","reason","order_id","fills":[{"px","qty","ts_ns"}],"frames":[…raw gateway frames],"rtt_us"}` |
| `order cancel` | `{"cid","status":"cancelled\|rejected\|timeout","reason_code","reason","frames","rtt_us"}` |
| `order batch` | stdin JSONL `{"user_id","symbol_id","side","price"\|"px","qty"\|"qty_raw","tif","reduce_only","post_only","cid"?}` → stdout one `order place` object per line, in input order, on one connection per user |
| `md snapshot` | `{"symbol_id","bbo":{"bid_px","bid_qty","ask_px","ask_qty","seq","ts_ns"}\|null,"bids":[[px,qty]],"asks":[[px,qty]]}` |
| `md follow` | per frame `{"kind":"bbo\|l2_snapshot\|l2_delta\|trade\|heartbeat", …}` |
| `stress` | `{"mode","config":{…},"offered","submitted","accepted","rejected_by_reason":{},"timeouts","latency_us":{"p50","p95","p99","p99_9","max","n"},"server_latency_ns":{…}}`, also written to `--out`; keeps the keys `bench-gate-e2e.sh:17` reads |
| `cluster plan` | `{"scenario","processes":[{"name","binary","args","env":{},"ports":[]}]}` |
| `cluster up/down/restart/reset` | `{"started":[],"stopped":[],"skipped":[{"name","reason"}]}` |
| `cluster ps` | `{"processes":[{"name","pid","state":"running\|exited\|absent","started_ms","log"}]}` |
| `fault kill` | `{"name","pid","signal"}` |
| `fault net` | `{"iface","delay_ms","loss_pct","active"}` |
| `fault wal-corrupt` | `{"stream_id","file","offset","old_byte","new_byte"}` |
| `fault dup-order` | `{"sent":2,"records":[…wal dump objects for that order]}` |
| `config show` | `{"symbols":[{"symbol_id","name","scheduled":[row],"applied":row\|null}]}`, rows = `symbol_config_schedule` columns |
| `config schedule` | `{"symbol_id","config_version","effective_at_ms"}` |
| `account seed` | `{"accounts","collateral"}` |
| `account show` / `risk overview` | `{"user_id","collateral","positions":[{"symbol_id","net_qty","avg_px","upnl"}],"frozen":[…]}` / per-user rows of the same shape |

Playground side: one glue module, `rsx_cli.py` (~60 LOC). It has `run(args)
-> dict` (`asyncio.create_subprocess_exec`, timeout, parse, raise `CliError`
carrying the stderr `error`) and `follow(args)`, an async iterator over JSON
Lines from a long-lived child. The binary is `target/debug/rsx-cli`, the path
`server.py:7648` already uses. There is **no Python fallback**: a missing binary is a
startup failure.

## 4. Gates and tests

- **Gate 1 (startup)** also runs `target/debug/rsx-cli --version`. A missing
  CLI fails the gate; it is never skipped. `make gate` gains `cargo build -p
  rsx-cli` as a prerequisite, the same as the daemons it already needs.
- **Gate 2 (partials)** is unchanged. Partials keep their HTML; the data comes
  from the CLI.
- **Gate 3 (API)** keeps every route that tests assert, with the same shape.
  The glue maps CLI JSON to the existing route JSON; when a route changes,
  its test changes in the same commit.
  - Tests that bind to internals are rewritten against routes, or move into
    `rsx-cli` Rust tests: `parse_wal_records`, `_build_order_msg`,
    `_classify_order_response`, `reject_reason_str`, `spawn_process`,
    `start_all`, `server.pg_query` (28 uses in `api_risk_test`), and
    `server.send_order_to_gateway`.
  - `tests/api_maker_test.py` joins the `_gate-api` list once the Python
    maker is gone (it runs the Go binary).
  - Deleted with their modules: `test_runtime.py` (75), `test_stress_api.py`
    (791), `stress_integration_test.py` (656), the stress/maker/scenario parts
    of `test_order_generators.py` (378), and `api_md_wire_test.py` (65). Their behaviour moves to `rsx-cli` tests: `cluster_test`,
    `stress_test` against a real gateway in `tests/` under `make
    integration`, and `md_test` against encoder golden bytes.
  - Only a mutation-proven Rust test may replace a deleted Python one. Each
    `verify` check gets a fixture WAL or PG state that makes it fail.
- **Gate 4 (Playwright)** drives HTTP only, so the specs stay. `signRsxJwt`
  is replaced by a helper that calls `rsx-cli token mint`.
- **`make test`** picks up `rsx-cli` unit tests once `lib.rs` exists.
  `rsx-cli/tests/*` (WAL fixtures, the JSON contract) run under `cargo test
  -p rsx-cli`. Anything that needs PG or a live gateway is `#[ignore]` under
  `make integration` (testcontainers).
- **`make perf-load`, `scripts/latency-publish.sh`** move to `rsx-cli
  stress` in the stress step. `make perf-e2e-gate` must keep passing against
  its sealed baseline, unchanged.
- `make gate` is never bypassed. Every playground step runs it whole. Step 0
  records a baseline (gate-3 count, gate-4 failures with host load), because
  Gate 4 is red on a loaded box today (PLAN ledger). A glue step is done when
  gates 1–3 are green and Gate 4 has no failure beyond the baseline.

## 5. Steps

One `sonnet` agent and one commit per step, sequential on the shared tree.
The parent reads each diff, and every added comment, before the next step.
The order follows the founder's: WAL, verify, the frame move (D4), order+token,
cluster, then the rest. Each Rust step lands before the glue step that consumes it.

| # | Step (commit) | Done when | Verify |
|---|---|---|---|
| 0 | `docs(bugs)`, `docs(ship)`: record in BUGS.md PG-VERIFY-FILLS-VACUOUS (`server.py:6356-6412`), PG-WAL-VERIFY-NO-CRC (`:7591`, `parse_wal_records` skips CRC), PG-QUICK-ORDER-NO-ADMIN (`:6871`), PG-STUB-RISK-ROUTES (deposit/liquidate/freeze), PG-SNAPSHOT-PARSER-DEAD (`:2063`), and extend PLAYGROUND-LOCAL-ONLY-BATCH-ORDERS (batch/random/invalid), each fix-now with the step that removes it; amend this step list for D1–D4; record the gate baseline in § 8 | entries cite lines that exist at HEAD; baseline written here | `git diff --stat` = BUGS.md + this file; `make gate` run whole, counts recorded |
| 1 | `refactor(cli)`: `lib.rs`, thin `main.rs`, `output.rs`, `record.rs`, `wal.rs`; clap tree `wal ls\|dump\|stats\|verify`; `wal-dump`/`dump` callers updated in the same commit (`server.py:7647`, `demo/wal_peek.sh`, `scripts/demo-trade.sh:107`); fix the `--follow` "not yet implemented" help text | byte-identical `--json` output on a fixture WAL before and after; `main.rs` < 200 lines | `cargo test -p rsx-cli` (16 existing pass); fixture diff; `make lint`; `make fmt-check` |
| 2 | `feat(cli)`: `wal ls`, `wal verify` (CRC, seq gaps, torn tail via `WalReader`), `--stats --json`, decode of every `rsx-messages` record type incl. 9/10/11; `json_contract_test.rs` | corrupt-byte fixture → `corrupt`; contract test pins the keys | `cargo test -p rsx-cli`; `make lint` |
| 3 | `refactor(playground)`: WAL glue — `rsx_cli.py`; one `wal dump --follow --json` child per stream feeding a record cache; delete `:1809-2049`, the snapshot parser, `scan_wal_*`; `/api/wal/verify` renders `wal verify`; heatmap/margin views read the cache until step 18; gate 1 checks the binary | `grep -n "struct.Struct\|RECORD_FILL" server.py` → nothing; route shapes unchanged | `make gate` whole vs baseline |
| 4 | `feat(cli)`: `verify` + `replay.rs` — checks per § 1 B over WAL dirs and PG; each check has a violating fixture | every check fails on its fixture and passes on a clean run | `cargo test -p rsx-cli`; PG checks `#[ignore]` under `make integration` |
| 5 | `refactor(playground)`: verify glue — `/api/verify/*`, `x_verify`, `x_invariant_status` render `verify --json`; delete `_run_invariant_checks`; reconciliation stays until step 14 | `api_verify_test`, `play_verify` green; a corrupted WAL (`fault` route) turns a row red | `make gate` whole |
| 6 | `refactor(messages)`: D4 — move the gateway client frames out of `rsx-gateway/src/records.rs` into a flat `rsx-messages/src/frames.rs`, re-exported from `lib.rs`: `WsFrame` (N/C/U/F/E/H/Q), `CancelKey`, `ParseError`, `parse`, `serialize` and their field helpers; `records_test.rs` moves with them; `serde_json` joins `rsx-messages`; every gateway src, test and bench imports `rsx_messages`; `records.rs` is deleted and no re-export is left behind; the rsx-messages and rsx-gateway README/ARCHITECTURE name the new owner | gateway behaviour unchanged; `rsx-cli` never depends on `rsx-gateway`; GW-SERVER-FRAME-PARSERS-UNUSED stays, now "used by the CLI client" (step 9) | `grep -rn "records::" rsx-gateway` → nothing; `cargo test -p rsx-messages -p rsx-gateway`; `make test`; `make lint`; `ws_parse_bench` and `ws_encode_bench` within 10% of before (a crate boundary can drop inlining; add `#[inline]` if so) |
| 7 | `feat(messages)`: `Claims` and `mint_jwt(&Claims, secret)` beside the frames (D4: the CLI cannot reach `rsx_gateway::jwt`, so the mint lives where both can; `jsonwebtoken` joins `rsx-messages`; see note 1); `rsx-gateway`'s `validate_jwt` uses the shared `Claims`; gateway benches and tests (the negative-claim tests build their own `Claims`) and `bench_probe` call `mint_jwt`; `rsx-cli token mint` | no `EncodingKey::from_secret` outside the `mint_jwt` body | `grep -rn EncodingKey --include=*.rs` → that one site; `cargo test -p rsx-messages -p rsx-gateway -p rsx-cli` |
| 8 | `feat(marketdata)`: `wire::decode` beside the encoders (pub prost types); `rsx-cli md snapshot\|follow` | decode(encode(x)) == x for every frame kind; golden bytes match `api_md_wire_test.py`'s | `cargo test -p rsx-marketdata -p rsx-cli` |
| 9 | `feat(cli)`: `order place\|cancel\|batch` — `gateway_client.rs` over the `rsx_messages` frames (step 6), `symbols.rs` from PG, human→raw with tick/lot/overflow checks, `--market` from `md snapshot`, `reason` by `FailureReason` name, `CidMint` cids | each outcome (filled, resting, rejected by reason, duplicate cid, timeout) seen against a real gateway | unit tests for conversion and classification; an `#[ignore]` cluster test under `make integration` |
| 10 | `refactor(playground)`: token + order glue — delete `_mint_user_token` and the inline mints, `_send_ws_frame`, `_build_order_msg`, `REJECT_REASONS`, `FAILURE_REASON_NAMES`, `order_ids.py`; batch/random/invalid go through `order batch` (D5; removes PLAYGROUND-LOCAL-ONLY-BATCH-ORDERS); `/api/orders/quick` calls `_require_admin_request` and takes the caller's user (removes PG-QUICK-ORDER-NO-ADMIN); `test_helpers.ts` and `conftest.py` mint with the CLI | `grep -n "pyjwt.encode" server.py` → nothing; `/api/orders/batch` orders appear in the ME WAL; a non-loopback caller of `/api/orders/quick` gets 403 | `make gate` whole |
| 11 | `feat(cli)`: `cluster plan\|up\|down\|restart\|ps\|reset` — scenarios in `cluster.rs`; the only launcher (D1), maker and auth sidecars included via `--only <name>`; symbols read from PG only and a clear error when `symbol_static` is empty (D3); detached spawn with pid files and logs in the playground's `tmp/` layout; certs | `cluster plan --scenario trio --json` equals today's `build_spawn_plan("trio")` env for every process | a test pinning plan equality, generated once from `runtime.py` before step 12 deletes it; `cluster up/ps/down` `#[ignore]` test |
| 12 | `refactor(playground)`: cluster glue — delete `runtime.py` (`SYMBOLS` included, D3), the `server.py:85-105` fallback, the spawn/stop/port/cert/build helpers, `do_maker_start`, `do_auth_start`, `scan_processes`; process routes, `/api/reset` and the launcher call `cluster`; no Python spawn path is left (D1); the watcher keeps its backoff and circuit and restarts via `cluster restart`; migration 006's comment points at PG as the catalog | `grep -rn "import runtime\|from runtime" rsx-playground` → nothing; `grep -n "def spawn_process\|def start_all\|def _ensure_repl_certs\|def do_build" server.py` → nothing | `make gate` whole; `api_processes_test`, `play_control`, `play_orch` |
| 13 | `feat(cli)`: `stress --mode load\|probe\|reject` — open-loop rate, workers, unique cids, reports to `--out`; delete the `bench-probe` bin; `scripts/latency-publish.sh` + `make perf-load` call it | report has the gate keys; probe no longer calls the playground over HTTP | `cargo test -p rsx-cli`; `make perf-load` then `make perf-e2e-gate` against the sealed baseline |
| 14 | `refactor(playground)`: stress glue — stress runs as a managed process; delete `stress.py`, `stress_client.py`, the probes `:8064-8401`; `x_reconciliation` uses `md snapshot`; delete `test_stress_api.py`, `stress_integration_test.py`, `test_runtime.py`, the stress parts of `test_order_generators.py` | `/api/stress/*` and `play_stress` green against CLI reports | `make gate` whole |
| 15 | `feat(cli)`: `fault kill\|net\|wal-corrupt\|dup-order`; `dup-order` on the public `CastSender` + `OrderMessage` | dup-order yields one ACCEPTED and one ORDER_FAILED(duplicate) in the ME WAL | unit tests for wal-corrupt; `#[ignore]` dup-order test |
| 16 | `refactor(playground)`: fault glue + delete `me_demo.py`; `/api/matching/crash` uses `fault kill` | `play_guarantees`, `api_matching_test` duplicate-demo green | `make gate` whole |
| 17 | `feat(risk)`: `config::schedule_symbol_config` writer (new version row, never updates the active row); `rsx-cli config show\|schedule`, `account seed\|show`, `risk overview` | a scheduled row is applied by risk and ME on their next poll | `#[ignore]` testcontainers tests; `make integration` |
| 18 | `refactor(playground)`: config/account glue — delete the config/seed/risk SQL, every static symbol table in the playground (`pages.SYMBOL_CONFIG`, `SYMBOL_NAMES`, `server.py:4431`; D3), the stubs (D5; removes PG-STUB-RISK-ROUTES); the heatmap/margin views read `account show`; `api_risk_test` drops `server.pg_query` | `grep -n "pg_query\|pg_pool" server.py` → only the dashboard's own tables, if any | `make gate` whole |
| 19 | `refactor(playground)`: delete `market_maker.py`, the `MAKER_SCRIPT` fallback (`:996-1001`, `:8798-8844`), its tests, `play-shard.sh:63`, the Makefile comments; `make maker` is a gate prerequisite; `api_maker_test.py` joins `_gate-api`; delete the synthetic `_maker_book` fallback and its four call sites (D5, "real system only"); the Go maker keeps minting its own token (D6) | maker always the Go binary; `grep -n _maker_book server.py` → nothing; an unseeded book renders empty, not synthetic | `make gate` whole; `make ci-full` market-maker shard |
| 20 | `refactor(playground)`: D2 — one long-lived `rsx-cli md follow --json` child replaces `_md_ws_subscriber` and `md_wire.py`; `api_md_wire_test.py` is deleted (its golden bytes live in step 8's test) | book renders live; "empty book on a running cluster" check holds; `grep -rn md_wire rsx-playground` → nothing | `make gate` whole |
| 21 | `refactor(demo)`: `demo_scenario.py`, `demo/*.py`, `scripts/demo-trade.sh`, `acceptance_test.py` call `rsx-cli` for tokens, orders, config, verify (D7: the choreography stays Python, no `rsx-cli scene`); symbol decimals (`demo/run.py` `TRIO`, `demo_scenario.py` `SYMBOL`) come from `config show` (D3) | no JWT or WS order client left in Python outside `_decode_user_token` | `grep -rn "jwt.encode\|\"N\":" rsx-playground scripts` → nothing; a demo run |
| 22 | `docs`: `rsx-cli` README/ARCHITECTURE + crate `CLAUDE.md` with keeper sections (thin main, JSON contract, no Python fallback); `rsx-playground/CLAUDE.md` Rules ("exchange tools are `rsx-cli`; the playground renders"); PLAN.md ledger; diary | docs match `rsx-cli --help` | `make test`, `make integration`, `make lint`, `make fmt-check`, `make gate` |

Notes on the 2026-10-26 amendment:

1. D4 consequence for the token. Step 7 puts `Claims` and `mint_jwt` in
   `rsx-messages` because the CLI no longer reaches `rsx-gateway`. The
   alternative, a second HS256 mint inside `rsx-cli`, repeats the claim set
   (`aud`, `iss`, `exp`, `jti`) and brings back the duplicate step 7 removes.
   Default is `rsx-messages`; founder to confirm before step 7 starts.
2. D1, D2, D3 and D5–D7 are folded into steps 10–12, 18–21 (each names its
   decision). D2 is no longer optional: step 20 always runs.
3. `rsx-cast` stays untouched by every step.

Ordering constraints:
- Step 3 needs 1–2. Step 7 needs 6. Step 9 needs 6–8. Step 11's plan-equality
  test is generated before step 12 deletes `runtime.py`. Step 13 needs 8–9 and
  11 (symbols, gw address). Step 18's heatmap change needs 17. Step 20 needs 8.
- Stress is the largest block (~1,850 LOC), but it follows cluster because it
  needs the token, the order client and the symbol catalog.

## 6. LOC leaving the playground

Estimates from the survey's line ranges. "Glue" is what replaces each block.

| Block | Leaves | Glue added | Net |
|---|---|---|---|
| A WAL | ~800 | ~120 | −680 |
| B verify | ~590 | ~70 | −520 |
| C+D token, orders, `order_ids.py` | ~965 | ~260 | −705 |
| F cluster (`runtime.py` + helpers + fallback) | ~990 | ~150 | −840 |
| E stress (`stress.py`, `stress_client.py`, probes) | ~2,000 | ~150 | −1,850 |
| G faults + `me_demo.py` | ~395 | ~40 | −355 |
| H config/accounts SQL + stubs + symbol tables | ~695 | ~200 | −495 |
| `market_maker.py` + fallbacks | ~690 | 0 | −690 |
| `notional.py` sizing half | ~60 | 0 | −60 |
| I `md_wire.py` + subscriber (D2) | ~310 | ~80 | −230 |
| **Python non-test** | **~7,500** | **~1,100** | **≈ −6,200 (−6,400 with D2)** of 20,790 |
| Python tests deleted or moved to Rust | ~2,000 (`test_stress_api` 791, `stress_integration_test` 656, `test_runtime` 75, generator parts, `api_md_wire_test` 65) | | |

`rsx-cli` grows by roughly 3,000 lines of Rust plus tests. `bench_probe.rs`
(518) is folded into `stress.rs`.

## 7. Founder decisions this plan needs

| # | Decision | Recommendation |
|---|---|---|
| D1 | Who spawns and supervises daemons | `rsx-cli cluster` is the one spawner (detached, pid files). The playground's watcher keeps its restart policy (backoff, circuit) and restarts through `cluster restart`. The alternative, keeping the Python spawner and moving only `cluster plan` to Rust, leaves two spawn paths |
| D2 | `md_wire.py` | Replace it with an `rsx-cli md follow --json` child (step 19). The Rust decoder is needed anyway for probe and `--market`. Keeping it means a fourth decoder, pinned only by golden bytes |
| D3 | Symbol catalog source | PG `symbol_static` + `symbol_config_schedule` (migration 006 is the one copy). `runtime.SYMBOLS` and the three `pages.py`/`server.py` tables go; `cluster plan` needs PG up first, which risk already requires |
| D4 | Where the client frame codec lives | decided 2026-10-26: the frames move into `rsx-messages` (step 6); `rsx-cli` does not depend on `rsx-gateway`. |
| D5 | Fake and stub routes | delete `deposit`/`liquidate`/`freeze` stubs; make batch/random/invalid real; delete the dead snapshot parser and the synthetic maker-book fallback (`_maker_book` `server.py:8999`, used at `:4762, :5071, :9062, :9099`; "real system only") |
| D6 | Go maker JWT (`rsx-maker/gateway.go:66`) | leave as is (the maker stays), or have the playground pass `RSX_MAKER_JWT` minted by `rsx-cli token mint` |
| D7 | Demo choreography (`demo_scenario.py`) | stays Python as demo glue, calling `rsx-cli` (step 20); no `rsx-cli scene` |
| D8 | Stage latency from log regex (`server.py:8426-8532`) | stays as log rendering; the authoritative numbers stay in benches and `stress` |

Out of scope: `rsx-cast` (frozen, used through its public API only),
`rsx-auth`'s issuer, and `rsx-glass`/`rsx-term`.

## Founder decisions — 2026-10-26

- D1: `rsx-cli cluster` is the only launcher; the playground keeps its auto-restart policy and restarts through `cluster restart`.
- D2: replace `md_wire.py` with `rsx-cli md follow --json` over a Rust decoder in rsx-marketdata.
- D3: the symbol list lives in Postgres only (risk's symbol config); every static copy goes.
- D4: the gateway's client frame types (N/C/U/F/E/H/Q) move into `rsx-messages`; rsx-cli does not depend on rsx-gateway.
- D5–D8 take the plan's recommended defaults (delete stub routes and the synthetic book fallback; the Go maker keeps minting its own token; demo scripts stay Python calling rsx-cli; stage latency stays display-only).

## 8. Gate baseline — 2026-10-26, HEAD `1909de33927da8fd2d73c268482f93aa1d0cb446`

One `make gate` run, started 15:33:49 UTC, ended 16:02:39 UTC (28.8 min),
exit 2 (Gate 4 red). Later steps compare against these numbers: a glue step is
done when Gates 1–3 match or beat this and Gate 4 has no failure beyond the
list below.

Host: 2 cores, 3.9 GiB RAM, no swap, shared with other services (dockerd, other
containers). Load average (1/5/15 min) 5.06 / 5.07 / 5.50 at the start, 13.51 /
14.71 / 13.33 at the end; RAM used 2.1 GiB of 3.9 GiB at both ends (about 1.8
GiB available). No RSX process was running before the run; none is left after.

| Gate | Result | Source |
|---|---|---|
| 1 startup/imports | pass (`server` imports) | re-run of `make _gate-partials` at this HEAD |
| 2 partials | 46 passed, 0 failed, 1.08 s | same re-run |
| 3 API | 746 passed, 0 failed, 0 skipped | `rsx-playground/tmp/gate-3-report.json`, written 15:38:48 by the gate |
| 4 Playwright | 433 passed, 7 failed, 0 flaky, 8 skipped (448 tests), 23.8 min | `rsx-playground/tmp/play-artifacts/run-20261026_153849/{report.json,run.log}` |

The gate wiped the repo-root `tmp/` during Gate 4 (a playground reset), taking
the `make gate` console log with it. Gates 1–2 therefore come from a stage
re-run at the same HEAD; Gate 3 and 4 numbers are the gate's own artifacts.
Write the console log outside `./tmp/` on the next run.

Gate 4 failures (7), with the cause the log shows:

| Spec | Failure | Class |
|---|---|---|
| `play_verify.spec.ts:94` each invariant row has a PASS/FAIL/WARN/SKIP badge | count of rows not greater than 0 | open; not load-shaped |
| `play_latency.spec.ts:68` HTMX partial < 500ms | `/x/risk-overview` took 1238 ms | load |
| `play_latency.spec.ts:130` API JSON endpoints < 200ms | `/api/mark/prices` took 232 ms | load |
| `play_guarantees.spec.ts:312` book page loads after recovery | `chrome-headless-shell` SIGSEGV on launch | box |
| `play_flows.spec.ts:26` order trace reaches a resolved state | `#trace-result` read "order pg6d not found in session" | open; not load-shaped |
| `play_links.spec.ts:112` every internal link resolves | 20 crate/component pages, spec wants at least 21 | open; fewer crate links than the spec counts (cause not checked) |
| `play_links.spec.ts:149` all 14 crate pages return 200 | 13 unique crate links, spec wants 14 | open; fewer crate links than the spec counts (cause not checked) |

Classes are read from the error text, not from a re-run. Two latency specs
assert milliseconds in a Playwright spec, against the playground rule that
specs are functional only; they are not removed here.

## 9. Step 5 landed — verify glue

`77c4fcb7` renders `rsx-cli verify --json` on the Verify page, `/api/verify/run`,
`/api/verify/run-json` and `/x/invariant-status` (`rsx-playground/invariants.py`);
`_run_invariant_checks` and its Python checks are deleted. `30bb5846` left
`tmp/wal/mark` out of the run while mark restarted its seqs at 1 on every Gate 4
run; mark resumes its seqs since `a5b117fe` and the run covers it again;
`e0c0207a` makes the Gate 3 teardown truncate risk's
Postgres state with the WAL, because a tip left in Postgres next to a deleted
WAL failed "Risk tips stay within the ME WAL" and "Position equals the sum of
fills" in Gate 4.

Gate, HEAD `e0c0207a`: Gates 1-3 pass (Gate 3: 745 passed, 0 failed), Gate 4
438 passed, 3 failed (play_flows:26, play_links:112 and :149, all open from the
baseline), 8 skipped; baseline 433/7, last 436/4.

## 10. Step 11 landed — `rsx-cli cluster`

`rsx-cli cluster plan|up|down|ps|restart|scenarios` (README § cluster). The
scenario table lives in `rsx-cli/src/scenario.rs`, the plan in `plan.rs`, the
symbols are read from Postgres only (`catalog.rs`, D3), the processes are
spawned and stopped by pid (`procs.rs`, D1), readiness is each daemon's
`/ready` (`probe.rs`).

Decisions taken in the step:

- Plan equality is pinned by `rsx-cli/tests/golden/cluster/*.json`, one file
  per scenario plus a pinned and a release variant, generated once from
  `runtime.py` `build_spawn_plan` (the generator is not kept; `runtime.py` goes
  in step 12). `plan_test.rs` compares process order, binary and env.
- The maker starts with every scenario (`rsx-playground/CLAUDE.md` wins over the
  Python comment), after the daemons are ready; `--no-maker` opts out. Its env
  carries the saved `tmp/maker-config.json` tuning and the plan's JWT secret.
  The auth sidecar is in the plan only when `rsx-auth/.venv` exists and the
  GitHub client id and secret are in the environment; it inherits the secrets
  from the caller, they are never written into the plan or the state files.
- `up` reports "up" when every daemon answers `/ready` (risk: only when Live; a
  warm replica: `/health`). No probe order is sent. `waited_ms` is the wait.
- Parent death: a daemon that dies with the CLI would die when `up` returns, so
  daemons run detached in their own process group and survive the CLI (and the
  playground), as they do under the Python launcher. The CLI itself takes
  `PR_SET_PDEATHSIG(SIGTERM)` and handles SIGTERM and SIGINT during `up` and
  `restart`: it stops what that call started and exits 1.
- Contract additions to § 3: `plan` processes carry `cwd`; `up` and `restart`
  print `waited_ms`; `cluster scenarios --json` exists so the playground can
  drop `SCENARIOS` (names, shapes, stress `load`).
- State is `tmp/cluster/state.json` (scenario, release, pin_cores, names) and
  `tmp/cluster/pids/<name>.json` (pid, kernel start time, start ms, log).
  Logs stay in `log/<name>.log`. `restart` rebuilds the plan from Postgres for
  the recorded scenario, so it needs Postgres up.

Not in this step: `cluster reset` (WAL wipe and risk table truncation behind
`/api/reset`), which step 12 needs; account seeding stays Python until step 17
(`cluster up` does not seed, as the live check below did by finding the 1001
accounts already there). `up` does not kill a stranger that holds a plan port
(the Python launcher used `pkill`/`fuser`): it exits 1 naming the ports.

Live check, 2026-10-27, minimal scenario: `up --build` waited 2.2 s, `ps`
showed 7 running, `order place` filled against the maker's book and rested a
second order, `restart gw-0` and `restart risk-0` (1.7 s) kept the order path,
`down` stopped 7 in 0.27 s with nothing left on the ports. A parent killed 1 s
into `up` made the CLI stop its 7 processes and exit with `interrupted`.

## 11. Step 12 landed — cluster glue

`6998c6ca` adds `rsx-cli cluster reset` (`down`, truncate risk's state, delete
`tmp/wal`, `tmp/md.tip`, `tmp/recorder-tip-*`); `2678dd22` makes `ps --json`
carry the scenario of the last `up`; `1acfac07` limits the plan-port check to a
cold start; `98696a0c` is the playground glue (`cluster_cli.py`).

Decisions taken in the step:

- `reset` truncates Postgres first and fails with the processes stopped and
  the WAL untouched when Postgres is down. It truncates `funding_intervals`
  as well, which the old wipe left behind; the old list named `funding` and
  `liquidations`, the tables after migration 002. The test teardown
  (`conftest.reset_cluster_state`) and `playground reset` call it; their
  `psql` copies are gone.
- `playground start-all` goes through the server, not straight to
  `cluster up`: the server seeds the 1001 test accounts first (BUGS
  PG-ACCOUNT-SEED-IN-SERVER, until step 17). `stop-all`, `ps`, `plan`,
  `scenarios`, `start-proc`, `stop-proc`, `restart-proc`, `reset` and
  `doctor`'s database default call the CLI; `demo` no longer starts the maker.
- `start_all` is `cluster up -b`: idempotent (a running process is skipped, a
  running cluster of another scenario is stopped first). A scenario switch is
  `down` then `up`, inside one `bulk_operation` that keeps the watcher out.
- The server's views: `scan_processes` is `cluster ps` plus the plan's names
  for what has not started, cached for a second; the plan and the symbol table
  are cached module state refreshed by the watcher and by the routes that read
  them. `current_scenario` follows `ps`.
- The watcher reads `ps` every two seconds. An `exited` record is a crash; a
  stopped process has no record (`absent`). A kill marks the exit
  intentional, then `down -o` drops the record. A manual restart resets the
  circuit.
- Removed: `/api/build`, `/api/build-log` (the plan moves `do_build` into
  `cluster up --build`; no page or script called them), the Python maker
  fallback (`MAKER_SCRIPT`; the file goes in step 19), `_maker_book`'s
  dependence on `managed`, the lifespan shutdown of daemons (they outlive the
  server; Playwright's `global-teardown.ts` stops what `global-setup.ts`
  started). `RSX_PLAYGROUND_RELEASE=1` still selects release binaries.
- Left for later steps: `pages.SYMBOL_CONFIG` and `SYMBOL_NAMES` (step 18),
  stress as a managed child of the server (step 14), `md_wire.py` (step 20).

Found live (2026-10-27, minimal): the first restart of `me-pengu` through
the watcher failed on "ports already in use: 9510" (marketdata binds 9510
by default; BUGS CLUSTER-PLAN-PORTS-ARE-A-HEURISTIC), and a scenario switch
raced the watcher, which restarted processes `down` had just stopped (fixed by
`bulk_operation`). After both: kill, start, stop, restart of `me-pengu`, maker
stop/start, scenario switch (7.5 s, an order filled at once), reset (stopped
7, cleared risk-state, wal, tips; `tips` empty), `fault/kill` auto-restarted
in under 2 s.

Gate 3 (`make _gate-api`) at this step: 719 passed, 0 failed. 26 fewer than
the 745 of step 5: the tests of the deleted spawn, port, scan and build
internals are gone, and `api_processes_test.py` now runs over an in-memory
cluster (`tests/fake_cluster.py`) plus two read-only calls of the real CLI.
