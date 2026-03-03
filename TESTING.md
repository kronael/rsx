# Testing

## Test Suites

### Rust Unit Tests

- `cargo test --workspace` (~785 tests, <5s)
- Tests in dedicated `tests/` dir with `_test.rs` suffix
- 88 test files across 11 crates
- Per-crate: `cargo test -p rsx-book -- test_name`
- Per-file: `cargo test -p rsx-dxs --test wal_test`

### Python Tests

- `pytest rsx-playground/tests/` (1034 tests)
- 19 test files covering: pages, HTMX fragments, v1 API,
  order flow, WAL, risk, stress, maker, edge cases
- api_e2e_test.py: 87 tests (core API coverage)
- No running processes needed for most tests

### Playwright Browser Tests

- 398 tests across 22 specs (~60s)
- Requires running server + processes
- `cd rsx-playground && bunx playwright test`

### Make Targets

| Target | Command | What | Time |
|--------|---------|------|------|
| unit | `make test` | Rust unit tests | <5s |
| WAL | `make wal` | WAL correctness | <10s |
| e2e | `make e2e` | Rust E2E + Playwright | ~30s |
| integration | `make integration` | testcontainers (PG) | 1-5min |
| benchmarks | `make perf` | Criterion | varies |
| bench gate | `make bench-gate` | regression gate (10%) | varies |
| Playwright | `make play` | all browser tests | ~60s |

## Test Coverage Summary

| Suite | Files | Tests | Time |
|-------|-------|-------|------|
| Rust unit | 88 | ~785 | <5s |
| Python | 19 | 1034 | ~10s |
| Playwright | 22 | 398 | ~60s |
| **Total** | **129** | **~2217** | |

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

### Running Tests

```sh
cd rsx-playground

# Python tests (no processes needed)
pytest tests/ -v            # all 1034 tests
pytest tests/api_e2e_test.py -v  # 87 core API tests

# Live verification with processes
python3 server.py &
curl -X POST localhost:49171/api/processes/all/start \
  -H "X-Confirm: yes"
# Then check each screen group
```
