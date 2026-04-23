# PROJECT.md — Remove Sim Mode + Stress Generator

## Goal
Remove fake order-matching from rsx-playground, delete the dead rsx-sim Rust
crate, and promote stress_client.py to a managed subprocess following the
existing maker pattern.

## Stack
- Python 3, FastAPI (rsx-playground/server.py)
- Rust workspace (Cargo.toml, rsx-sim/ deletion)
- Playwright + pytest (test cleanup)

## Scope

### Delete
- `rsx-sim/` directory and workspace membership
- Globals: `_sim_book`, `_sim_wal_events`, `_sim_seq`
- Functions: `_seed_sim_book()`, `_sim_submit()`
- All sim callsites in: lifespan, POST /api/orders/test, GET /x/wal-events,
  x_book(), x_book_stats(), api_book(), GET /v1/orders, insurance fund
  fallback, mark prices "sim" source
- In-process stress: `STRESS_SCENARIOS`, `_run_scenario_loop()`,
  `stress_tasks`, `stress_scenario_metrics`, scenario endpoints,
  baseline auto-start

### Create
- `rsx-playground/stress.py` — standalone entry point; reads env vars,
  calls `stress_client.run_stress_test`, prints 5s stats, writes JSON
  report to `REPORT_DIR/`, exits 0/1 on p99 threshold, handles SIGTERM

### Modify
- `server.py`: add `do_stress_start()`, `do_stress_stop()`,
  `_topo_stress()`, API routes POST /api/stress/start,
  POST /api/stress/stop, GET /api/stress/status; follow maker subprocess
  pattern (PID file, pipe_output to log/stress.log)
- Tests: remove sim assertions, add gateway-down → error/503 assertions
- Docs: TODO.md, FEATURES.md, PROGRESS.md, TESTING.md

## IO Surfaces
- stress.py env vars: RSX_STRESS_GW_URL, RSX_STRESS_USERS,
  RSX_STRESS_RATE, RSX_STRESS_DURATION, RSX_STRESS_TARGET_P99,
  RSX_STRESS_REPORT_DIR
- Report output: `tmp/stress/<timestamp>.json`
- Log output: `log/stress.log`
- Subprocess management: PID file (follow maker pattern)

## Constraints
- stress_client.py unchanged
- /stress page kept, updated for subprocess status
- Stress report storage/list/view endpoints kept
- Gateway-down endpoints return empty or error — never "simulated" data

## Success Criteria
1. No endpoint returns "simulated" in any field
2. POST /api/orders/test returns error when gateway offline
3. `cargo check` passes with rsx-sim removed
4. `pytest tests/api_e2e_test.py` passes
5. POST /api/stress/start spawns subprocess visible in topology
6. GET /api/stress/status reflects running state
7. POST /api/stress/stop terminates process cleanly
8. JSON report written to tmp/stress/ after run completes
