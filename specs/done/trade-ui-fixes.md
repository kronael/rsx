# Trade UI Fixes

## Goal

The `/trade/` React SPA shows live data for all panels. Read all
referenced source files fully before editing anything.

## Stack

- SPA: `rsx-webui/src/` (React 19 + TypeScript + Vite + Tailwind +
  Zustand + lightweight-charts)
- Server: `rsx-playground/server.py` (FastAPI, proxies to gateway)
- Tests: `rsx-playground/tests/play_trade.spec.ts` (Playwright)

## Fix 1 — Symbols Load

**Files:** `rsx-webui/src/hooks/useRestApi.ts`,
`rsx-playground/server.py`

Read `useRestApi.ts` `fetchSymbols()`. Confirm what format it expects
from `/v1/symbols`. Read `server.py` route for `/v1/symbols` and
confirm what it returns. If there is a mismatch, fix the server to
return the format the hook expects. If the server already returns the
correct format (`{"M": [[id, tick, lot, name], ...]}`) and the hook
expects that, then verify the TopBar.tsx `useEffect` that calls
`fetchSymbols` is correctly wired to the Zustand store. Run a quick
Playwright test to confirm symbols appear in the dropdown. If they
already do, mark as verified (no change needed).

## Fix 2 — RSI Sub-chart Renders

**File:** `rsx-webui/src/components/chart/Chart.tsx`

**Problem:** The RSI container `div` is rendered conditionally on
`{rsiVisible && ...}`. The `createChart` call for the RSI panel runs
in a `useEffect` with `[rsiVisible]` as dependency. When `rsiVisible`
becomes `true` the effect runs, but if the `rsiContainerRef.current`
is `null` at that point (DOM not yet attached), the chart is not
created.

**Fix:** In the RSI `useEffect`, guard with `if (!rsiContainerRef.current) return;`
and also add `rsiContainerRef` to the dependency array if missing.
Alternatively, use a `useLayoutEffect` so the DOM is guaranteed to be
attached before the effect fires.

Read Chart.tsx fully (it is ~900 lines). Find the RSI useEffect and
fix the guard. Do not break the existing EMA/candlestick chart logic.

## Fix 3 — Funding Rate Populated

**Files:** `rsx-webui/src/components/layout/TopBar.tsx`,
`rsx-playground/server.py`

Read `TopBar.tsx` fully. Find where `fetchFunding()` is called and
where `setFundingRate` is called. If `fetchFunding()` is already
called but the server's `/v1/funding` endpoint does not exist or
returns an error, add the endpoint to `server.py`.

The endpoint should return recent funding records for the selected
symbol. Parse from the WAL using `parse_wal_records` with
`RECORD_FUNDING` type (if it exists) or return a synthetic rate of
0.01% if no WAL data. Return format: `[{"ts": ..., "rate": ...}]`.

If `fetchFunding()` is already called and the endpoint exists but
`setFundingRate` is never called with the result, wire the result to
the store.

## Fix 4 — Candle History

**Files:** `rsx-webui/src/components/chart/Chart.tsx`,
`rsx-playground/server.py`

Read Chart.tsx lines 620-680 where candles are seeded from REST on
symbol/timeframe change. Confirm it calls `fetchCandles(sym, tf,
200)`. Confirm `server.py` has `GET /v1/candles?sym=&tf=&limit=`
endpoint. If the endpoint exists and returns OHLCV data, the chart
should populate. If the endpoint is missing or broken, fix it:

- Parse WAL fill records, aggregate into OHLCV bars by timeframe
- Timeframe strings: `1m`, `5m`, `15m`, `1h`, `4h`, `1D`
- Return: `[{"t": unix_sec, "o": px, "h": px, "l": px, "c": px, "v": qty}]`
- If no WAL fills, return synthetic stubs (3-5 bars) so chart is
  never blank

## Fix 5 — DepthChart Toggle

**Files:** `rsx-webui/src/components/layout/TradeLayout.tsx`,
`rsx-webui/src/components/chart/DepthChart.tsx`

**Problem:** `DepthChart.tsx` is fully implemented but never imported
or rendered. No toggle button exists.

**Fix:**
1. Import `DepthChart` in `TradeLayout.tsx`.
2. Add a boolean state `showDepth` (default `false`).
3. Add a small toggle button in the chart header area (e.g., "Depth"
   text button that toggles between candlestick and depth view).
4. When `showDepth=true`, render `<DepthChart />` instead of
   `<Chart />` in the chart panel.

Do not change `DepthChart.tsx` itself. Do not add bun dependencies.
Use existing Tailwind classes for the button.

## Acceptance Criteria

1. `cd rsx-webui && bun run build` — zero TypeScript errors.
2. Navigate to `/trade/` in Playwright; within 5s:
   - Symbol dropdown shows at least 1 symbol (e.g., "PENGU")
   - Orderbook panel shows bid/ask rows (may be empty if no maker)
   - Chart panel renders (candle area or depth area visible, not blank)
3. Click RSI toggle button; RSI sub-chart appears below main chart.
4. Funding rate field shows a numeric value (not `--`) within 5s.
5. Click "Depth" toggle; DepthChart renders in the chart panel area.
6. `bunx playwright test play_trade.spec.ts` — existing tests still
   pass (no regressions).

## Constraints

- Do NOT add new bun dependencies.
- Do NOT change the Zustand store shape — only wire missing calls.
- All Tailwind classes must use existing palette
  (`bg-surface`, `text-secondary`, `buy`, `sell`, etc.).
- 80 char line width, max 120.
- Read each file fully before editing.
