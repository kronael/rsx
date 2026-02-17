# RSX Playground Progress

updated: Feb 17
target: 223/223 Playwright tests passing

## Acceptance Proof

```
commit:    27f63c1
artifacts: 2026-02-17 15:02 UTC

GATE  STATUS   DETAIL
────  ───────  ──────────────────────────────────────────
1     PASS     server imports cleanly
2     PASS     all 13 pages + 38 HTMX partials HTTP 200
3     PASS     API suite 806/806 (0 failed)
4     PEND     Playwright shards not yet run (need Rust procs)

API BREAKDOWN (gate-3 by step):
  startup        1/1
  routing       89/89   (13 pages + 38 partials + no-abs-links)
  htmx-partials 419/419 (processes/risk/wal/orders/logs/verify)
  proxy          11/11
  spa-assets    196/196 (e2e + edge-cases)
  order-path     90/90

PLAYWRIGHT (gate-4):
  total:    0/223  (shards not run — Rust processes required)
  pass:     0
  fail:     0
  canonical_ok: false

FAILING IDs: none
ALL GREEN:   no  (gate-4 pending)
```

```
[████████████░░░░░░░░░░░░░░░░░░] 40%  89/223
```

## Test Inventory

| Suite | Count | Source |
|---|---|---|
| Playwright (current specs) | 214 | tests/play_*.spec.ts |
| Playwright (pending: proxy/trade) | 9 | to be written |
| **Acceptance target** | **223** | |

## Release-Critical Unblockers

These block green Playwright runs. Fix in order.

| # | Gate | Status | Playwright Impact |
|---|---|---|---|
| 1 | Server imports cleanly (`python3 -c "import server"`) | DONE | All |
| 2 | All 13 page routes return HTTP 200 | DONE | ~90 tests |
| 3 | All 39 `/x/*` HTMX partials return HTTP 200 | DONE | ~80 tests |
| 4 | `/v1/*` REST proxy → 502 when GW down (not 500) | DONE | 9 tests |
| 5 | `/ws/private`, `/ws/public` → 1013 when down (not crash) | DONE | 4 tests |
| 6 | Python API tests pass (`make api-unit`) | PENDING | gate for 7 |
| 7 | Playwright 223/223 green | PENDING | acceptance |

## Feature Tasks (non-blocking)

These improve UX but don't gate Playwright green.

| Task | Status | Notes |
|---|---|---|
| WAL dump tool (`rsx-cli`) | PENDING | rsx-dxs 97% |
| Risk replication/failover | PENDING | rsx-risk 95% |
| Orderbook snapshot save/load | PENDING | rsx-book 99% |
| Proxy contract tests (mock GW) | DONE | tests/api_proxy_test.py |
| Absolute link regression check | DONE | verified zero in server+pages |
| React.memo on leaf components | DONE | rsx-webui |
| Vite base="./" for /trade/ SPA | DONE | dist/index.html verified |

## Per-Spec Playwright Breakdown

| Spec | Tests | Status |
|---|---|---|
| play_overview.spec.ts | 15 | unknown |
| play_topology.spec.ts | 11 | unknown |
| play_book.spec.ts | 15 | unknown |
| play_risk.spec.ts | 18 | unknown |
| play_wal.spec.ts | 16 | unknown |
| play_logs.spec.ts | 13 | unknown |
| play_control.spec.ts | 15 | unknown |
| play_faults.spec.ts | 7 | unknown |
| play_verify.spec.ts | 14 | unknown |
| play_orders.spec.ts | 20 | unknown |
| play_navigation.spec.ts | 3 | unknown |
| play_trade.spec.ts | 67 | unknown |
| **subtotal** | **214** | |
| proxy/trade (TBD) | 9 | to write |
| **total** | **223** | |

## How to Run

```bash
# Gate 1-3: server health
python3 -c "import server"
curl -s localhost:49171/healthz

# Gate 4-5: proxy smoke
curl -s localhost:49171/v1/ping  # expect 502, not 500

# Gate 6: python API tests
cd rsx-playground && .venv/bin/pytest tests/api_unit -v --tb=short

# Gate 7: full Playwright
cd rsx-playground/tests && npx playwright test
```
