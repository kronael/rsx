# SHIP-TRADE-UI.md

Ship spec for rsx-webui trade UI. Target: Bybit-grade
perpetual futures trading interface. RSX dashboard color
scheme. Ultrafast rendering.

## Current State

- React 19 + Vite 6 + Tailwind 3.4 + Zustand 5
- lightweight-charts 4.2 for candlesticks
- ~1.5k LOC across 26 files
- Dual WS (public market data, private fills/orders)
- 10-level orderbook, order entry (limit/market), positions,
  orders, fills, funding, 6-timeframe chart
- 223 Playwright e2e tests passing

## Color Scheme (RSX Dashboard)

Use the existing RSX palette from tailwind.config.ts.
Do NOT use Bybit's yellow (#F7A600) accent or their
exact colors. Keep the RSX identity.

```
bg-primary:   #0b0e11    (main background)
bg-surface:   #1e2329    (panels, cards)
bg-hover:     #2b3139    (hover states)
border:       #2b3139    (dividers)
border-light: #363c45    (subtle borders)
text-primary: #eaecef    (main text)
text-secondary: #848e9c  (muted text)
text-disabled:  #5e6673  (disabled)
buy:          #0ecb81    (green, long, bid)
sell:         #f6465d    (red, short, ask)
accent:       #fcd535    (yellow highlights)
font-sans:    Inter
font-mono:    JetBrains Mono
```

## Target Layout (Bybit-style Grid)

```
+------------------------------------------------------------------+
| TopBar: symbol | last price | 24h stats | connection status       |
+----------+---------------------------+---------------------------+
| Orderbook|        Chart              |   Order Entry             |
| 288px    |        flex-1             |   320px                   |
|          |                           |   margin mode             |
| asks     |   TradingView-style       |   leverage slider         |
| spread   |   candlestick + volume    |   limit/market/cond       |
| bids     |                           |   price + qty             |
|          |                           |   TP/SL (future)          |
|----------|                           |   buy/sell buttons        |
| Trades   |                           |   cost/margin preview     |
| tape     |                           |                           |
+----------+---------------------------+---------------------------+
| Bottom Tabs: Positions | Orders | History | Funding | Assets      |
| resizable height (default 256px, drag to resize)                  |
+------------------------------------------------------------------+
```

## Phase 1: Performance Foundation

Goal: make the UI render at 60fps even with 100+
orderbook updates/second.

### 1.1 React rendering optimization

- Use React.memo on ALL leaf components (Row, TradeRow,
  PositionRow, OrderRow, FillRow)
- useMemo for derived data (sorted asks, cumulative totals)
- useCallback for all event handlers
- Zustand selectors: subscribe to individual fields,
  not entire store slices. Use shallow equality.
- Split market store: separate orderbook selector from
  bbo selector from trades selector (avoid re-renders)

### 1.2 Orderbook virtualization

- Only render visible rows (currently 10 asks + 10 bids
  is fine, but depth bar animation must not trigger full
  re-render)
- CSS `will-change: width` on depth bars
- Use CSS transforms for depth bar width instead of
  inline style recalc
- Batch orderbook state updates: collect deltas within
  one animation frame, apply once

### 1.3 Trade tape optimization

- Use a ring buffer (fixed-size array) instead of
  array.unshift() which copies the entire array
- Only re-render new rows, not the entire list
- requestAnimationFrame throttle for rapid updates

### 1.4 Chart performance

- lightweight-charts is already efficient
- Add crosshair sync between chart and orderbook
- Historical candle loading from REST (currently only
  live trades)

## Phase 2: Bybit Visual Polish

Goal: match Bybit's professional look and feel while
keeping RSX colors.

### 2.1 TopBar enhancements

Current: symbol selector + BBO + connection status

Add:
- Last price with tick direction arrow (up/down)
- 24h change (%, absolute) with color
- 24h high, 24h low
- 24h volume
- Funding rate + countdown in header
- Mark price display
- Index price display (if available)

### 2.2 Orderbook improvements

Current: 10 levels, price/size/total columns

Add:
- Toggle between: both sides / bids only / asks only
- Grouping control (tick grouping: 0.01, 0.1, 1, 10)
- Count column (number of orders at each level)
- Last price centered between asks and bids with
  arrow indicator showing direction
- Depth visualization: horizontal bars from RIGHT edge
  (bids) and LEFT edge (asks) like Bybit
- Animated transitions on level changes (fade in/out)
- Row highlight flash on update

### 2.3 Order entry improvements

Current: limit/market, buy/sell, TIF, RO/PO

Add:
- Margin mode indicator (cross/isolated) — display only
  for now, RSX uses cross margin
- Leverage display (read from account)
- Order cost preview: "Cost ≈ X USDT", "Max ≈ Y"
- Available balance display above buttons
- Percentage slider with 25/50/75/100% buttons (exists
  but improve visual — Bybit uses a slider track)
- Buy/Sell as full-width stacked buttons (Bybit style)
- TP/SL inputs (placeholder — future feature)
- Confirmation modal option (toggle in settings)

### 2.4 Bottom tabs improvements

Current: 4 tabs (Positions, Orders, History, Funding)

Add:
- Assets tab (account balances, margin summary)
- Tab badges with count
- Resizable height (drag handle on border)
- PnL column with real-time updates (flash green/red)
- Unrealized PnL total in tab header
- Liquidation price column (already exists)
- ADL indicator column
- Close position with limit or market choice
- TP/SL display per position (future)

### 2.5 Trades tape improvements

Current: time, price, qty, side

Add:
- Aggregate mode (group by same price within 100ms)
- Size visualization (bar proportional to qty)
- Flash animation on new trade
- Separate large trades visually (>10x avg qty)

## Phase 3: Advanced Features

### 3.1 Depth chart

- Cumulative depth visualization below orderbook
  or as toggle overlay on main chart
- Bid/ask curves with fill
- Tooltips on hover showing price/total

### 3.2 Keyboard shortcuts

- B: focus buy, S: focus sell
- Escape: cancel current input
- Enter: submit order
- Up/Down arrows: adjust price by tick
- Shift+Up/Down: adjust price by 10 ticks

### 3.3 Sound alerts (optional)

- Trade fill sound
- Liquidation warning sound
- Toggle in settings

### 3.4 Chart enhancements

- Drawing tools (horizontal line, trend line)
- Indicators (EMA, Bollinger, RSI — if lightweight-
  charts supports or via plugin)
- Multiple timeframe comparison

## Performance Targets

- 60fps during rapid orderbook updates (100/s)
- <16ms React render cycle for orderbook update
- <50ms trade-to-screen latency (WS message → DOM)
- Bundle size <500KB gzipped
- First contentful paint <1s
- No layout shift after initial load

## Implementation Order

1. Performance foundation (Phase 1) — do first
2. TopBar enhancements (2.1)
3. Orderbook improvements (2.2)
4. Order entry improvements (2.3)
5. Bottom tabs improvements (2.4)
6. Trades tape improvements (2.5)
7. Depth chart (3.1)
8. Keyboard shortcuts (3.2)
9. Sound alerts (3.3)
10. Chart enhancements (3.4)

Use the visual agent extensively for steps 2-6.
Use the improve agent for step 1.

## File Structure (keep flat)

```
src/
  components/
    layout/TopBar.tsx        (enhance)
    layout/TradeLayout.tsx   (enhance)
    orderbook/Orderbook.tsx  (enhance)
    trades/TradesTape.tsx    (enhance)
    chart/Chart.tsx          (enhance)
    chart/DepthChart.tsx     (new)
    order/OrderEntry.tsx     (enhance)
    positions/BottomTabs.tsx (enhance)
    positions/Positions.tsx  (enhance)
    positions/OpenOrders.tsx (enhance)
    positions/OrderHistory.tsx (enhance)
    positions/Funding.tsx    (enhance)
    positions/Assets.tsx     (new)
    ErrorBoundary.tsx
    Toast.tsx
  hooks/
    useRestApi.ts
    usePublicWs.ts
    usePrivateWs.ts
    useKeyboard.ts           (new)
  store/
    connection.ts
    market.ts
    trading.ts
    settings.ts              (new — UI prefs)
  lib/
    protocol.ts
    types.ts
    format.ts
    toast.ts
    ring.ts                  (new — ring buffer)
```

## Testing

- All existing 223 Playwright tests must continue passing
- Add visual regression tests for orderbook, order entry
- Performance benchmark: measure render time per
  orderbook update in CI

## Non-Goals

- Mobile-first design (desktop primary, basic mobile)
- Authentication UI (handled elsewhere)
- Multi-chart / multi-symbol (single symbol focus)
- Social/chat features
- Copy trading UI
