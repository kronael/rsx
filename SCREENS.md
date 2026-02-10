# Screens Specification

This describes the required screens for **two frontends**:

- **React (Bybit replica)**
- **Yew (Hyperliquid replica)**

Perps-only, global market assumptions, desktop + mobile web.

---

## 1. Trade (Main)

### Desktop

**Bybit-style (React)**
- Top bar: symbol selector, last price, 24h change, high/low, volume
- Left column: orderbook + trades tape
- Center: chart panel with timeframe + indicators
- Right: order entry (limit/market/conditional)
- Bottom: positions + open orders + order history tabbed

**Hyperliquid-style (Yew)**
- Compact grid layout
- Chart center
- Book + tape adjacent
- Order entry right
- Positions + orders bottom

### Mobile Web

**Bybit-style (React)**
- Chart first
- Tabbed sections: Orderbook / Trades / Order Entry
- Bottom sheet for leverage + margin settings

**Hyperliquid-style (Yew)**
- Single column
- Compact chart
- Book + tape stacked
- Order entry in drawer

---

## 2. Chart (Full View)

- Full width/height chart
- Overlay controls: timeframe, indicators, drawing tools
- Toggle to collapse back into main trade screen

---

## 3. Orderbook

- Bids/asks with depth shading
- Mid-price line
- Click on price populates order entry

---

## 4. Trades Tape

- Real-time prints
- Side color (buy/sell)
- Size + price columns

---

## 5. Order Entry

- Tabs: Limit, Market, Conditional
- Size input + slider
- Leverage + margin mode
- Estimated liquidation price preview

---

## 6. Positions

- Open positions list/table
- PnL (unrealized + realized)
- Leverage, entry, mark, liquidation
- Close/Reduce actions

---

## 7. Open Orders

- Open order list
- Cancel single / cancel all

---

## 8. Order History

- Filled orders
- Time, price, size, fee

---

## 9. Funding / Fees

- Funding rate + countdown
- Fee tier summary (maker/taker)

---

## 10. Risk / Leverage Settings

- Leverage selector
- Margin mode toggle
- Liquidation preview

---

## Mobile Considerations

- All screens available via bottom nav
- Dense lists with horizontal scroll for tables
- Touch targets ≥44px

---

## v2 (Deferred)

- Native mobile app
- Auth, onboarding, wallet/deposit flows
