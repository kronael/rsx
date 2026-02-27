# SCENARIOS.md

Deployment scenarios for the RSX exchange. Defines which
processes are spawned per scenario and how ports are allocated.

The `start` script is Python (`./start`, shebang
`#!/usr/bin/env python3`). It defines `SCENARIOS`,
`SYMBOLS`, `select_symbols()`, `build_spawn_plan()`.

## Current State

### What Works
- `minimal` scenario: 1 ME (PENGU), 1 Risk, 1 Gateway, 1
  Marketdata, 1 Mark
- `build_spawn_plan()` in `start` correctly iterates
  `config["symbols"]` to spawn one ME per symbol
- `get_spawn_plan()` in `rsx-playground/server.py` calls
  `start_mod.build_spawn_plan(config, PG_URL)` which
  returns `[(name, binary, env), ...]` — playground passes
  env dicts directly to subprocess. After tasks 1-5,
  playground automatically picks up new env var names
  because it delegates to `build_spawn_plan()`.
- Scenario switcher UI: `POST /api/scenario/switch`
- All scenario presets defined in `SCENARIOS` dict

### What Is Broken
- Marketdata hardcoded to `symbols[0]` — only connects to
  the first ME regardless of symbol count
- Risk hardcoded to `symbols[0]` for `RSX_ME_CMP_ADDR` —
  only forwards to first ME
- Risk replicas also hardcode `symbols[0]`
- Mark uses only first symbol for Binance WS URL
- No Recorder process in `build_spawn_plan()`

---

## Scenario Matrix

Active scenarios (in `SCENARIOS` dict):

| Scenario | Code | Symbols       | MEs | GWs | Replicas |
|----------|------|---------------|-----|-----|----------|
| minimal  | 1Z   | PENGU         | 1   | 1   | none     |
| standard | 1    | PENGU         | 1   | 1   | 1×risk   |
| duo      | 2Z   | PENGU, SOL    | 2   | 1   | none     |
| full     | 3    | PENGU+SOL+BTC | 3   | 1   | 1×risk   |
| stress   | M3S  | PENGU+SOL+BTC | 3   | 2   | 2×risk   |

Deferred (in `SCENARIOS` dict but not tested, out of scope
for this spec):
- `stress-low`, `stress-high`, `stress-ultra` — load
  variants with `load` config. Implementation deferred
  until stress.py subprocess management is wired up.

Symbol selection order: PENGU, SOL, BTC, ETH, WIF, BONK,
PEPE, DOGE.

Replicas: per-shard count. "1×risk" = 1 replica per
primary risk shard. "2×risk" = 2 replicas (spare mode).

---

## Port Allocation

```
BASE_ME_CMP      = 9100   ME CMP: 9100 + symbol_id
BASE_RISK_CMP    = 9200   Risk primary: 9200 + shard_id
                           Risk replicas: 9210+shard*2+replica
BASE_GW_CMP      = 9300   GW CMP: 9300 + gw_index
BASE_MARK_CMP    = 9400   Mark aggregator (single)
BASE_MD_CMP      = 9500   Marketdata CMP: 9500 + symbol_id
BASE_RISK_MARK   = 9600   Mark→Risk push (single)
BASE_GW_WS       = 8080   GW WebSocket: 8080 + gw_index
BASE_MD_WS       = 8180   Marketdata WebSocket (single)
```

Symbol IDs from `SYMBOLS` dict in `start` script:
PENGU=10, SOL=3, BTC=1, ETH=2.

Example — `duo` (PENGU id=10, SOL id=3):
```
me-pengu     CMP 9110   (9100 + 10)
me-sol       CMP 9103   (9100 + 3)
risk-0       CMP 9200
gw-0         CMP 9300, WS 8080
marketdata   WS  8180   (subscribes to both MEs via CMP)
mark         CMP 9400
recorder     TCP connects to ME replay servers
```

---

## Process Spawn Rules

### ME (one per symbol)
Already correct. Each ME gets its own CMP port and WAL
dir. No changes needed.

### Risk (one per shard)
`RSX_ME_CMP_ADDR` must become `RSX_ME_CMP_ADDRS` (comma-
separated). Risk routes outbound orders to the correct ME
by `symbol_id`.

```
RSX_ME_CMP_ADDRS = "127.0.0.1:9110,127.0.0.1:9103"
```

Risk parses this into a `Vec<SocketAddr>`, routes by
`symbol_id` to the matching ME address.

### Risk Replicas
Same fix: `RSX_ME_CMP_ADDRS` comma-separated.

### Marketdata (single process, multi-ME — Option B)

Single marketdata process subscribes to all MEs via CMP.
`RSX_ME_CMP_ADDRS` = comma-separated list of all ME CMP
addresses. Single WS port (8180). Clients filter by
symbol in subscription message.

This is consistent with existing single-port design and
the `S:[sym, channels]` subscription protocol.

### Mark
Binance combined stream supports multiple symbols:
```
wss://stream.binance.com:9443/stream?streams=
  pengusdt@trade/solusdt@trade/btcusdt@trade
```
Build URL from all symbols in config, not just
`symbols[0]`.

### Gateway
Already correct. All GW instances get per-symbol tick/lot
env vars from the full symbols list.

### Recorder
Missing from `build_spawn_plan()`. Add one Recorder per
scenario. Recorder is a DxsConsumer (TCP client), NOT a
CMP subscriber. It connects to an ME's DXS replay server
to archive WAL records.

Env vars (from `RecorderConfig::from_env()`):
```
RSX_RECORDER_STREAM_ID      = <symbol_id>
RSX_RECORDER_PRODUCER_ADDR  = 127.0.0.1:<me_replay_port>
RSX_RECORDER_ARCHIVE_DIR    = ./tmp/wal/archive
RSX_RECORDER_TIP_FILE       = ./tmp/recorder-tip-<sid>
```

For multi-symbol, spawn one recorder per ME (each ME is a
separate WAL stream). Or spawn one recorder for the
primary symbol only (simpler, sufficient for dev).

---

## Implementation Tasks

### 1. Fix Risk multi-symbol routing (`rsx-risk`)
- Read `RSX_ME_CMP_ADDRS` (comma-separated), fall back to
  `RSX_ME_CMP_ADDR` for backwards compat
- Build `HashMap<symbol_id, SocketAddr>` mapping
- Route outbound orders to correct ME by symbol_id

### 2. Fix Marketdata multi-symbol (`rsx-marketdata`)
- Read `RSX_ME_CMP_ADDRS` (comma-separated), fall back to
  single `RSX_ME_CMP_ADDR`
- Subscribe to all MEs on startup via CMP

### 3. Fix Mark multi-symbol Binance URL (`start` script)
- In `build_spawn_plan()`, build combined stream URL from
  all symbols in config
- Replace hardcoded `symbols[0]` URL construction

### 4. Fix `build_spawn_plan` env vars (`start` script)
- Compute `me_cmp_addrs` as comma-joined list of all ME
  CMP addresses before the Risk/Marketdata entries
- Pass as `RSX_ME_CMP_ADDRS` to Risk, Risk replicas, and
  Marketdata
- Playground's `get_spawn_plan()` delegates to
  `build_spawn_plan()` and passes env dicts to subprocess
  unchanged — no server.py changes needed

### 5. Add Recorder to `build_spawn_plan` (`start` script)
- Append one `("recorder", target/debug/rsx-recorder, env)`
  entry per scenario
- Set `RSX_RECORDER_STREAM_ID` to first symbol's id
- Set `RSX_RECORDER_PRODUCER_ADDR` to first ME's replay
  address
- Set `RSX_RECORDER_ARCHIVE_DIR` and
  `RSX_RECORDER_TIP_FILE`

---

## Acceptance Criteria

Process counts per scenario:

- `./start minimal` spawns 6: me-pengu, risk-0, gw-0,
  marketdata, mark, recorder
- `./start duo` spawns 7: me-pengu, me-sol, risk-0, gw-0,
  marketdata, mark, recorder
- `./start full` spawns 9: me-pengu, me-sol, me-btc,
  risk-0, risk-0-replica-0, gw-0, marketdata, mark,
  recorder
- `./start stress` spawns 12: me-pengu, me-sol, me-btc,
  risk-0, risk-0-replica-0, risk-0-replica-1, gw-0, gw-1,
  marketdata, mark, recorder

Routing:
- SOL order submitted via gw-0 routes to me-sol (not
  me-pengu)
- Marketdata WS delivers SOL L2 updates for `duo`

Playground:
- Scenario switch via `POST /api/scenario/switch` tears
  down old processes and starts new set within 10s

---

## Testing

`make e2e` — scenario spawn tests (fast, no load):
- Start each scenario, verify expected process count
- Submit one order per symbol, verify fill returned
- Confirm no cross-symbol routing (PENGU fill not on SOL)

`make smoke` — deployed system (requires running exchange):
- `./start duo` then run stress against both symbols
- Check Marketdata WS receives updates for both symbols
- Check `./start stress` with 2 gateways load-balances
