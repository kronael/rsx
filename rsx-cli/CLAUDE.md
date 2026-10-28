# CLAUDE.md — rsx-cli

Local to `rsx-cli/`; the root `../CLAUDE.md` applies. `rsx-cli` is the one home
of the exchange's operator tools: WAL decode and check, invariant checks, the
token mint, the gateway order client, stress, the cluster launcher, faults, and
symbol config and account reads and writes. The playground and the demo scripts
call it and render what it prints. Commands and flags:
`README.md`; modules and JSON shapes: `ARCHITECTURE.md`.

## Rules

- A command is a library function that returns `Outcome`. A new command is a
  module in `src/`, one `run_<group>` arm in `main.rs`, a README section, an
  ARCHITECTURE module line and JSON row, and a contract test, in one commit.
- `rsx-cast` is frozen: a fix here uses the `WalReader`, `CastSender` and
  `CastReceiver` APIs that exist. The wire records and gateway frames live in
  `rsx-messages`; NEVER copy a layout into this crate.
- One owner per fact. Symbol config is read from Postgres and written only
  through `rsx_risk::config::schedule_symbol_config`, which never updates the
  active row. Risk owns the schema and the migrations; the CLI applies them
  and writes no table of its own. The JWT mint is `rsx_messages::mint_jwt`;
  NEVER a second `EncodingKey` in this crate.
- A command that signals a process signals only a pid whose record still has
  the kernel start time it was stored with (`procs.rs`). NEVER `kill` by a
  name or a bare pid.
- A Postgres url is printed through `catalog::redact`; no message carries a
  password.
- Exit codes: 0 success, 1 runtime failure, 2 misuse.
- Unit tests sit in `src/<module>_test.rs`. `tests/` drives the built binary.
  A test that needs Docker, `sudo` or a built daemon is `#[ignore]`; Docker
  tests run under `make integration`.

## Keeper sections — do not regress

- **Thin main.** `main.rs` holds the clap tree and the dispatch, nothing
  else: it maps the flags to the library's input types and calls the library.
  A rule, a conversion or a check belongs in a module, where a unit test
  reaches it. `main.rs` is long because the clap tree is; it never gains
  logic.
- **The JSON contract.** `--json` prints exactly one JSON object on stdout,
  or JSON lines for a stream (`wal dump`, `md follow`, `order batch`). The
  keys are the contract: `tests/json_contract_test.rs` pins them, and the
  ARCHITECTURE table and the README section state them. A changed, renamed or
  dropped key changes the playground glue that reads it (the `*_cli.py` files,
  `md_feed.py` and `wal_feed.py` in `rsx-playground/`) in the same commit. Under `--json`
  an error is one `{"error": "<message>"}` line on stderr with stdout empty.
  A command with a verdict, such as `verify`, `wal verify` and `stress`,
  prints its report and then exits 1, so a failed run still has its JSON.
  Timestamps and quantities are raw integers, never formatted strings.
- **No Python fallback.** The tools above exist once, here. NEVER
  reimplement a command, a decoder, a mint or a launcher in Python, shell or
  Go because a flag is missing: add the flag. A caller that cannot find
  `target/debug/rsx-cli` stops and says so; it does not fall back to a copy.
  This includes tests, demos and the playground's own routes.
- **The report is honest.** A check that examined nothing is `skip`, never
  `pass`. A timeout is `timeout`, not a failure of the exchange and not a
  success. A command that cannot reach the cluster says so; it never prints a
  synthetic result.
- **Float formatting works in a debug build.** `tests/float_format_test.rs`
  guards the cranelift workaround in `.cargo/config.toml`; do not remove it.

A new or changed command updates `README.md`, the `ARCHITECTURE.md` JSON table,
`json_contract_test.rs` and the playground or script that calls it, then
re-runs `make test` and `make integration`.
