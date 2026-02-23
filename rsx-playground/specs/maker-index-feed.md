# Maker: Index Price Feed

## Goal

Market maker should quote around the RSX mark/index
price (which aggregates Binance + other feeds) instead
of hardcoded defaults when exchange processes aren't
producing live BBO.

## Current State

`market_maker.py` price precedence:
1. `tmp/maker-config.json` mid_override (manual)
2. `RSX_MAKER_MID_OVERRIDE` env var
3. Live BBO from marketdata WS (`ws://localhost:8181`)
4. Hardcoded defaults: `{10: 50000, 1: 30000, ...}`

Problem: when exchange is starting up or marketdata WS
is down, maker uses stale hardcoded prices.

## Deliverable

Add a new price source tier between 3 and 4:

**3b. Poll `/api/mark/prices` every quote cycle**

- `market_maker.py`: add `_fetch_mark_prices()` method
- Polls `http://localhost:{port}/api/mark/prices`
- Parses `{prices: {sid: {mark, bid, ask}}}` response
- Uses `mark` value as mid when live BBO unavailable
- Falls back to hardcoded defaults only if mark API
  also returns empty
- Port comes from `RSX_PLAYGROUND_PORT` env (default 49171)

**Server-side: enhance `/api/mark/prices`**

- `server.py`: add `_book_snap` as additional source
  (sim book has seeded prices even offline)
- Return sim mid when WAL has no BBO data
- Add `source` field: "wal", "sim", or "live"

## Acceptance Criteria

- [ ] Maker quotes around mark price when MD WS is down
- [ ] Maker still prefers live BBO when MD WS is up
- [ ] Maker still respects mid_override (highest priority)
- [ ] `/api/mark/prices` returns prices even offline
      (from sim book seed)
- [ ] `api_maker_test.py`: test mark price fallback

## Constraints

- Don't change existing precedence (override > env > BBO)
- Mark poll is best-effort: timeout 1s, no crash on fail
- Keep maker <600 lines
