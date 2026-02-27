# SIM.md — Remove Sim Mode + Stress Generator

## Context

The playground has fake order-matching (`_sim_submit`,
`_sim_book`, `_seed_sim_book`) for when gateway is offline.
Remove it — empty state when processes are down is correct.

`stress_client.py` already works as a real WS load generator.
`rsx-sim/` (Rust) is a dead stub. Delete it. Promote the
Python stress client to a managed subprocess.

---

## 1. Remove Sim Mode from server.py

### Delete

Globals: `_sim_book`, `_sim_wal_events`, `_sim_seq`

Functions: `_seed_sim_book()`, `_sim_submit()`

Callsites:
- `_seed_sim_book()` in `lifespan()` startup
- Sim fallback in `POST /api/orders/test`
- `_sim_wal_events` merge in `GET /x/wal-events`
- `_seed_sim_book()` fallbacks in `x_book()`,
  `x_book_stats()`, `api_book()`
- `_sim_book` reads in `GET /v1/orders`
- Insurance fund "simulated" hardcoded fallback
- Mark prices "sim" source fallback

### Endpoint behavior after removal

| Endpoint | Gateway down |
|----------|-------------|
| POST /api/orders/test | Error: "gateway not running" |
| GET /x/book | WAL / maker / empty |
| GET /x/book-stats | WAL / maker / empty |
| GET /api/book/{id} | WAL / maker / empty |
| GET /x/wal-events | Real WAL only |
| GET /v1/orders | recent_orders only |
| GET /api/risk/insurance | Empty list |
| GET /api/mark/prices | WAL / empty |

### Fix tests

Remove "simulated" from accepted statuses. Expect error/503
when gateway down. Delete sim-only tests:
- `tests/api_e2e_test.py`
- `tests/api_integration_test.py`
- `tests/api_orders_test.py`
- `tests/test_order_api.py` (gw_down sim tests)
- `tests/api_maker_test.py` (seed_sim_book tests)
- `tests/play_safety.spec.ts`
- `tests/play_guarantees.spec.ts`

---

## 2. Delete rsx-sim/ Rust Crate

- `rm -rf rsx-sim/`
- Remove from workspace `Cargo.toml`
- Remove from Makefile targets

---

## 3. Stress Generator

### Files

```
rsx-playground/stress.py         — entry point (NEW)
rsx-playground/stress_client.py  — library (existing)
```

`stress.py` is the standalone script. `stress_client.py`
has the WS client logic (StressConfig, StressClient,
run_stress_test). No changes to the library.

### stress.py

```
python3 stress.py
```

Config via env vars:

| Variable | Default | Description |
|----------|---------|-------------|
| RSX_STRESS_GW_URL | ws://localhost:8080 | Gateway WS |
| RSX_STRESS_USERS | 10 | Concurrent sessions |
| RSX_STRESS_RATE | 1000 | Orders/sec total |
| RSX_STRESS_DURATION | 60 | Seconds (0=infinite) |
| RSX_STRESS_TARGET_P99 | 50000 | p99 target (us) |
| RSX_STRESS_REPORT_DIR | ./tmp/stress | Report output |

Behavior:
- Import `stress_client.run_stress_test`
- Run with config from env vars
- Print periodic stats to stdout (5s interval)
- Write final report JSON to `REPORT_DIR/`
- Exit 0 if p99 < target, exit 1 otherwise
- SIGTERM: cancel workers, write partial report

### Playground management

Follow maker pattern:

```python
STRESS_SCRIPT = ROOT / "rsx-playground" / "stress.py"
STRESS_NAME = "stress"
```

- `do_stress_start(cfg)` — spawn subprocess, register in
  `managed`, PID file, pipe_output to `log/stress.log`
- `do_stress_stop()` — SIGTERM, cleanup
- Topology handler `_topo_stress()`
- API: `POST /api/stress/start`, `POST /api/stress/stop`,
  `GET /api/stress/status`

### Remove in-process stress

Delete from server.py:
- `STRESS_SCENARIOS` dict, `_run_scenario_loop()`
- `stress_tasks`, `stress_scenario_metrics` globals
- In-process scenario start/stop/status endpoints
- Baseline auto-start in `@app.on_event("startup")`

Keep:
- `/stress` page (update for subprocess status)
- Stress report storage + list/view endpoints

---

## 4. Update Docs

- `TODO.md`: remove rsx-sim, update stress
- `FEATURES.md`: rsx-sim → stress.py
- `PROGRESS.md`: rsx-sim → deleted
- `TESTING.md`: remove "Sim-mode" reference

---

## Files

```
rsx-playground/server.py         — remove sim + stress refactor
rsx-playground/stress.py         — NEW entry point
rsx-playground/stress_client.py  — unchanged
rsx-playground/tests/*.py        — remove sim assertions
rsx-playground/tests/*.spec.ts   — remove sim assertions
rsx-sim/                         — DELETE
Cargo.toml                       — remove rsx-sim member
TODO.md, FEATURES.md, etc.       — update
```

---

## Acceptance Criteria

- No "simulated" responses from any endpoint
- Orders return error when gateway offline
- `cargo check` passes (rsx-sim removed)
- `pytest tests/api_e2e_test.py` passes
- stress appears in topology when running
- `/api/stress/start` spawns subprocess, logs latency
- `/api/stress/stop` kills subprocess cleanly
- Reports saved to `tmp/stress/` after each run
