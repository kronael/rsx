# SCENARIOS.md

Deployment scenarios for the RSX exchange. Defines which processes
are spawned per scenario and how ports are allocated.

## Current State

### What Works
- `minimal` scenario: 1 ME (PENGU), 1 Risk, 1 Gateway, 1
  Marketdata, 1 Mark, 1 Recorder
- `build_spawn_plan()` in `start` script correctly iterates
  `config["symbols"]` to spawn one ME per symbol
- `get_spawn_plan()` in `rsx-playground/server.py` correctly
  calls `select_symbols(preset["symbols"])` to pick N symbols
- Scenario switcher UI exists: `POST /api/scenario/switch`
- All scenario presets defined in `SCENARIOS` dict

### What Is Broken
- Marketdata is hardcoded to `symbols[0]` — only connects to
  the first ME regardless of symbol count (multi-symbol blind)
- Risk is hardcoded to `symbols[0]` for `RSX_ME_CMP_ADDR` —
  only forwards to first ME (multi-symbol broken)
- Risk replicas also hardcode `symbols[0]` for `RSX_ME_CMP_ADDR`
- Mark uses only first symbol for Binance WS URL — combined
  stream endpoint needed for multi-symbol
- No Recorder process in `build_spawn_plan()` at all (missing)

---

## Scenario Matrix

| Scenario     | Code | Symbols      | MEs | GWs | Replicas | Load    |
|--------------|------|--------------|-----|-----|----------|---------|
| minimal      | 1Z   | PENGU        | 1   | 1   | none     | -       |
| standard     | 1    | PENGU        | 1   | 1   | 1×risk   | -       |
| duo          | 2Z   | PENGU, SOL   | 2   | 1   | none     | -       |
| full         | 3    | PENGU+SOL+BTC| 3   | 1   | 1×risk   | -       |
| stress       | M3S  | PENGU+SOL+BTC| 3   | 2   | 2×risk   | -       |
| stress-low   | -    | PENGU+SOL+BTC| 3   | 1   | none     | 10/s    |
| stress-high  | -    | PENGU+SOL+BTC| 3   | 1   | 1×risk   | 100/s   |
| stress-ultra | -    | PENGU+SOL+ETH+WIF| 4 | 2 | 1×risk   | 500/s   |

Symbol selection order: PENGU, SOL, BTC, ETH, WIF, BONK, PEPE, DOGE

Replicas: per-shard count. "1×risk" = 1 replica per primary risk
shard. "2×risk" = 2 replicas per primary (spare mode).

---

## Port Allocation

```
BASE_ME_CMP      = 9100   ME CMP: 9100 + symbol_id
BASE_RISK_CMP    = 9200   Risk primary: 9200 + shard_id
                           Risk replicas: 9210 + shard*2 + replica
BASE_GW_CMP      = 9300   GW CMP: 9300 + gw_index
BASE_MARK_CMP    = 9400   Mark aggregator (single)
BASE_MD_CMP      = 9500   Marketdata CMP: 9500 + symbol_id
BASE_RISK_MARK   = 9600   Mark→Risk push (single)
BASE_GW_WS       = 8080   GW WebSocket: 8080 + gw_index
BASE_MD_WS       = 8180   Marketdata WebSocket (single)
```

Symbol IDs are defined in `SYMBOLS` dict in the `start` script
(PENGU=0, SOL=1, BTC=2, ETH=3, ...). Port offsets use `symbol_id`
so ports are stable across scenarios.

Example — `duo` (PENGU id=0, SOL id=1):
```
me-pengu     CMP 9100    (9100 + 0)
me-sol       CMP 9101    (9100 + 1)
risk-0       CMP 9200
gw-0         CMP 9300, WS 8080
marketdata   WS  8180    (must connect to both MEs)
mark         CMP 9400
```

---

## Process Spawn Rules

### ME (one per symbol)
Already correct. Each ME gets its own CMP port and WAL dir.
No changes needed.

### Risk (one per shard)
`RSX_ME_CMP_ADDR` must list all ME CMP addresses, not just
`symbols[0]`. Risk forwards orders to the correct ME by
`symbol_id`. Change to comma-separated list:

```
RSX_ME_CMP_ADDRS = "127.0.0.1:9100,127.0.0.1:9101"
```

Risk engine must parse this and route by symbol_id index.

### Risk Replicas
Same fix as primary: `RSX_ME_CMP_ADDRS` comma-separated.

### Marketdata (one per symbol, or one multi-symbol)
Two options (pick one):

**Option A — one Marketdata per symbol (simpler):**
Spawn `marketdata-{name}` for each symbol. Each connects to
its ME only. Clients subscribe by symbol. MD WS port per
symbol: `BASE_MD_WS + symbol_id` (8180, 8181, ...).

**Option B — single Marketdata, multi-ME (current intent):**
`RSX_MKT_CMP_ADDRS` = comma-separated list. Marketdata
subscribes to all MEs and fans out by symbol_id. Single WS
port (8180). Clients filter by symbol in subscription message.

Option B is consistent with existing single-port design.

### Mark
Binance combined stream supports multiple symbols:
```
wss://stream.binance.com:9443/stream?streams=
  pengusdt@trade/solusdt@trade/btcusdt@trade
```
Build URL from all symbols in config, not just `symbols[0]`.

### Gateway
Already correct. All GW instances get per-symbol tick/lot env
vars from the full symbols list.

### Recorder
Missing from `build_spawn_plan()`. Add one Recorder process
per scenario. Recorder subscribes to WAL output from ME(s).

```
RSX_RECORDER_WAL_DIR = ./tmp/wal/recorder
RSX_RECORDER_ME_CMP_ADDRS = <all ME CMP addrs>
```

---

## Implementation Tasks

### 1. Fix Risk multi-symbol routing (`rsx-risk`)
- Replace `RSX_ME_CMP_ADDR` (single) with `RSX_ME_CMP_ADDRS`
  (comma-separated)
- Route outbound order by `symbol_id` to the correct ME CMP
  socket

### 2. Fix Marketdata multi-symbol (`rsx-marketdata`)
- Replace `RSX_MKT_CMP_ADDR` and `RSX_ME_CMP_ADDR` (single)
  with `RSX_MKT_CMP_ADDRS` and `RSX_ME_CMP_ADDRS`
- Subscribe to all MEs on startup

### 3. Fix Mark multi-symbol Binance URL (`start` script)
- Build combined stream URL from all symbols in config
- Replace hardcoded `symbols[0]` URL construction

### 4. Fix `build_spawn_plan` env for Risk + Marketdata
  (`start` script)
- Compute `me_cmp_addrs` as comma-joined list before Risk loop
- Pass it as `RSX_ME_CMP_ADDRS` to Risk and replicas
- Pass it as `RSX_ME_CMP_ADDRS` to Marketdata

### 5. Add Recorder to `build_spawn_plan` (`start` script)
- Append one `("recorder", ...)` entry with WAL dir env

### 6. Verify playground scenario switcher
- `get_spawn_plan()` in `server.py` already calls
  `select_symbols(preset["symbols"])` correctly — no changes
  needed after tasks 1–5 are done

---

## Acceptance Criteria

- `./start duo` spawns: me-pengu, me-sol, risk-0, gw-0,
  marketdata, mark, recorder (7 processes)
- `./start full` spawns: 3 MEs, risk-0 + 1 replica, gw-0,
  marketdata, mark, recorder (8 processes)
- `./start stress` spawns: 3 MEs, risk-0 + 2 replicas, gw-0,
  gw-1, marketdata, mark, recorder (10 processes)
- SOL order submitted via gw-0 routes to me-sol (not me-pengu)
- Marketdata WebSocket delivers SOL L2 updates for `duo`
- Scenario switch via `POST /api/scenario/switch` tears down
  old processes and starts new set within 10s

---

## Testing

`make e2e` — scenario spawn tests (fast, no load):
- Start each scenario, verify expected process count
- Submit one order per symbol, verify fill returned
- Confirm no cross-symbol routing (PENGU fill not on SOL book)

`make smoke` — deployed system (requires running exchange):
- `./start duo` then run `rsx-sim` against both symbols
- Check Marketdata WS receives updates for both symbols
- Check `./start stress` with 2 gateways load-balances clients

Port conflict check: all port assignments in scenario matrix
must be disjoint. Verified statically by `scripts/check_ports.py`
(to be written).
