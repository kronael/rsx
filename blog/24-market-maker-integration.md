# Market Maker Full Integration

The playground now auto-starts a Python market maker as a subprocess,
continuously quotes all configured symbols, and the whole thing is
verifiable end-to-end via Playwright.

## What was wired

**`market_maker.py`** already had spread/depth/refresh logic. Added
`RSX_MAKER_MID_OVERRIDE` env var: when set (integer, raw units), the maker
skips the marketdata WS subscription entirely and uses that value as mid for
all symbols. Also polls `tmp/maker-config.json` each cycle for a runtime
`mid_override` key — lets tests shift the price without restarting the maker.

**`server.py`** got `PATCH /api/maker/config {"mid_override": <int>}`. Writes
`tmp/maker-config.json` atomically via `.tmp` rename. Returns `{"ok": true}`.
Also accepts float (isinstance check covers int|float) — a type mismatch
that would have silently discarded the override.

**`pages.py`** book rows already had `data-testid="bid-row"` /
`data-testid="ask-row"` and `data-px="{price}"` attributes. Confirmed
present; no change needed.

**`rsx-webui/src/`** BBO component already had `data-testid="bbo-bid"` /
`data-testid="bbo-ask"`. Confirmed.

## Test suite (`play_maker.spec.ts`)

Four tests, all using `mid_override=50000` via config patch — no live
marketdata required, but the full Rust stack (gateway, risk, matching)
runs live.

- **Book populated:** `/api/book/10` returns ≥ 1 bid and ≥ 1 ask level
  with best_bid < best_ask after maker starts.
- **Stop/start cycle:** stop → poll `active_orders==0` from
  `/api/maker/status` (not book depth — that's a WAL BBO max of 1 level,
  not 2); restart → levels repopulate within 8s.
- **Cross fill:** user 1 sends bid at `best_ask+1` via Node WS client with
  `x-user-id` header, side=0; within 5s both user 1 and maker (user 99)
  WS receive `F` in their frame stream.
- **Price movement:** patch `mid_override=51000`, poll `/api/bbo/10` for
  price change within one refresh cycle (2s, polling to 4s).

`play_trade.spec.ts` extended with a BBO live data test: sets
`mid_override=50000`, asserts `data-testid="bbo-bid"` and
`data-testid="bbo-ask"` show values > 0 and ask > bid within 10s.

## Bugs found during integration

- `setupMaker` was polling for ≥ 2 book levels each side; WAL BBO delivers
  at most 1 level per side (no live marketdata WS). Fixed to ≥ 1.
- Stop/start test was polling `book.bids.length === 0` for clean state;
  that races against WAL BBO replay. Fixed to poll `active_orders == 0`
  from the maker status endpoint.
- `_quote_cycle` drains at most 2 gateway responses per order pair with a
  0.2s timeout but places 10 orders per symbol (5 bids + 5 asks). Rejected
  order acknowledgements are silently dropped; the rejected order IDs persist
  in `active_cids`, causing the next cycle's cancel loop to send invalid
  cancel frames for orders the exchange has already rejected. Gateway error
  responses are never observed. Stale `active_cids` accumulate until
  restart.
- Float `mid_override` from JSON was rejected by an isinstance check that
  only allowed int. Fixed to allow int|float.

## Stack at end state

```
make e2e   # all tests pass
```

- Maker starts within 5s of stack ready
- `tmp/maker-status.json`: `running=true`, `orders_placed >= 10`
- `/api/book/10`: ≥ 1 bid, ≥ 1 ask, best_bid < best_ask
- BBO changes as maker re-quotes every 2s
- Stop → `active_orders == 0` within 5s; restart → levels back within 8s
- Cross fill: both sides receive `F` + `U` within 5s
- `/trade/` BBO panel: bid > 0, ask > bid within 10s of load
