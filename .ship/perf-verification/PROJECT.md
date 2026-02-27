# Performance Verification

## Goal
Wire up latency measurement end-to-end in the playground and add a
Criterion regression gate script.

## Stack
- Bash + jq: `scripts/bench-gate.sh`
- Python (FastAPI): `rsx-playground/server.py`, `pages.py`
- TypeScript (Playwright): `rsx-playground/tests/play_latency.spec.ts`
- Python pytest: `rsx-playground/tests/api_e2e_test.py`
- Rust Criterion: workspace benchmarks

## Deliverables

### 1. `scripts/bench-gate.sh`
- Runs `cargo bench --workspace`
- Walks `target/criterion/*/new/estimates.json`, extracts `mean.point_estimate`
- Compares against `tmp/bench-baseline.json`
- Exits 1 if any benchmark > 1.10x baseline
- `--save-baseline` flag writes baseline
- Prints table: name | baseline ns | current ns | ratio | PASS/FAIL
- No baseline on first run: save + pass

### 2. Playground latency tests
- Fix `play_latency.spec.ts`: remove skipped/vacuous tests, add 3 tests
  (latency after orders, risk-latency card renders, latency-regression card)
- Add `test_api_latency` to `api_e2e_test.py`

### 3. `GET /api/gateway-mode`
- Returns `{"mode": "live"|"offline", "url": "<GATEWAY_URL>"}`
- Uses existing `_probe_gateway_tcp()`
- Overview page HTMX badge: `GW: live` (green) or `GW: offline` (amber)
- Python test validates shape

## Files Changed
```
scripts/bench-gate.sh                        NEW
rsx-playground/server.py                     add /api/gateway-mode
rsx-playground/pages.py                      overview badge
rsx-playground/tests/api_e2e_test.py         add 2 tests
rsx-playground/tests/play_latency.spec.ts    replace 3 tests
Makefile                                     bench-gate, bench-save targets
tmp/bench-baseline.json                      gitignored, developer-local
```

## Success Criteria
1. `make bench-gate` saves baseline on first run, passes; fails on >10% regression
2. `make bench-save` overwrites baseline
3. `pytest tests/api_e2e_test.py` — all pass including latency + gateway-mode
4. `play_latency.spec.ts` — 0 skipped, 0 vacuous assertions
5. `GET /api/latency` and `GET /api/gateway-mode` return valid JSON
6. Overview page renders gateway mode badge
7. No existing tests broken
