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
- w2: Add `GET /api/gateway-mode` to `rsx-playground/server.py`
using the existing `_probe_gateway_tcp()` coroutine — return
`{"mode": "live" if reachable else "offline", "url": GATEWAY_URL}`. Then
add an HTMX badge to the overview partial in `rsx-playground/pages.py`:
`&lt;span hx-get="/api/gateway-mode" hx-trigger="load" hx-target="this"&gt;GW:
checking...&lt;/span&gt;` that renders green for live, amber for offline. Add
`test_api_gateway_mode` to `rsx-playground/tests/api_e2e_test.py`: GET
`/api/gateway-mode`, assert 200, `data["mode"] in ("live", "offline")`,
`"url" in data`. Acceptance: curl returns valid JSON, pytest test passes,
overview page renders badge without error.
