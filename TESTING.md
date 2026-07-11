# Testing

## Test Suites

### Rust Unit Tests

- `cargo test --workspace -- --test-threads=1`
  → **878 passed, 0 failed** (<5s)
- 912 `#[test]` + `#[tokio::test]` attributes total; the
  delta is feature-gated and `#[ignore]` integration tests
- Tests in dedicated `tests/` dir with `_test.rs` suffix
- 88 test files across 12 Rust crates
- Per-crate: `cargo test -p rsx-book -- test_name`
- Per-file: `cargo test -p rsx-cast --test wal_test`

### Python Tests

- `pytest rsx-playground/tests/` (~930 + 13 tests)
- 19 test files covering: pages, HTMX fragments, v1 API,
  order flow, WAL, risk, stress, maker, edge cases
- api_e2e_test.py: 87 tests (core API coverage)
- No running processes needed for most tests

### Playwright Browser Tests

- Canonical: 421 / 424 passing across 23 specs
  (3 conditional skips), ~60s
- Requires running server + processes
- `make e2e` (or `cd rsx-playground && bunx playwright test`)

### Make Targets

| Target | Command | What | Time |
|--------|---------|------|------|
| unit | `make test` | Rust unit tests | <5s |
| WAL | `make wal` | WAL correctness | <10s |
| e2e | `make e2e` | Rust E2E + Playwright (421/424) | ~3min |
| integration | `make integration` | testcontainers (PG) | 1-5min |
| performance | `make perf` | Criterion characterization | varies |
| perf gate | `make perf-gate` | Criterion regression gate (10%) | varies |
| load latency | `make perf-load` | sustained GW→ME→GW measurement | varies |
| E2E perf gate | `make perf-e2e-gate` | full-route regression gate | varies |

`make gate` is the ordered Playground release check. `make ci` and
`make ci-full` are automation lanes; `make shards-gated` is an advanced
browser-debugging lane. Focused browser specs should be run directly with
`cd rsx-playground/tests && bunx playwright test <spec>`.

## Test Coverage Summary

| Suite | Files | Tests | Time |
|-------|-------|-------|------|
| Rust unit | 88 | 878 pass / 912 attrs | <5s |
| Python (playground) | 19 | ~930 | ~10s |
| Python (rsx-auth) | 2 | 13 | <1s |
| Playwright | 23 | 421 / 424 (3 skips) | ~60s |
| **Total** | **132** | **~2,242 runnable** | |

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
pytest tests/ -v            # all 1035 tests
pytest tests/api_e2e_test.py -v  # 87 core API tests

# Live verification with processes
python3 server.py &
curl -X POST localhost:49171/api/processes/all/start \
  -H "X-Confirm: yes"
# Then check each screen group
```
