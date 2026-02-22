# Market Maker Integration

## Goal

Make the market maker a first-class part of the playground server and
Playwright test suite. The maker runs automatically alongside RSX
processes and is verified before tests execute.

## Current State

- `server.py` auto-starts `market_maker.py` via `do_maker_start()` after
  RSX processes stabilize — works but no health check
- `play_maker.spec.ts` exists with 4 tests but is orphaned (not in any
  playwright shard)
- `global-setup.ts` waits 5s for maker to seed the book — no explicit
  confirmation that maker is running
- `play_trade.spec.ts` tests UI structure but not live orderbook data

## Changes

### 1. global-setup.ts — verify maker before tests

After the 5s sleep, poll `GET /api/maker/status` until
`{"running": true}` or 15s timeout. If maker fails to start, log a
warning but do not abort (some tests don't need maker).

Also poll `GET /api/book/10` until best_bid > 0 and best_ask > 0, or
8s timeout. This ensures the book is seeded before trade-ui tests run.

### 2. playwright.config.ts — add market-maker shard

Add a 5th project:

```typescript
{
  name: "market-maker",
  testMatch: ["play_maker.spec.ts"],
  use: { baseURL: "http://localhost:49171" },
}
```

Order: after `process-control`, before `trade-ui`. The market-maker
shard must run before trade-ui so the book is populated.

### 3. play_maker.spec.ts — harden and expand

Fix `setupMaker()` to:
- Explicitly call `POST /api/maker/start` even if already running (idempotent)
- Poll `/api/maker/status` with 15s timeout
- Poll `/api/book/10` for ≥1 bid + ≥1 ask with 15s timeout
- On timeout: throw with clear message

Add 2 new tests:

**Orderbook depth** — after maker runs 2 cycles (4s), book has ≥3 levels
each side.

**Mid override updates BBO** — PATCH `/api/maker/config` with new
mid_override, poll BBO for price shift within 6s, restore.

### 4. play_trade.spec.ts — live orderbook section

Add a new `describe("Live Orderbook", ...)` section (runs after
setupMaker). Tests:

- Orderbook panel shows ≥1 bid row with price > 0
- Orderbook panel shows ≥1 ask row
- Best bid < best ask (no crossed book)
- Mark price or last price visible and numeric

### 5. server.py — maker health in /api/status

Expose maker state in the existing `/api/status` or `/api/processes`
response so global-setup can poll one endpoint. Add `"maker"` key:

```json
{"maker": {"running": true, "pid": 12345, "levels": 5}}
```

Levels count comes from `tmp/maker-status.json` if present.

## Acceptance Criteria

1. `npx playwright test` — all 5 shards run; market-maker shard passes
2. After `POST /api/processes/all/start`, maker starts within 10s
   (verifiable via `/api/maker/status`)
3. global-setup completes with book seeded (best_bid > 0) within 20s
4. play_trade.spec.ts live orderbook tests pass when exchange is running
5. play_maker.spec.ts 6 tests (4 existing + 2 new) all pass

## Constraints

- No new npm dependencies
- Maker subprocess management stays in server.py (not global-setup)
- play_maker.spec.ts uses existing `setupMaker()` pattern
- 80 char line width, max 120
- Read each file fully before editing
