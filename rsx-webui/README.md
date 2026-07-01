# rsx-webui — DISCONTINUED

> **DISCONTINUED (2026-07-01).** The React trading GUI is no longer
> the trade surface. The primary trading client is now a **terminal
> UI (`rsx-tui`, ratatui)**. This SPA is frozen — no new features,
> bugfix-only until removed. The playground keeps a lighter,
> less-interactive web GUI for read-only demonstration; heavy
> interactive trading moves to the TUI. Do not build on this.

React trading UI for RSX. Vite + Tailwind, connects
to gateway via WebSocket.

## Running

```bash
bun install
bun run dev    # dev server at http://localhost:5173
bun run build  # production build to dist/
```

Production build served by playground at `/trade/`.

## Components

```
src/
  components/
    orderbook/    Orderbook ladder (bid/ask levels)
    order/        OrderEntry, OpenOrders, OrderHistory
    positions/    Open positions, funding, assets
    trades/       TradesTape, DepthChart
    chart/        Price chart
    layout/       TopBar, BottomTabs, TradeLayout
    Toast.tsx     Notification toasts
    ErrorBoundary.tsx
  hooks/
    usePublicWs   Public market data WebSocket
    usePrivateWs  Private order/fill WebSocket
    useRestApi    REST API calls
    useKeyboard   Keyboard shortcuts
    useSoundAlerts Audio feedback
  store/
    market.ts     Orderbook, BBO, trades state
    trading.ts    Orders, positions, fills state
    connection.ts WebSocket connection state
    settings.ts   User preferences
```

## WebSocket Protocol

- Public WS (`/ws/public`): orderbook, BBO, trades
- Private WS (`/ws/private`): orders, fills, positions
- Frame format: compact JSON `{N:[...]}`/`{U:[...]}`
- See specs/2/49-webproto.md

## See Also

- [rsx-gateway](../rsx-gateway/README.md) — WS backend
- [rsx-playground](../rsx-playground/README.md) — serves at /trade/
- [specs/2/49-webproto.md](../specs/2/49-webproto.md) — protocol
