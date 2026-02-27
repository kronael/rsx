# PROGRESS

updated: Feb 27 15:20:25  
phase: executing

```
[██████████████████████░░░░░░░░] 75%  3/4
```

| | count |
|---|---|
| completed | 3 |
| running | 1 |
| pending | 0 |
| failed | 0 |

## workers

- w2: Replace in-process stress with a managed subprocess in rsx-playground/server.py. Delete STRESS_SCENARIOS dict, _run_scenario_loop(), stress_tasks, stress_scenario_metrics globals, in-process scenario start/stop/status endpoints, and baseline auto-start from lifespan. Create rsx-playground/stress.py: reads env vars RSX_STRESS_GW_URL (default ws://localhost:8080), RSX_STRESS_USERS (10), RSX_STRESS_RATE (1000), RSX_STRESS_DURATION (60), RSX_STRESS_TARGET_P99 (50000), RSX_STRESS_REPORT_DIR (./tmp/stress); imports stress_client.run_stress_test; prints stats every 5s; writes final JSON report to REPORT_DIR/; exits 0 if p99 &lt; target else 1; handles SIGTERM by cancelling workers and writing partial report. Add to server.py: do_stress_start(cfg), do_stress_stop(), _topo_stress(), and routes POST /api/stress/start, POST /api/stress/stop, GET /api/stress/status — mirror the existing maker subprocess pattern exactly (PID file, pipe_output to log/stress.log, managed dict registration). Keep /stress page and report storage/list/view endpoints. Acceptance: POST /api/stress/start spawns a subprocess visible in topology; GET /api/stress/status reflects running state; POST /api/stress/stop terminates it cleanly; JSON report written to tmp/stress/ after run.

## log

- `15:16:24` done: committed rsx-sim deletion (4 files, +0/-53)
- `15:18:59` done: Strip sim mode from rsx-playground/server.py. Delete globals (22 files, +262/-555)
- `15:19:28` done: Clean up tests and docs. In rsx-playground/tests/: remove al (22 files, +380/-538)
- `15:20:20` judge skip: Clean up tests and docs. In rsx-playgrou
- `15:28` Replace in-process stress with managed subprocess: complete — stress.py created with all env vars/SIGTERM handling; server.py has do_stress_start/stop, _topo_stress, three /api/stress/* routes following maker pattern, old globals removed.
