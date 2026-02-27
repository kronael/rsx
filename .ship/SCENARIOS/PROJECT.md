# PROJECT.md

## Goal
Fix multi-symbol process spawning in the RSX exchange `start` script and
dependent Rust crates so all scenarios (`minimal`, `standard`, `duo`,
`full`, `stress`) spawn the correct set of processes with correct routing.

## Stack
- **Language:** Python 3 (`./start` script), Rust (rsx-risk, rsx-marketdata)
- **Build:** `cargo build` (debug), `make e2e`, `make smoke`
- **Runtime:** Linux, local dev, no containers

## Scope (5 tasks)

1. **rsx-risk**: Read `RSX_ME_CMP_ADDRS` (comma-separated), build
   `HashMap<symbol_id, SocketAddr>`, route outbound orders by symbol_id.
   Fallback to `RSX_ME_CMP_ADDR` for backwards compat.

2. **rsx-marketdata**: Read `RSX_ME_CMP_ADDRS`, subscribe to all MEs on
   startup. Fallback to `RSX_ME_CMP_ADDR`.

3. **`start` script — Mark URL**: Build combined Binance stream URL from
   all symbols in config, not `symbols[0]`.

4. **`start` script — env vars**: Compute `me_cmp_addrs` as comma-joined
   list of all ME CMP addresses; pass as `RSX_ME_CMP_ADDRS` to Risk,
   Risk replicas, and Marketdata entries in `build_spawn_plan()`.

5. **`start` script — Recorder**: Append one `("recorder", binary, env)`
   entry per scenario using `RSX_RECORDER_STREAM_ID` (first symbol id),
   `RSX_RECORDER_PRODUCER_ADDR` (first ME replay port),
   `RSX_RECORDER_ARCHIVE_DIR=./tmp/wal/archive`,
   `RSX_RECORDER_TIP_FILE=./tmp/recorder-tip-<sid>`.

## IO Surfaces
- `build_spawn_plan(config, pg_url)` returns `[(name, binary, env), ...]`
- Risk reads `RSX_ME_CMP_ADDRS` env var at startup
- Marketdata reads `RSX_ME_CMP_ADDRS` env var at startup
- Playground `get_spawn_plan()` delegates unchanged — no server.py edits

## Constraints
- No changes to `server.py` (playground delegates to `build_spawn_plan`)
- Backwards compat: single-addr `RSX_ME_CMP_ADDR` still works
- Debug builds only
- Port formula fixed: ME CMP = 9100 + symbol_id, etc.

## Success Criteria (verifiable process counts)

| Scenario | Expected processes |
|----------|--------------------|
| minimal  | 6: me-pengu, risk-0, gw-0, marketdata, mark, recorder |
| duo      | 7: me-pengu, me-sol, risk-0, gw-0, marketdata, mark, recorder |
| full     | 9: +me-btc, risk-0-replica-0 |
| stress   | 12: +risk-0-replica-1, gw-1 |

Routing: SOL order via gw-0 fills on me-sol, not me-pengu.
Marketdata WS delivers SOL L2 updates in `duo` scenario.
Scenario switch via `POST /api/scenario/switch` completes within 10s.

## Test Targets
- `make e2e`: spawn each scenario, verify process count, submit one order
  per symbol, verify fill, confirm no cross-symbol routing
- `make smoke`: `./start duo` + stress both symbols + MD WS check
