# Trade UI — End-to-End Working State

## Goal

The `/trade/` SPA works correctly with live RSX processes running.
Every panel shows real data, no field shows `--` when a data source
exists, and the Playwright suite passes with zero failures.

## Stack

- **SPA**: React 19 + TypeScript, Vite, Tailwind, Zustand, lightweight-charts
- **Entry**: `rsx-webui/src/` — built to `rsx-webui/dist/`, served at `/trade/`
- **Server**: `rsx-playground/server.py` — proxies WS and REST to gateway
- **Protocol**: `specs/v1/WEBPROTO.md` (WS), `specs/v1/REST.md` (HTTP)
- **Tests**: `rsx-playground/tests/play_trade.spec.ts` (Playwright)

Read `rsx-webui/src/` fully before making any changes. All existing
patterns (Zustand stores, hooks, Tailwind classes, protocol frame shapes)
must be preserved. Do not add dependencies.

## Known Issues To Fix

### Critical — blocks basic use

**Symbols never load.** `server.py`'s `/v1/symbols` returns
`{"symbols": [...objects...]}` but the UI's `fetchSymbols()` in
`useRestApi.ts` expects `{"M": [[id, tick, lot, name], ...]}` per
`specs/v1/REST.md`. The TopBar symbol selector stays "Loading..."
permanently. Fix the server response shape to match the spec.

**RSI sub-chart never renders.** In `Chart.tsx`, the RSI
`createChart` useEffect runs once on mount before `rsiVisible` is
true, so `rsiContainerRef.current` is null. Fix the effect
dependencies so the RSI chart is created when `rsiVisible` first
becomes true and its container is in the DOM.

### Data not wired

**Funding rate always shows `--`.** `setFundingRate()` exists in
`market.ts` but is never called. The server already exposes
`GET /v1/funding?sym=&limit=1` — fetch the most recent record on
symbol load and call `setFundingRate()`. Update `nextFundingTs` to
the `ts` from that record plus 8 hours.

**Candle history starts empty.** The chart is built purely from the
live trade stream; on load or timeframe switch the chart is blank.
Add `GET /v1/candles?sym=&tf=&limit=200` to `server.py` that returns
OHLCV bars assembled from recent WAL trade events (or stubs with
synthetic data when WAL is unavailable). Fetch on symbol/timeframe
change in `Chart.tsx` and seed the series before live trades arrive.

**DepthChart implemented but unreachable.** `DepthChart.tsx` (426
lines, fully implemented) is never imported. Add a toggle button to
the chart panel in `TradeLayout.tsx` that switches between the candle
chart and the depth chart view.

## Acceptance Criteria

The end state is verified by running the Playwright suite against a
live server with all 5 RSX processes running:

```
cd rsx-playground && uv run python server.py &
# start RSX processes
cd tests && npx playwright test play_trade.spec.ts --reporter=list
```

All tests must pass **and** the following additional assertions must
hold (add them to `play_trade.spec.ts` if not already present):

1. **Symbols load**: after page load, symbol dropdown button does NOT
   show "Loading..." — it shows a real symbol name.
2. **Orderbook populates**: `#book` or the orderbook container shows
   at least one bid row and one ask row within 5 seconds.
3. **Chart renders candles**: the `<canvas>` inside the chart panel
   is non-empty (width > 0, height > 0) and at least one candle
   series data point exists after symbol loads.
4. **RSI visible**: clicking the RSI toggle shows the RSI sub-chart
   container with a `<canvas>` element.
5. **Funding rate not `--`**: the TopBar funding rate element does
   not contain `--` when `/v1/funding` returns data.
6. **Depth chart toggle**: clicking the depth chart toggle shows the
   SVG depth chart and hides the candle chart.
7. **Assets tab**: clicking the Assets tab shows the equity and
   balance rows (even if values are 0.00).

Zero failures. Zero `--` in fields that have a live data source.
