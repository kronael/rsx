# PLAN

## goal
Remove fake sim order-matching from rsx-playground, delete the dead rsx-sim
Rust crate, and promote stress_client.py to a managed subprocess following
the existing maker pattern.

## approach
Work in two parallel streams: one cleans the Rust workspace (delete rsx-sim/,
update Cargo.toml/Makefile, verify cargo check), the other rewrites
rsx-playground (strip sim globals/functions/callsites from server.py, create
stress.py entry point, add subprocess management API following the maker
pattern). A final pass updates tests and docs once both streams land.

## tasks
- [ ] Delete rsx-sim/ Rust crate: remove the directory, workspace member
      entry in Cargo.toml, any Makefile targets referencing rsx-sim, and
      verify `cargo check` passes.
- [ ] Strip sim mode from server.py: delete _sim_book/_sim_wal_events/_sim_seq
      globals, _seed_sim_book()/_sim_submit() functions, and every callsite
      (lifespan startup, POST /api/orders/test fallback, GET /x/wal-events
      merge, x_book/x_book_stats/api_book fallbacks, GET /v1/orders sim read,
      insurance fund "simulated" hardcode, mark prices "sim" source). Endpoints
      return empty/error when gateway is offline — never simulated data.
- [ ] Replace in-process stress with subprocess management: delete
      STRESS_SCENARIOS, _run_scenario_loop(), stress_tasks,
      stress_scenario_metrics, scenario start/stop/status endpoints, and
      baseline auto-start from server.py. Create rsx-playground/stress.py
      (reads env vars RSX_STRESS_*, calls stress_client.run_stress_test,
      prints 5s stats, writes JSON report to REPORT_DIR/, exits 0/1 on p99
      threshold, handles SIGTERM). Add do_stress_start(), do_stress_stop(),
      _topo_stress(), and API routes POST /api/stress/start,
      POST /api/stress/stop, GET /api/stress/status to server.py following
      the existing maker subprocess pattern (PID file, pipe_output to
      log/stress.log).
- [ ] Clean up tests and docs: remove sim assertions and sim-only tests from
      tests/api_e2e_test.py, api_integration_test.py, api_orders_test.py,
      test_order_api.py, api_maker_test.py, play_safety.spec.ts,
      play_guarantees.spec.ts; add gateway-down → error/503 assertions where
      appropriate. Update TODO.md, FEATURES.md, PROGRESS.md, TESTING.md to
      reflect rsx-sim deletion and stress.py promotion.
