# PROGRESS

updated: Feb 27 15:18:58  
phase: executing

```
[███████░░░░░░░░░░░░░░░░░░░░░░░] 25%  1/4
```

| | count |
|---|---|
| completed | 1 |
| running | 3 |
| pending | 0 |
| failed | 0 |

## workers

- w1: Strip sim mode from rsx-playground/server.py. Delete globals _sim_book, _sim_wal_events, _sim_seq; functions _seed_sim_book() and _sim_submit(); and every callsite: _seed_sim_book() in lifespan startup, sim fallback in POST /api/orders/test (return error "gateway not running" instead), _sim_wal_events merge in GET /x/wal-events, _seed_sim_book fallbacks in x_book/x_book_stats/api_book, _sim_book read in GET /v1/orders, insurance fund "simulated" hardcoded fallback, and mark prices "sim" source fallback. No endpoint may return simulated data — all return real WAL/maker data or empty. Acceptance: grep -r "sim" server.py returns no sim-data references; POST /api/orders/test with gateway offline returns an error response.
- w2: Replace in-process stress with a managed subprocess in rsx-playground/server.py. Delete STRESS_SCENARIOS dict, _run_scenario_loop(), stress_tasks, stress_scenario_metrics globals, in-process scenario start/stop/status endpoints, and baseline auto-start from lifespan. Create rsx-playground/stress.py: reads env vars RSX_STRESS_GW_URL (default ws://localhost:8080), RSX_STRESS_USERS (10), RSX_STRESS_RATE (1000), RSX_STRESS_DURATION (60), RSX_STRESS_TARGET_P99 (50000), RSX_STRESS_REPORT_DIR (./tmp/stress); imports stress_client.run_stress_test; prints stats every 5s; writes final JSON report to REPORT_DIR/; exits 0 if p99 &lt; target else 1; handles SIGTERM by cancelling workers and writing partial report. Add to server.py: do_stress_start(cfg), do_stress_stop(), _topo_stress(), and routes POST /api/stress/start, POST /api/stress/stop, GET /api/stress/status — mirror the existing maker subprocess pattern exactly (PID file, pipe_output to log/stress.log, managed dict registration). Keep /stress page and report storage/list/view endpoints. Acceptance: POST /api/stress/start spawns a subprocess visible in topology; GET /api/stress/status reflects running state; POST /api/stress/stop terminates it cleanly; JSON report written to tmp/stress/ after run.
- w3: Clean up tests and docs. In rsx-playground/tests/: remove all assertions that accept "simulated" as a valid status; remove sim-only test cases from api_e2e_test.py, api_integration_test.py, api_orders_test.py, test_order_api.py, api_maker_test.py; remove sim assertions from play_safety.spec.ts and play_guarantees.spec.ts; add gateway-down → error/503 assertions to api_orders_test.py where the old sim fallback was tested. Update docs: remove rsx-sim from TODO.md and Makefile references, update FEATURES.md (rsx-sim → stress.py), update PROGRESS.md (rsx-sim deleted), remove "Sim-mode" from TESTING.md. Acceptance: `pytest tests/api_e2e_test.py` passes; no test file contains "simulated" as an accepted status string; docs reflect current state.

## log

- `15:16:24` done: committed rsx-sim deletion (4 files, +0/-53)
- `15:35` task: INCOMPLETE — mark prices endpoint still has a `"sim"` source fallback at line ~4914 (`# fall back to sim book for symbols without WAL data`, `"source": "sim"`); grep still returns sim-data references.
