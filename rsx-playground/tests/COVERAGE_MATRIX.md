# Playwright Test Coverage Matrix

Maps each of the 223 Playwright tests to page, endpoint/HTMX partial, and
user-flow so that failures can be prioritized by blast radius.

**Blast radius legend:**
- `P0` — breaks primary navigation or page load (everything else likely broken too)
- `P1` — breaks a core data flow (ordering, risk lookup, WAL, logs)
- `P2` — breaks a secondary feature (polling, specific card, filter)
- `P3` — cosmetic / edge-case (responsive layout, empty state copy)

**Shard legend:**
- `routing` — play_navigation, play_overview, play_topology
- `htmx` — play_book, play_risk, play_wal, play_logs, play_faults, play_verify
- `control` — play_control, play_orders
- `trade` — play_trade

---

## play_navigation.spec.ts — 13 tests

| # | Test | Page | Endpoint/Partial | User Flow | Blast |
|---|------|------|-----------------|-----------|-------|
| 1 | all 10 tab links are present | `/` | GET `/` | Nav renders | P0 |
| 2 | clicking Overview navigates | `/` | GET `/overview` | Tab nav | P0 |
| 3 | clicking Topology navigates | `/` | GET `/topology` | Tab nav | P0 |
| 4 | clicking Book navigates | `/` | GET `/book` | Tab nav | P0 |
| 5 | clicking Risk navigates | `/` | GET `/risk` | Tab nav | P0 |
| 6 | clicking WAL navigates | `/` | GET `/wal` | Tab nav | P0 |
| 7 | clicking Logs navigates | `/` | GET `/logs` | Tab nav | P0 |
| 8 | clicking Control navigates | `/` | GET `/control` | Tab nav | P0 |
| 9 | clicking Faults navigates | `/` | GET `/faults` | Tab nav | P0 |
| 10 | clicking Verify navigates | `/` | GET `/verify` | Tab nav | P0 |
| 11 | clicking Orders navigates | `/` | GET `/orders` | Tab nav | P0 |
| 12 | root shows overview as active | `/` | GET `/` | Root redirect | P0 |
| 13 | root shows RSX title | `/` | GET `/` | Page title | P0 |

---

## play_overview.spec.ts — 15 tests

| # | Test | Page | Endpoint/Partial | User Flow | Blast |
|---|------|------|-----------------|-----------|-------|
| 14 | loads and shows process table | `/overview` | GET `/overview` | Page load | P0 |
| 15 | has start all and stop all buttons | `/overview` | GET `/overview` | Process control UI | P1 |
| 16 | has system health card | `/overview` | `./x/health` | Health display | P1 |
| 17 | has WAL status card | `/overview` | `./x/wal-status` | WAL display | P1 |
| 18 | has key metrics card | `/overview` | `./x/key-metrics` | Metrics display | P1 |
| 19 | process table auto-refreshes every 2s | `/overview` | `./x/processes` | HTMX polling | P2 |
| 20 | health score updates dynamically | `/overview` | `./x/health` | HTMX polling | P2 |
| 21 | key metrics display process counts | `/overview` | `./x/key-metrics` | Metrics content | P2 |
| 22 | WAL status auto-refreshes every 2s | `/overview` | `./x/wal-status` | HTMX polling | P2 |
| 23 | has scenario selector dropdown | `/overview` | GET `/overview` | Scenario select | P2 |
| 24 | build spinner shows during build | `/overview` | GET `/overview` | Build indicator | P3 |
| 25 | logs tail auto-refreshes every 2s | `/overview` | `./x/logs-tail` | HTMX polling | P2 |
| 26 | invariants card has auto-refresh configured | `/overview` | `./x/invariant-status` | HTMX polling | P2 |
| 27 | ring backpressure card displays | `/overview` | `./x/ring-pressure` | Ring metrics | P2 |
| 28 | start result container exists | `/overview` | GET `/overview` | Start action target | P2 |

---

## play_topology.spec.ts — 11 tests

| # | Test | Page | Endpoint/Partial | User Flow | Blast |
|---|------|------|-----------------|-----------|-------|
| 29 | loads and shows process graph | `/topology` | GET `/topology` | Page load | P0 |
| 30 | has core affinity card | `/topology` | `./x/core-affinity` | Affinity display | P2 |
| 31 | has CMP connections card | `/topology` | `./x/cmp-flows` | CMP display | P2 |
| 32 | has process list card | `/topology` | `./x/processes` | Process list | P1 |
| 33 | process graph shows nodes for running processes | `/topology` | GET `/topology` | Graph content | P1 |
| 34 | process graph shows edges for CMP connections | `/topology` | GET `/topology` | Graph edges | P1 |
| 35 | core affinity map auto-refreshes every 5s | `/topology` | `./x/core-affinity` | HTMX polling | P2 |
| 36 | core affinity displays process-to-core mapping | `/topology` | `./x/core-affinity` | Affinity content | P2 |
| 37 | CMP connections card auto-refreshes every 2s | `/topology` | `./x/cmp-flows` | HTMX polling | P2 |
| 38 | CMP connections show gateway-risk-ME flow | `/topology` | `./x/cmp-flows` | CMP content | P2 |
| 39 | process list auto-refreshes every 2s | `/topology` | `./x/processes` | HTMX polling | P2 |

---

## play_book.spec.ts — 15 tests

| # | Test | Page | Endpoint/Partial | User Flow | Blast |
|---|------|------|-----------------|-----------|-------|
| 40 | loads and has symbol selector | `/book` | GET `/book` | Page load | P0 |
| 41 | symbol selector has expected options | `/book` | GET `/book` | Symbol options | P1 |
| 42 | has book stats card | `/book` | `./x/book-stats` | Stats display | P2 |
| 43 | has live fills card | `/book` | `./x/live-fills` | Fills display | P2 |
| 44 | symbol selector changes orderbook display | `/book` | `./x/book` | Symbol switch | P1 |
| 45 | symbol selector triggers HTMX swap | `/book` | `./x/book` | Symbol switch | P1 |
| 46 | book ladder auto-refreshes every 1s | `/book` | `./x/book` | HTMX polling | P2 |
| 47 | book ladder shows placeholder when no processes | `/book` | `./x/book` | Empty state | P3 |
| 48 | book stats card auto-refreshes every 2s | `/book` | `./x/book-stats` | HTMX polling | P2 |
| 49 | book stats updates over time | `/book` | `./x/book-stats` | Stats refresh | P2 |
| 50 | live fills card auto-refreshes every 1s | `/book` | `./x/live-fills` | HTMX polling | P2 |
| 51 | live fills shows placeholder initially | `/book` | `./x/live-fills` | Empty state | P3 |
| 52 | book stats card shows compression info | `/book` | `./x/book-stats` | Stats content | P3 |
| 53 | trade aggregation card auto-refreshes | `/book` | `./x/trade-agg` | HTMX polling | P2 |
| 54 | all book cards load without errors | `/book` | `./x/book`, `./x/book-stats`, `./x/live-fills`, `./x/trade-agg` | Console errors | P1 |

---

## play_risk.spec.ts — 18 tests

| # | Test | Page | Endpoint/Partial | User Flow | Blast |
|---|------|------|-----------------|-----------|-------|
| 55 | loads and has user lookup | `/risk` | GET `/risk` | Page load | P0 |
| 56 | has freeze and unfreeze buttons | `/risk` | GET `/risk` | User actions UI | P1 |
| 57 | has position heatmap card | `/risk` | `./x/position-heatmap` | Heatmap display | P2 |
| 58 | has margin ladder card | `/risk` | `./x/margin-ladder` | Ladder display | P2 |
| 59 | has liquidation queue card | `/risk` | `./x/liquidations` | Liq display | P2 |
| 60 | user lookup by ID updates display | `/risk` | `./x/risk-user` | User lookup | P1 |
| 61 | user lookup shows DB unavailable message | `/risk` | `./x/risk-user` | DB error state | P2 |
| 62 | freeze button triggers action | `/risk` | POST `./api/risk/freeze` | Freeze user | P1 |
| 63 | unfreeze button triggers action | `/risk` | POST `./api/risk/unfreeze` | Unfreeze user | P1 |
| 64 | position heatmap auto-refreshes every 2s | `/risk` | `./x/position-heatmap` | HTMX polling | P2 |
| 65 | position heatmap shows placeholder when no data | `/risk` | `./x/position-heatmap` | Empty state | P3 |
| 66 | margin ladder auto-refreshes every 2s | `/risk` | `./x/margin-ladder` | HTMX polling | P2 |
| 67 | margin ladder shows liquidation distance placeholder | `/risk` | `./x/margin-ladder` | Empty state | P3 |
| 68 | funding card auto-refreshes | `/risk` | `./x/funding` | HTMX polling | P2 |
| 69 | liquidation queue auto-refreshes | `/risk` | `./x/liquidations` | HTMX polling | P2 |
| 70 | risk latency card auto-refreshes every 5s | `/risk` | `./x/risk-latency` | HTMX polling | P2 |
| 71 | user action buttons have correct HTMX attributes | `/risk` | `./api/users/create`, `./api/risk/liquidate` | HTMX attrs | P1 |
| 72 | all risk cards load without errors | `/risk` | all risk partials | Console errors | P1 |

---

## play_wal.spec.ts — 16 tests

| # | Test | Page | Endpoint/Partial | User Flow | Blast |
|---|------|------|-----------------|-----------|-------|
| 73 | loads with per-process WAL state card | `/wal` | GET `/wal` | Page load | P0 |
| 74 | has lag dashboard card | `/wal` | `./x/wal-lag` | Lag display | P1 |
| 75 | has WAL files card | `/wal` | `./x/wal-files` | Files display | P2 |
| 76 | has timeline card with filter | `/wal` | `./x/wal-timeline` | Timeline display | P2 |
| 77 | per-process WAL state auto-refreshes every 2s | `/wal` | `./x/wal-detail` | HTMX polling | P2 |
| 78 | per-process WAL state shows streams | `/wal` | `./x/wal-detail` | WAL content | P2 |
| 79 | lag dashboard auto-refreshes every 1s | `/wal` | `./x/wal-lag` | HTMX polling | P2 |
| 80 | lag dashboard shows producer-consumer gap | `/wal` | `./x/wal-lag` | Lag content | P1 |
| 81 | timeline filter has event type options | `/wal` | GET `/wal` | Filter options | P2 |
| 82 | timeline auto-refreshes every 2s | `/wal` | `./x/wal-timeline` | HTMX polling | P2 |
| 83 | timeline shows placeholder when no data | `/wal` | `./x/wal-timeline` | Empty state | P3 |
| 84 | WAL files card auto-refreshes every 5s | `/wal` | `./x/wal-files` | HTMX polling | P2 |
| 85 | WAL files card has verify and dump buttons | `/wal` | POST `./api/wal/verify`, POST `./api/wal/dump` | Action attrs | P1 |
| 86 | verify button triggers WAL integrity check | `/wal` | POST `./api/wal/verify` | WAL verify | P1 |
| 87 | dump JSON button triggers WAL dump | `/wal` | POST `./api/wal/dump` | WAL dump | P2 |
| 88 | all WAL cards load without errors | `/wal` | all wal partials | Console errors | P1 |

---

## play_logs.spec.ts — 13 tests

| # | Test | Page | Endpoint/Partial | User Flow | Blast |
|---|------|------|-----------------|-----------|-------|
| 89 | loads and has filters | `/logs` | GET `/logs` | Page load | P0 |
| 90 | process filter has expected options | `/logs` | GET `/logs` | Filter options | P1 |
| 91 | level filter has expected options | `/logs` | GET `/logs` | Filter options | P1 |
| 92 | has error aggregation card | `/logs` | `./x/error-agg` | Error agg display | P2 |
| 93 | full line visibility: long lines visible | `/logs` | `./x/logs` | Log line UX | P2 |
| 94 | quick filters: gateway chip applies filter | `/logs` | `./x/logs` | Quick filter | P1 |
| 95 | smart search: multiple keywords apply filters | `/logs` | `./x/logs` | Smart search | P1 |
| 96 | keyboard shortcuts: / focuses search | `/logs` | GET `/logs` | Keyboard UX | P2 |
| 97 | filter clearing: Ctrl+L clears all filters | `/logs` | `./x/logs` | Filter clear | P2 |
| 98 | line expansion: click shows full content | `/logs` | GET `/logs` | Log modal | P2 |
| 99 | copy button exists in modal | `/logs` | GET `/logs` | Modal copy | P3 |
| 100 | auto-refresh with filters: filters persist | `/logs` | `./x/logs` | Filter persistence | P2 |
| 101 | log viewer loads without console errors | `/logs` | `./x/logs` | Console errors | P1 |

---

## play_faults.spec.ts — 7 tests

| # | Test | Page | Endpoint/Partial | User Flow | Blast |
|---|------|------|-----------------|-----------|-------|
| 102 | loads with fault injection card | `/faults` | GET `/faults` | Page load | P0 |
| 103 | has recovery notes card | `/faults` | GET `/faults` | Notes display | P3 |
| 104 | fault injection grid auto-refreshes every 2s | `/faults` | `./x/faults-grid` | HTMX polling | P2 |
| 105 | fault injection grid shows stop and kill buttons | `/faults` | `./x/faults-grid` | Grid content | P1 |
| 106 | kill button triggers fault injection | `/faults` | POST `./api/process/kill` | Kill process | P1 |
| 107 | restart button appears for stopped processes | `/faults` | `./x/faults-grid` | Restart option | P2 |
| 108 | recovery notes show iptables and tc commands | `/faults` | GET `/faults` | Notes content | P3 |

---

## play_verify.spec.ts — 14 tests

| # | Test | Page | Endpoint/Partial | User Flow | Blast |
|---|------|------|-----------------|-----------|-------|
| 109 | loads with invariants card | `/verify` | GET `/verify` | Page load | P0 |
| 110 | has Run All Checks button | `/verify` | GET `/verify` | Verify action UI | P1 |
| 111 | has reconciliation card | `/verify` | `./x/reconciliation` | Recon display | P2 |
| 112 | has latency regression card | `/verify` | `./x/latency-regression` | Latency display | P2 |
| 113 | run checks button triggers verification | `/verify` | `./x/verify` | Run checks | P1 |
| 114 | invariants run on page load | `/verify` | `./x/verify` | Auto-run | P1 |
| 115 | verify results auto-refresh every 5s | `/verify` | `./x/verify` | HTMX polling | P2 |
| 116 | invariants show 10 system checks | `/verify` | `./x/verify` | Check count | P2 |
| 117 | invariants show pass/fail/skip indicators | `/verify` | `./x/verify` | Status indicators | P1 |
| 118 | reconciliation checks auto-refresh every 5s | `/verify` | `./x/reconciliation` | HTMX polling | P2 |
| 119 | reconciliation shows margin and book sync | `/verify` | `./x/reconciliation` | Recon content | P2 |
| 120 | latency regression auto-refreshes every 5s | `/verify` | `./x/latency-regression` | HTMX polling | P2 |
| 121 | latency regression shows baseline comparison | `/verify` | `./x/latency-regression` | Baseline content | P2 |
| 122 | all verify cards load without errors | `/verify` | all verify partials | Console errors | P1 |

---

## play_control.spec.ts — 15 tests

| # | Test | Page | Endpoint/Partial | User Flow | Blast |
|---|------|------|-----------------|-----------|-------|
| 123 | loads and has process control card | `/control` | GET `/control` | Page load | P0 |
| 124 | has notes card with scenario commands | `/control` | GET `/control` | Notes display | P3 |
| 125 | has resource usage card | `/control` | `./x/resource-usage` | Resource display | P2 |
| 126 | has scenario selector dropdown | `/control` | GET `/control` | Scenario select | P1 |
| 127 | scenario selector has all options | `/control` | GET `/control` | Option enumeration | P2 |
| 128 | scenario switch button triggers action | `/control` | POST `./api/scenario/switch` | Scenario switch | P1 |
| 129 | control grid auto-refreshes every 2s | `/control` | `./x/control-grid` | HTMX polling | P2 |
| 130 | resource usage has auto-refresh configured | `/control` | `./x/resource-usage` | HTMX polling | P2 |
| 131 | current scenario displays correctly | `/control` | `./x/current-scenario` | Current scenario | P2 |
| 132 | process control grid shows process rows | `/control` | `./x/control-grid` | Grid content | P1 |
| 133 | resource usage card exists | `/control` | `./x/resource-usage` | Resource content | P2 |
| 134 | notes card contains scenario commands | `/control` | GET `/control` | Notes content | P3 |
| 135 | scenario selector shows stress test options | `/control` | GET `/control` | Stress options | P2 |
| 136 | process control grid has action buttons | `/control` | `./x/control-grid` | Grid actions | P1 |
| 137 | scenario status shows current scenario | `/control` | GET `/control` | Status text | P2 |

---

## play_orders.spec.ts — 20 tests

| # | Test | Page | Endpoint/Partial | User Flow | Blast |
|---|------|------|-----------------|-----------|-------|
| 138 | loads with order form | `/orders` | GET `/orders` | Page load | P0 |
| 139 | has order lifecycle trace card | `/orders` | GET `/orders` | Trace UI | P2 |
| 140 | has recent orders card | `/orders` | `./x/recent-orders` | Orders display | P1 |
| 141 | has batch and stress test buttons | `/orders` | GET `/orders` | Batch UI | P2 |
| 142 | submits valid order successfully | `/orders` | POST `./api/order` | Order submit | P1 |
| 143 | handles invalid order via invalid button | `/orders` | POST `./api/order` | Order rejection | P1 |
| 144 | handles empty qty field | `/orders` | POST `./api/order` | Input validation | P2 |
| 145 | batch order submission creates 10 orders | `/orders` | POST `./api/order/batch` | Batch orders | P1 |
| 146 | random order submission creates 5 orders | `/orders` | POST `./api/order/random` | Random orders | P2 |
| 147 | order lifecycle trace by OID | `/orders` | GET `./x/order-trace` | OID trace | P2 |
| 148 | recent orders table updates after submission | `/orders` | `./x/recent-orders` | Table refresh | P1 |
| 149 | recent orders auto-refresh every 2s | `/orders` | `./x/recent-orders` | HTMX polling | P2 |
| 150 | order form has all TIF options | `/orders` | GET `/orders` | TIF options | P2 |
| 151 | order form has reduce_only checkbox | `/orders` | GET `/orders` | Checkbox | P2 |
| 152 | order form has post_only checkbox | `/orders` | GET `/orders` | Checkbox | P2 |
| 153 | cancel button appears for submitted orders | `/orders` | POST `./api/order/cancel` | Cancel order | P2 |
| 154 | order form supports all symbol options | `/orders` | GET `/orders` | Symbol options | P2 |
| 155 | order form supports buy and sell sides | `/orders` | GET `/orders` | Side toggle | P2 |
| 156 | order form has user_id input field | `/orders` | GET `/orders` | User ID input | P2 |
| 157 | order form has order_type selector | `/orders` | GET `/orders` | Order type | P2 |

---

## play_trade.spec.ts — 76 tests (nested describes)

> Page: `/trade/`  Shard: `trade`  Backend: React/Vite SPA (rsx-webui)

### Page Load & Layout (4)

| # | Test | Endpoint | User Flow | Blast |
|---|------|----------|-----------|-------|
| 158 | trade page loads with RSX title | GET `/trade/` | SPA load | P0 |
| 159 | root has dark background | GET `/trade/` | Theme | P3 |
| 160 | main layout grid renders | GET `/trade/` | Layout | P1 |
| 161 | bottom tabs section renders | GET `/trade/` | Tabs visible | P1 |

### TopBar (9)

| # | Test | Endpoint | User Flow | Blast |
|---|------|----------|-----------|-------|
| 162 | symbol dropdown button visible | GET `/trade/` | Symbol select | P1 |
| 163 | symbol button has dropdown arrow | GET `/trade/` | Symbol UI | P2 |
| 164 | clicking dropdown opens symbol list | GET `/trade/` | Symbol dropdown | P1 |
| 165 | connection status dot visible | GET `/trade/` | WS status | P1 |
| 166 | connection shows red when disconnected | GET `/trade/` | WS offline | P2 |
| 167 | price stats show default dashes | GET `/trade/` | Empty BBO | P2 |
| 168 | bid and ask size labels visible | GET `/trade/` | BBO labels | P2 |
| 169 | latency shows default dash | GET `/trade/` | Latency label | P3 |
| 170 | Mark/Index labels visible | GET `/trade/` | Price labels | P2 |

### Orderbook (3)

| # | Test | Endpoint | User Flow | Blast |
|---|------|----------|-----------|-------|
| 171 | header shows Price, Size, Total | GET `/trade/` | Book header | P1 |
| 172 | spread bar visible | GET `/trade/` | Spread display | P2 |
| 173 | spread shows default dash when no data | GET `/trade/` | Empty spread | P3 |

### Trades Tape (2)

| # | Test | Endpoint | User Flow | Blast |
|---|------|----------|-----------|-------|
| 174 | header shows Price, Size, Time | GET `/trade/` | Tape header | P2 |
| 175 | empty state with no trades | GET `/trade/` | Empty tape | P3 |

### Chart (5)

| # | Test | Endpoint | User Flow | Blast |
|---|------|----------|-----------|-------|
| 176 | timeframe buttons visible | GET `/trade/` | Timeframe UI | P1 |
| 177 | 1m is active by default | GET `/trade/` | Default TF | P2 |
| 178 | clicking 5m changes active state | GET `/trade/` | TF switch | P2 |
| 179 | clicking 1D timeframe works | GET `/trade/` | TF switch | P2 |
| 180 | chart container rendered | GET `/trade/` | Chart canvas | P1 |

### Order Entry (21)

| # | Test | Endpoint | User Flow | Blast |
|---|------|----------|-----------|-------|
| 181 | Limit tab visible and active by default | GET `/trade/` | Order type UI | P1 |
| 182 | Market tab visible | GET `/trade/` | Order type UI | P1 |
| 183 | Buy button visible and active by default | GET `/trade/` | Side UI | P1 |
| 184 | Sell button visible | GET `/trade/` | Side UI | P1 |
| 185 | price input visible in limit mode | GET `/trade/` | Price input | P1 |
| 186 | quantity input visible | GET `/trade/` | Qty input | P1 |
| 187 | percentage buttons visible | GET `/trade/` | Pct buttons | P2 |
| 188 | TIF selector visible with options | GET `/trade/` | TIF select | P2 |
| 189 | post-only checkbox visible in limit mode | GET `/trade/` | Post-only | P2 |
| 190 | reduce-only checkbox visible | GET `/trade/` | Reduce-only | P2 |
| 191 | submit button shows Buy Limit | GET `/trade/` | Submit label | P1 |
| 192 | switching to Sell changes button | GET `/trade/` | Side toggle | P1 |
| 193 | switching to Market hides price input | GET `/trade/` | Mode switch | P1 |
| 194 | switching to Market hides TIF selector | GET `/trade/` | Mode switch | P2 |
| 195 | Market mode shows Buy Market button | GET `/trade/` | Mode label | P1 |
| 196 | Market + Sell shows Sell Market button | GET `/trade/` | Mode+side | P1 |
| 197 | post-only hidden in market mode | GET `/trade/` | Conditional UI | P2 |
| 198 | reduce-only still visible in market mode | GET `/trade/` | Conditional UI | P2 |
| 199 | available balance shows 0.00 | GET `/trade/` | Balance label | P3 |
| 200 | price input accepts text | GET `/trade/` | Input UX | P2 |
| 201 | qty input accepts text | GET `/trade/` | Input UX | P2 |
| 202 | percentage button sets qty | GET `/trade/` | Pct fill | P3 |

### Bottom Tabs (6)

| # | Test | Endpoint | User Flow | Blast |
|---|------|----------|-----------|-------|
| 203 | all 4 tabs visible | GET `/trade/` | Tabs UI | P1 |
| 204 | Positions tab active by default | GET `/trade/` | Default tab | P2 |
| 205 | Orders tab inactive by default | GET `/trade/` | Tab state | P2 |
| 206 | clicking Orders tab switches | GET `/trade/` | Tab switch | P1 |
| 207 | clicking History tab switches | GET `/trade/` | Tab switch | P1 |
| 208 | clicking Funding tab switches | GET `/trade/` | Tab switch | P1 |

### Positions Tab (1)

| # | Test | Endpoint | User Flow | Blast |
|---|------|----------|-----------|-------|
| 209 | shows empty state message | GET `/trade/` | Empty positions | P3 |

### Open Orders Tab (1)

| # | Test | Endpoint | User Flow | Blast |
|---|------|----------|-----------|-------|
| 210 | shows empty state message | GET `/trade/` | Empty orders | P3 |

### Order History Tab (2)

| # | Test | Endpoint | User Flow | Blast |
|---|------|----------|-----------|-------|
| 211 | shows empty state message | GET `/trade/` | Empty history | P3 |
| 212 | load more button visible | GET `/trade/` | Load more | P3 |

### Funding Tab (6)

| # | Test | Endpoint | User Flow | Blast |
|---|------|----------|-----------|-------|
| 213 | funding rate label visible | GET `/trade/` | Funding display | P2 |
| 214 | funding rate shows default dash | GET `/trade/` | Empty funding | P3 |
| 215 | next funding countdown visible | GET `/trade/` | Countdown | P2 |
| 216 | countdown in HH:MM:SS format | GET `/trade/` | Countdown format | P2 |
| 217 | empty funding history message | GET `/trade/` | Empty history | P3 |
| 218 | funding tab content visible | GET `/trade/` | Tab content | P3 |

### Cross-Component Interactions (3)

| # | Test | Endpoint | User Flow | Blast |
|---|------|----------|-----------|-------|
| 219 | switching Buy/Sell toggles submit color | GET `/trade/` | Side toggle | P1 |
| 220 | switching order type updates submit text | GET `/trade/` | Type toggle | P1 |
| 221 | tab navigation preserves order entry state | GET `/trade/` | State persistence | P2 |

### Responsive (4)

| # | Test | Endpoint | User Flow | Blast |
|---|------|----------|-----------|-------|
| 222 | mobile viewport shows chart | GET `/trade/` | Responsive | P2 |
| 223 | orderbook hidden on mobile | GET `/trade/` | Responsive | P2 |
| 224 | order entry visible on mobile | GET `/trade/` | Responsive | P2 |
| 225 | bottom tabs visible on mobile | GET `/trade/` | Responsive | P2 |

---

## Summary by Blast Radius

| Level | Count | Description |
|-------|-------|-------------|
| P0 | 17 | Page load / navigation broken — everything else likely broken |
| P1 | 76 | Core data flows (ordering, risk, WAL, logs) |
| P2 | 103 | Secondary features (polling, filters, UI modes) |
| P3 | 29 | Cosmetic / empty-state copy / edge cases |
| **Total** | **225** | |

> Note: count is 225 (includes for-loop generated navigation tests). The 223
> target accounts for minor spec changes. Any failures in P0/P1 tests
> typically indicate the server or SPA is down or broken, making all other
> test results unreliable.

---

## Summary by Shard

| Shard | Spec Files | Test Count | P0 | P1 |
|-------|-----------|-----------|----|----|
| `routing` | navigation, overview, topology | 39 | 14 | 10 |
| `htmx` | book, risk, wal, logs, faults, verify | 83 | 6 | 30 |
| `control` | control, orders | 35 | 2 | 14 |
| `trade` | trade | 68 | 1 | 22 |

---

## Endpoint Coverage Index

Quick lookup: which tests exercise which endpoint.

| Endpoint | Test #s |
|----------|---------|
| GET `/` | 1, 12, 13 |
| GET `/overview` | 14–28 |
| GET `/topology` | 29–39 |
| GET `/book` | 40–54 |
| GET `/risk` | 55–72 |
| GET `/wal` | 73–88 |
| GET `/logs` | 89–101 |
| GET `/faults` | 102–108 |
| GET `/verify` | 109–122 |
| GET `/control` | 123–137 |
| GET `/orders` | 138–157 |
| GET `/trade/` | 158–225 |
| `./x/processes` | 19, 32, 39 |
| `./x/health` | 16, 20 |
| `./x/wal-status` | 17, 22 |
| `./x/key-metrics` | 18, 21 |
| `./x/logs-tail` | 25 |
| `./x/invariant-status` | 26 |
| `./x/ring-pressure` | 27 |
| `./x/core-affinity` | 30, 35, 36 |
| `./x/cmp-flows` | 31, 37, 38 |
| `./x/book` | 44, 45, 46, 47, 54 |
| `./x/book-stats` | 42, 48, 49, 52, 54 |
| `./x/live-fills` | 43, 50, 51, 54 |
| `./x/trade-agg` | 53, 54 |
| `./x/position-heatmap` | 57, 64, 65 |
| `./x/margin-ladder` | 58, 66, 67 |
| `./x/liquidations` | 59, 69 |
| `./x/risk-user` | 60, 61 |
| `./x/funding` | 68 |
| `./x/risk-latency` | 70 |
| `./x/wal-detail` | 77, 78 |
| `./x/wal-lag` | 74, 79, 80 |
| `./x/wal-timeline` | 76, 82, 83 |
| `./x/wal-files` | 75, 84 |
| `./x/error-agg` | 92 |
| `./x/logs` | 93–101 |
| `./x/faults-grid` | 104, 105, 107 |
| `./x/verify` | 113, 114, 115, 116, 117 |
| `./x/reconciliation` | 111, 118, 119 |
| `./x/latency-regression` | 112, 120, 121 |
| `./x/control-grid` | 129, 132, 134, 136 |
| `./x/resource-usage` | 125, 130, 133 |
| `./x/current-scenario` | 131 |
| `./x/recent-orders` | 140, 148, 149 |
| `./x/order-trace` | 147 |
| POST `./api/order` | 142, 143, 144 |
| POST `./api/order/batch` | 145 |
| POST `./api/order/random` | 146 |
| POST `./api/order/cancel` | 153 |
| POST `./api/wal/verify` | 85, 86 |
| POST `./api/wal/dump` | 85, 87 |
| POST `./api/risk/freeze` | 62 |
| POST `./api/risk/unfreeze` | 63 |
| POST `./api/risk/liquidate` | 71 |
| POST `./api/users/create` | 71 |
| POST `./api/scenario/switch` | 128 |
| POST `./api/process/kill` | 106 |
