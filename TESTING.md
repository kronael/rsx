# Testing

## Test Suites

### Rust Unit Tests

- `cargo test` (~570 tests, <5s)
- Tests in dedicated `tests/` dir with `_test.rs` suffix
- Per-crate: `cargo test -p rsx-book -- test_name`
- Per-file: `cargo test -p rsx-dxs --test wal_test`

### Python E2E Tests

- `pytest rsx-playground/tests/api_e2e_test.py` (85 tests)
- Covers: all 13 pages, 27 HTMX fragments, v1 API,
  order flow, WAL timeline, edge cases
- Sim-mode: no running processes needed

### Playwright Browser Tests

- 228 tests across 19 specs (~60s)
- Requires running server + processes
- `cd rsx-playground && npx playwright test`

### Make Targets

| Target | Command | What | Time |
|--------|---------|------|------|
| unit | `make test` | Rust unit tests | <5s |
| WAL | `make wal` | WAL correctness | <10s |
| e2e | `make e2e` | Rust E2E + Playwright | ~30s |
| integration | `make integration` | testcontainers (PG) | 1-5min |
| benchmarks | `make perf` | Criterion | varies |
| Playwright | `make play` | all 228 browser tests | ~60s |

## Playground Screen Verification (Feb 27)

### Methodology

- 4 parallel agents, each verifying a screen group
- curl-based HTTP status + content quality checks
- Live verification with 6 running RSX processes

### Screens Verified

| Group | Endpoints | Result |
|-------|-----------|--------|
| Overview + Topology | 8 | All 200, live data |
| WAL + Logs | 12 | All 200, events streaming |
| Book + Orders + Risk | 13 | All 200, maker data |
| Control + Faults + Verify | 13 | All 200, resources shown |

### Issues Found and Fixed (10 bugs)

1. WAL timeline only parsed 3/11 record types
2. WAL filter buttons cosmetic-only (no hx-include)
3. MARGIN_CHECK filter (non-existent type)
4. Filter key mismatch (type_name vs type)
5. Book ladder showed raw i64 prices
6. SYMBOL_CONFIG decimals mismatched server.py
7. Unknown symbols in book stats display
8. Book stats empty when WS snap empty
9. /v1/account returned raw i64
10. /v1/orders returned raw i64 prices

### Running Verification

```sh
cd rsx-playground

# Python e2e (no processes needed)
python3 -m pytest tests/api_e2e_test.py -v  # 85 tests

# Live verification with processes
python3 server.py &
curl -X POST localhost:49171/api/processes/all/start \
  -H "X-Confirm: yes"
# Then check each screen group
```
