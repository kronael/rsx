# PLAN

## goal
Wire up latency measurement end-to-end in the playground and add a
Criterion regression gate script for developer-local use.

## approach
Three independent deliverables: a bash+jq bench-gate script that reads
Criterion JSON output and compares against a gitignored baseline; fixes
to the Playwright latency tests and a new pytest latency test; and a new
`/api/gateway-mode` endpoint in the FastAPI playground server with an
HTMX badge on the overview page. All three touch different files and can
be built in parallel.

## tasks
- [ ] Write `scripts/bench-gate.sh`: runs `cargo bench --workspace`,
  walks `target/criterion/*/new/estimates.json`, extracts
  `mean.point_estimate`, compares against `tmp/bench-baseline.json`,
  exits 1 if any benchmark >1.10x baseline, `--save-baseline` flag
  writes baseline, prints name/baseline/current/ratio/PASS/FAIL table.
  Add `bench-gate` and `bench-save` targets to Makefile.
- [ ] Fix `rsx-playground/tests/play_latency.spec.ts`: remove all
  skipped/vacuous tests, add three tests (latency after orders, risk-
  latency card renders, latency-regression card renders). Add
  `test_api_latency` to `rsx-playground/tests/api_e2e_test.py`.
- [ ] Add `GET /api/gateway-mode` to `rsx-playground/server.py` using
  existing `_probe_gateway_tcp()`, returning `{"mode": "live"|"offline",
  "url": "..."}`. Add HTMX badge to overview partial in `pages.py`.
  Add `test_api_gateway_mode` to `api_e2e_test.py`.
