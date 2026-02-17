# Definition of Done

Each gate must be green before the next gate runs (enforced by Makefile dependencies).
A task marked "done" rolls back to "failed" if any gate it depends on regresses.

## Gate 1 — Startup/Imports

**Required test:** `python -c "import server; print('ok')"` exits 0

**Done when:**
- `make gate-1-startup` exits 0
- No ImportError, no SyntaxError, no top-level exception

**Rollback condition:** any subsequent `import server` fails

---

## Gate 2 — Routing / HTMX Partials

**Required test:** `pytest tests/test_htmx_partials.py`

**Done when:**
- `make gate-2-partials` exits 0
- All 13 page routes return HTTP 200
- All 38 HTMX partial routes (`/x/*`) return HTTP 200
- Zero 5xx in any response body or status

**Rollback condition:** gate-1 regresses, or any route drops below HTTP 200

---

## Gate 3 — API Test Suite

**Required tests** (all must pass, zero failures):
```
tests/api_processes_test.py
tests/api_risk_test.py
tests/api_wal_test.py
tests/api_logs_metrics_test.py
tests/api_verify_test.py
tests/api_orders_test.py
tests/api_edge_cases_test.py
```

**Done when:**
- `make gate-3-api` exits 0
- Zero pytest failures across all 7 files
- `track_5xx` conftest fixture reports zero 5xx interceptions
- Endpoint-class triage in terminal summary shows 0 failures per class

**Rollback condition:** gate-2 regresses, or any test that was passing begins failing

---

## Gate 4 — Playwright Suite (223/223)

**Required tests** (4 shards, all must exit 0):
```
shard-routing:  play_navigation.spec.ts + play_overview.spec.ts + play_topology.spec.ts
shard-htmx:     play_book.spec.ts + play_risk.spec.ts + play_wal.spec.ts
                + play_logs.spec.ts + play_faults.spec.ts + play_verify.spec.ts
shard-control:  play_control.spec.ts + play_orders.spec.ts
shard-trade:    play_trade.spec.ts
```

**Done when:**
- `make gate-4-playwright` exits 0 (all 223 tests pass)
- OR equivalently: all 4 shards exit 0 (`make shards`)
- `play-shard.sh` exits 0 for each shard (not 1 or 2)
- No test is marked "unexpected" in Playwright JSON output

**Rollback condition:** gate-3 regresses, or any previously passing test spec fails

---

## Per-Domain Task DoD

### Domain: routing

- **Tests:** play_navigation.spec.ts, play_overview.spec.ts, play_topology.spec.ts
- **Gate:** shard-routing exits 0
- **Artifacts:** `tmp/play-sig/routing.sig` absent (pass) or unchanged (no new failures)
- **Rollback:** navigation link is broken, or page load > 15s timeout

### Domain: htmx-partials

- **Tests:** play_book, play_risk, play_wal, play_logs, play_faults, play_verify specs
- **Gate:** shard-htmx exits 0
- **Artifacts:** `tmp/play-sig/htmx-partials.sig` absent
- **Rollback:** any partial returns 4xx/5xx, or HTMX swap target missing

### Domain: process-control

- **Tests:** play_control.spec.ts, play_orders.spec.ts
- **Gate:** shard-control exits 0
- **Artifacts:** `tmp/play-sig/process-control.sig` absent
- **Rollback:** process start/stop commands broken, or order forms missing

### Domain: trade-ui

- **Tests:** play_trade.spec.ts (React SPA)
- **Gate:** shard-trade exits 0
- **Artifacts:** `tmp/play-sig/trade-ui.sig` absent; `rsx-webui/dist/` present and up to date
- **Rollback:** `/trade/` returns non-200, or React app fails to mount

---

## Regression Rollback Protocol

1. Any gate regression (gate N fails after being green) → all tasks that
   have gate N as a dependency are immediately reset from "done" to "failed".
2. Failure signature in `tmp/play-sig/<shard>.sig` is authoritative — a shard
   is only "done" when `play-shard.sh` exits 0 (sig file is deleted on pass).
3. Gate order is enforced: `make gate` runs 1→2→3→4 and stops on first failure.
   Attempting gate-4-playwright without gate-3-api being green is blocked by
   Makefile dependency chain.
4. No task is marked "done" based on partial output or visual inspection alone —
   the corresponding `make gate-N-*` or `make shard-*` command must exit 0.
