# RSX Playground — Coverage & End-State Specification

updated: Feb 19 2026

## What "done" means

The objective is: every page, every endpoint, and every user
flow works end-to-end with RSX processes running. Playwright
tests pass AND live data flows through the full pipeline.

---

## Current State vs Required End State

### Layer 1 — Static rendering (DONE)

All 13 pages return HTTP 200. All HTMX polling attributes are
configured. All asset refs use `./` prefix. No 500 errors.

| Check | State |
|---|---|
| All 13 page routes → 200 | DONE |
| All 40 `/x/*` partials → 200 | DONE |
| All 30 `/api/*` endpoints → 200/204 | DONE |
| No absolute hrefs (krons.cx proxy) | DONE |
| `/api/stress/run` → 502 when gateway down | DONE |
| `/trade/` SPA loads | DONE |
| 806 Python unit + API tests green | DONE |

### Layer 2 — Playwright (DONE, but shallow)

223 Playwright tests pass. However, all 223 tests run against
the playground server only — no RSX Rust processes are
started. Tests verify:

- Page headings visible
- Form fields present
- HTMX `hx-trigger` attributes set correctly
- Placeholder/empty-state text appears

Tests do NOT verify live data. Specific examples of
acceptance criteria that hide no-process state:

```
# play_book.spec.ts
await expect(bookData).toContainText(
  /no book data|waiting for orders|no processes running/i
);

# play_orders.spec.ts
# "Without gateway running, order is queued"
await expect(page.locator("#order-result"))
  .toContainText(/accepted|queued|order/);

# play_faults.spec.ts
# checks fault buttons exist, not that faults fire
```

| Spec file | Tests | Tests w/ live RSX required |
|---|---|---|
| play_navigation.spec.ts | 3 | 0 |
| play_overview.spec.ts | 15 | 0 |
| play_topology.spec.ts | 11 | 0 |
| play_book.spec.ts | 15 | 15 |
| play_risk.spec.ts | 18 | 18 |
| play_wal.spec.ts | 16 | 16 |
| play_logs.spec.ts | 13 | 5 |
| play_faults.spec.ts | 7 | 7 |
| play_verify.spec.ts | 14 | 14 |
| play_control.spec.ts | 15 | 8 |
| play_orders.spec.ts | 20 | 20 |
| play_trade.spec.ts | 67 | 67 |
| **play_stress.spec.ts** | **0** | **~20 needed** |
| **Total** | **214** | **170** |

### Layer 3 — Live end-to-end (NOT DONE)

No test or manual verification has confirmed that RSX
processes start, orders flow, fills appear, or the market
maker quotes. This is the entire objective.

---

## End-State Expectations by Page

### Overview (`/`)

**Required live state:**
- Process status shows running/stopped per process
- Uptime counters increment
- CMP message rates show non-zero values
- Key metrics panel shows order count, fill count, book depth

**Current state:** Placeholders. All counters zero or dashes.

---

### Topology (`/topology`)

**Required live state:**
- Process boxes show green (running) or red (stopped)
- CMP flow arrows show message rates between boxes
- Ring pressure gauges show queue depths

**Current state:** Static diagram. No live process data.

---

### Book (`/book`)

**Required live state:**
- Orderbook ladder shows bid/ask levels with price and qty
- Selecting a symbol shows that symbol's book
- Book stats show level count, spread, mid price
- Live fills show matched trades as they happen
- Trade aggregation shows volume candles

**Current state:** "no book data" placeholder. Ladder empty.

---

### Risk (`/risk`)

**Required live state:**
- Position heatmap shows user positions per symbol
- Margin ladder shows margin usage per user
- Funding panel shows current funding rate and countdown
- Risk latency panel shows tail latencies from risk engine

**Current state:** Empty heatmap, zero margin rows, no funding.

---

### WAL (`/wal`)

**Required live state:**
- WAL status shows active stream files and sizes
- WAL timeline shows record sequence numbers and types
- WAL lag shows replication offset vs writer tip
- WAL rotation shows segment rotation events

**Current state:** No WAL files exist. All panels empty.

---

### Logs (`/logs`)

**Required live state:**
- Log tail shows structured log lines from running processes
- Log filtering by process name works
- Error aggregation shows error counts by type
- Auth failure panel shows rejected auth attempts

**Current state:** Empty. No processes → no logs.

---

### Control (`/control`)

**Required live state:**
- Process grid shows start/stop/restart buttons per process
- Clicking Start → process starts, row turns green
- Clicking Stop → process stops, row turns red
- Scenario switch rebuilds and restarts processes
- Resource usage shows CPU/mem per process

**Current state:** Grid renders. Buttons exist. Clicking Start
fires `/api/processes/{name}/start` but binaries may not be
built. No verification that processes actually come up.

---

### Faults (`/faults`)

**Required live state:**
- Kill button kills a process → status changes to stopped
- Restart button restarts it → status changes to running
- Stale order panel shows orders open > threshold
- Ring pressure shows SPSC ring fill percentage

**Current state:** Buttons render. No processes to kill.

---

### Verify (`/verify`)

**Required live state:**
- Invariant status shows PASS/FAIL for each of 10 invariants
  (fills before ORDER_DONE, monotonic tips, no crossed book…)
- Reconciliation shows position = sum of fills per user
- Stale order count is 0 (or shows actual stale orders)

**Current state:** No data. Invariant checks return empty.

---

### Orders (`/orders`)

**Required live state:**
- Submit order → accepted by gateway → fill or queue
- Order appears in recent orders table
- OID trace shows lifecycle: submitted → routed → filled
- Cancel order → ORDER_DONE with cancel reason
- Batch 10 → 10 orders submitted, each gets ACK
- Stress 100 → 100 orders submitted in <2s

**Current state:** Orders submitted but "queued" (no gateway).
No OID trace possible. Cancel returns 404 (no order exists).

---

### Stress (`/stress`)

**Required live state:**
- Fill in rate + duration, click Run
- Progress shows during test (orders/s, latency)
- On completion: submitted count, accept rate %, p99 µs
- Report saved to disk and appears in reports table
- Historical reports are clickable and show detail
- **play_stress.spec.ts does not exist** — zero E2E coverage

**Current state:** Form renders. No gateway → 502. No reports.

---

### Trade (`/trade/`)

**Required live state:**
- React SPA loads and connects to WS gateway
- Order form submits → fill appears in trade history
- Orderbook panel shows live bids/asks
- Position panel shows current position and PnL

**Current state:** SPA loads. WS connection fails (no gateway).
All panels show empty/connecting state.

---

### Docs (`/docs`)

**Required live state:** Static. Renders spec markdown files.
**Current state:** DONE. No live data needed.

---

## Market Maker — End State

The market maker (`market_maker.py`) auto-starts 3s after
all RSX processes are running. In end state:

| Check | Required |
|---|---|
| Maker auto-starts after processes | Process startup → maker PID exists |
| Maker status card shows "running (pid X)" | `/api/maker/status` → running |
| Maker places quotes on BBO | Book shows 5 bid + 5 ask levels from user_id=99 |
| Maker cancels and re-quotes every 2s | Book refreshes every 2s |
| Stop maker → status changes | PID file gone, status = stopped |

**Current state:** Code is wired. Never run against live processes.

---

## Stress Test — End State

| Check | Required |
|---|---|
| Run stress (rate=100, dur=10) | Submits ~1000 orders |
| Accept rate > 90% | Gateway ACKs orders |
| p99 latency shown | Actual round-trip time |
| Report on disk | `tmp/stress-reports/stress-*.json` |
| Reports table updates | `/x/stress-reports-list` shows row |
| play_stress.spec.ts exists | ~20 tests covering full flow |

**Current state:** No reports exist. No Playwright spec. 0/5.

---

## What Must Be Done

### Phase A — Build & start RSX processes
1. `cargo build -p rsx-gateway -p rsx-risk -p rsx-matching ...`
2. `./start` brings all processes up
3. `/api/processes` shows all as running

### Phase B — Market maker live
1. Auto-starts after Phase A
2. Book page shows 10 levels (5+5 from maker)
3. Maker status card shows running

### Phase C — Order flow
1. Submit single order → accepted (not queued)
2. OID trace shows fill event
3. Recent orders table shows the order

### Phase D — Stress test with live gateway
1. Run stress test 100 ord/s × 10s
2. Report saved to disk
3. Reports table shows new row

### Phase E — Playwright with live system
1. Write `play_stress.spec.ts` (~20 tests)
2. Update `play_orders.spec.ts` to assert `accepted` not
   `accepted|queued`
3. Update `play_book.spec.ts` to assert real bid/ask levels
4. All 243+ tests pass with `reuseExistingServer: false`
   (or with processes started in globalSetup)

---

## Summary

| Layer | Tests | Status |
|---|---|---|
| Routes + HTML structure | 223 Playwright | DONE |
| Python unit + API mocks | 806 pytest | DONE |
| Live RSX process startup | 0 | NOT DONE |
| Live order flow | 0 | NOT DONE |
| Live market maker | 0 | NOT DONE |
| Live stress test | 0 | NOT DONE |
| Playwright w/ live system | 0 of ~170 | NOT DONE |
| play_stress.spec.ts | 0 tests exist | NOT DONE |
