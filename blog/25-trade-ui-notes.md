# Trade UI Notes: RSX WebUI

React 19 + Vite 6 + Tailwind 3.4 + Zustand 5. ~1.5k LOC, 26 files,
223 Playwright e2e tests. Dual WebSocket (public market data, private
fills/orders).

## Color Palette

RSX dashboard colors from `tailwind.config.ts`. Do not use Bybit's
yellow (#F7A600).

```
bg-primary:      #0b0e11   (main background)
bg-surface:      #1e2329   (panels, cards)
bg-hover:        #2b3139   (hover states)
border:          #2b3139   (dividers)
border-light:    #363c45   (subtle borders)
text-primary:    #eaecef   (main text)
text-secondary:  #848e9c   (muted text)
text-disabled:   #5e6673   (disabled)
buy:             #0ecb81   (green, long, bid)
sell:            #f6465d   (red, short, ask)
accent:          #fcd535   (yellow highlights)
font-sans:       Inter
font-mono:       JetBrains Mono
```

## Layout (Bybit-style grid)

```
+------------------------------------------------------------------+
| TopBar: symbol | last price | 24h stats | connection status       |
+----------+---------------------------+---------------------------+
| Orderbook|        Chart              |   Order Entry             |
| 288px    |        flex-1             |   320px                   |
|          |   lightweight-charts 4.2  |   margin/leverage         |
|          |   candlestick + volume    |   limit/market/cond       |
+----------+                           |   buy/sell buttons        |
| Trades   |                           |   cost/margin preview     |
| tape     |                           |                           |
+----------+---------------------------+---------------------------+
| Bottom Tabs: Positions | Orders | History | Funding | Assets      |
| resizable height (default 256px, drag handle)                     |
+------------------------------------------------------------------+
```

## Performance Targets

- 60 fps during 100+ orderbook updates/sec
- <16ms React render cycle per orderbook update
- <50ms trade-to-screen latency (WS message to DOM)
- <500KB gzipped bundle
- <1s first contentful paint

## Key Implementation Notes

**Rendering:**
- `React.memo` on all leaf components (Row, TradeRow, PositionRow, etc.)
- Zustand selectors: subscribe to individual fields with shallow equality
- Split market store: separate orderbook/bbo/trades selectors
- CSS `will-change: width` on depth bars; CSS transforms over inline style

**Trade tape:**
- Ring buffer (fixed-size array) instead of `array.unshift()` — avoids
  copying the entire array on every update
- `requestAnimationFrame` throttle for rapid updates

**Orderbook:**
- Batch state updates within one animation frame, apply once
- Depth bar animation must not trigger full re-render

## File Structure (flat)

```
src/
  components/
    layout/   TopBar.tsx, TradeLayout.tsx
    orderbook/ Orderbook.tsx
    trades/   TradesTape.tsx
    chart/    Chart.tsx, DepthChart.tsx
    order/    OrderEntry.tsx
    positions/ BottomTabs.tsx, Positions.tsx, OpenOrders.tsx,
               OrderHistory.tsx, Funding.tsx, Assets.tsx
  hooks/      useRestApi.ts, usePublicWs.ts, usePrivateWs.ts,
              useKeyboard.ts
  store/      connection.ts, market.ts, trading.ts, settings.ts
  lib/        protocol.ts, types.ts, format.ts, toast.ts, ring.ts
```

## Non-Goals

- Mobile-first (desktop primary)
- Multi-symbol / multi-chart view
- Social, copy trading, authentication UI

## See Also

- `rsx-webui/` — source
- `blog/19-playground-dashboard.md` — dev dashboard (HTMX, not React)
