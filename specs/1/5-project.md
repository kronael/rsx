---
status: shipped
---

# Market Maker Integration

## Goal
Make the market maker a first-class component in the playground test
suite: auto-verified on startup, covered by a dedicated Playwright
shard, and validated by live orderbook checks in trade-ui tests.

## Stack
- TypeScript / Playwright (test layer)
- Python 3 / FastAPI (server.py, market_maker.py)
- No new bun or pip dependencies

## IO Surfaces
| Endpoint | Method | Purpose |
|---|---|---|
| `/api/maker/status` | GET | `{"running": bool, "pid": int}` |
| `/api/maker/start` | POST | Idempotent start |
| `/api/maker/config` | PATCH | `{"mid_override": float}` |
| `/api/book/10` | GET | L2 snapshot, `best_bid`/`best_ask` fields |
| `/api/status` or `/api/processes` | GET | Adds `"maker"` key |

## Files Changed
- `rsx-playground/tests/global-setup.ts` — poll maker status + book seed
- `rsx-playground/tests/playwright.config.ts` — add `market-maker` shard
- `rsx-playground/tests/play_maker.spec.ts` — harden setup + 2 new tests
- `rsx-playground/tests/play_trade.spec.ts` — add Live Orderbook section
- `rsx-playground/server.py` — expose maker state in status endpoint

## Constraints
- No new dependencies (bun or pip)
- Subprocess management stays in server.py
- Polling timeouts: maker ready 15s, book seeded 8s, depth 4s wait
- 80 char line width, max 120
- Read each file fully before editing

## Success Criteria
1. `bunx playwright test` — all 5 shards run and pass
2. Maker starts within 10s of `POST /api/processes/all/start`
3. global-setup completes with `best_bid > 0` within 20s
4. play_trade.spec.ts Live Orderbook tests pass (bid < ask, price > 0)
5. play_maker.spec.ts: 6 tests pass (4 existing + 2 new)
6. `/api/status` returns `"maker": {"running": true, "pid": N, "levels": N}`
