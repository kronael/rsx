# PROGRESS

updated: Feb 27 21:56:41  
phase: executing

```
[░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░] 0%  0/3
```

| | count |
|---|---|
| completed | 0 |
| running | 3 |
| pending | 0 |
| failed | 0 |

## workers

- w0: Write `scripts/bench-gate.sh`: pure bash + jq script
that (1) runs `cargo bench --workspace`, (2) walks
`target/criterion/*/new/estimates.json` and extracts `mean.point_estimate`
per benchmark, (3) loads `tmp/bench-baseline.json` if it exists, (4) prints
a table of name | baseline ns | current ns | ratio | PASS/FAIL, (5) exits 1
if any ratio > 1.10, (6) when `--save-baseline` flag is given (or no
baseline exists) writes `tmp/bench-baseline.json` and exits 0. Add
`bench-gate` and `bench-save` Makefile targets. Acceptance: `make bench-gate`
with no baseline saves and passes; `make bench-gate` with a baseline that
has a 20% regression exits 1; `make bench-save` overwrites baseline.
- w1: Fix `rsx-playground/tests/play_latency.spec.ts`: read
the file first, then remove every test that either skips on 404 or asserts
only `>= 0`. Replace with exactly three tests: (1) submit 5 orders via
`POST /api/orders/test` then `GET /api/latency` and assert `count >= 0`
and conditional `p50 > 0` when count > 0, (2) `GET /x/risk-latency` returns
200 and HTML contains "latency", (3) `GET /x/latency-regression` returns 200
and HTML contains "p50". Also add `test_api_latency` to
`rsx-playground/tests/api_e2e_test.py` that GETs `/api/latency` and asserts
status 200 and `"count"` key present with value `>= 0`. Acceptance: zero
skipped Playwright tests in this file, zero vacuous assertions, pytest passes.
## log

- 22:05 w1 fix play_latency.spec.ts: partial — three required tests added and `test_api_latency` added to pytest, but old tests were NOT removed; file still has 10 tests including multiple that skip on 404 (recent-orders, live-fills, wal-timeline), violating the "exactly three tests" and "zero skipped" acceptance criteria.
- 21:57 w0 bench-gate.sh: complete — `scripts/bench-gate.sh` created with all six required behaviors (run benchmarks, walk criterion results, compare baseline, print table, exit 1 on >10% regression, save on --save-baseline or missing baseline) and both Makefile targets added.

- 21:58 w2 gateway-mode: complete — `GET /api/gateway-mode` added to server.py returning `{"mode", "url"}`, HTMX badge in pages.py overview partial (uses `/x/gateway-mode` HTMX partial as designed), and `test_api_gateway_mode` in api_e2e_test.py with all three required assertions.
