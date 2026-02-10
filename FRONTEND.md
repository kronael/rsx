# Frontend Specification

Two frontends, two visual references:

- **React (Vite + shadcn + Tailwind)**: pixel-level replica of **Bybit** trading UI
- **Yew (Rust)**: pixel-level replica of **Hyperliquid** trading UI

Scope is **perpetuals trading only**. No auth, no onboarding, no KYC, no
wallet/deposit flows. **Global** market rules (full leverage). Web **desktop
and mobile**. Native mobile app is planned **v2** (defer).

---

## Shared Product Scope

- Perps only
- Global compliance assumptions
- Dark + light themes
- Desktop + mobile web layouts
- Real-time updates (orderbook, trades, positions, funding)

### Must-Have Screens (both frontends)

- Trade (main)
- Chart (embedded + full view)
- Orderbook
- Trades tape
- Positions
- Open orders
- Order history
- Funding/Fees
- Risk/Leverage settings

---

## Visual Reference Fidelity

These are **pixel-level replicas** in layout, spacing, sizing, and surface
hierarchy. Colors and typography must match the reference as closely as
possible. Branding may use placeholder name/logo, but the UI **look** must
match the reference.

- React frontend: Bybit web trading UI (desktop + mobile web)
- Yew frontend: Hyperliquid web trading UI (desktop + mobile web)

If a detail is ambiguous, default to the reference UI behavior and spacing.

---

## React Frontend (Bybit Replica)

### Tech

- Vite + React
- Tailwind CSS
- shadcn/ui components (customized to match Bybit)

### Layout (Desktop)

- Top global bar: symbol selector, price/24h stats, timeframe controls
- Left: orderbook + trades tape stack
- Center: chart (primary focus), with expandable full-screen
- Right: order entry (limit/market/conditional), leverage, margin mode
- Bottom: positions + open orders + order history tabs

### Layout (Mobile Web)

- Top: symbol selector + price stats
- Main: chart first
- Tabs/segments: orderbook, trades, order entry
- Bottom sheets for leverage/margin settings
- Positions and orders in separate tabbed screens

### Component Mapping

- Market selector
- Price stats strip
- Chart panel + indicators
- Orderbook depth (vertical split)
- Trades tape
- Order entry
- Positions table
- Orders table
- Funding/fees panel

---

## Yew Frontend (Hyperliquid Replica)

### Tech

- Yew
- Tailwind CSS (or utility-first equivalent), with custom theme tokens

### Layout (Desktop)

- Dense, grid-first layout
- Chart center, book + trades adjacent
- Order entry right
- Positions and orders docked bottom
- Compact typography and reduced padding vs Bybit

### Layout (Mobile Web)

- Single-column, high density
- Chart first, then orderbook + tape
- Order entry as modal/bottom drawer
- Positions/orders as tabbed list views

### Component Mapping

- Compact market list
- Minimalist stats row
- Orderbook + trades in stacked split
- Order entry (compact)
- Positions and orders list

---

## Themes

- Dark and light
- Must match reference contrast levels and surface hierarchy
- No alternate color themes (only reference-accurate)

---

## Data + Real-time Behavior

- Orderbook updates: 50-200ms cadence
- Trades tape: real-time
- Positions PnL: real-time
- Funding countdown timer
- Leverage/margin changes should preview liquidation price

---

## Accessibility

- Keyboard navigable order entry
- Clear focus states (match reference)
- Avoid tiny touch targets on mobile

---

## v2 (Deferred)

- Native mobile app
- Auth, onboarding, wallet/deposit flows
- Settings and profile management
