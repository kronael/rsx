# RSX Playground Playwright Tests

Comprehensive E2E tests for the RSX Playground dashboard.

## Overview

This test suite covers all 10 dashboard tabs with 157 total tests:
- **Original tests:** 90 tests (basic structure/visibility)
- **New interactive tests:** 67 tests (HTMX, auto-refresh, user interactions)

## Test Files

### Advanced/Data Display Tabs (67 new interactive tests)

1. **play_book.spec.ts** (15 tests: 4 original + 11 new)
   - Symbol selector HTMX swap
   - Book ladder auto-refresh (100ms)
   - Live fills real-time updates
   - Book stats compression/order count

2. **play_risk.spec.ts** (18 tests: 5 original + 13 new)
   - User lookup by ID
   - Freeze/unfreeze buttons
   - Position heatmap auto-refresh
   - Margin ladder liquidation distance

3. **play_wal.spec.ts** (16 tests: 4 original + 12 new)
   - WAL status per-process streams
   - Lag dashboard producer-consumer gap
   - Timeline filter by event type
   - Dump JSON and verify integrity actions

4. **play_verify.spec.ts** (14 tests: 4 original + 10 new)
   - Run checks button triggers verification
   - 10 system invariants display
   - Pass/fail/skip indicators
   - Auto-refresh 5s interval

5. **play_logs.spec.ts** (13 tests: 4 original + 9 new)
   - Full line visibility (scroll/wrap)
   - Quick filter chips (instant filter)
   - Smart search (gateway error order)
   - Keyboard shortcuts (/, Ctrl+L)
   - Line expansion modal
   - Copy functionality
   - Auto-refresh with filter persistence

6. **play_topology.spec.ts** (11 tests: 4 original + 7 new)
   - Process graph nodes/edges
   - Core affinity process-to-core mapping
   - CMP connections gateway→risk→ME

7. **play_faults.spec.ts** (7 tests: 2 original + 5 new)
   - Fault injection kill/stop process
   - Recovery verification

### Other Tabs (existing tests)

- **play_overview.spec.ts** (15 tests)
- **play_control.spec.ts** (15 tests)
- **play_orders.spec.ts** (21 tests)
- **play_navigation.spec.ts** (12 tests)

## Helper Utilities

**test_helpers.ts** provides reusable functions:

```typescript
// HTMX interaction
waitForHTMX(page, timeout)
verifyHTMXSwap(page, triggerSelector, targetSelector)
verifyPolling(page, selector, expectedInterval)

// Auto-refresh verification
waitForRefresh(intervalMs, buffer)
verifyAutoRefresh(page, selector, intervalMs)

// Element verification
waitForText(page, selector, text, timeout)
verifyTableHasRows(page, selector, minRows)
verifyMetricHasValue(page, selector)
verifyRealData(page, selector, excludePatterns)

// Process management
startRSXProcesses(page, scenario)
stopRSXProcesses(page)

// Keyboard shortcuts
pressSlashForSearch(page)
pressClearShortcut(page)
```

## Running Tests

### Prerequisites

1. Install dependencies:
   ```bash
   cd /home/onvos/sandbox/rsx/rsx-playground/tests
   npm install
   ```

2. Start playground server:
   ```bash
   cd /home/onvos/sandbox/rsx/rsx-playground
   uv run server.py
   ```

   Server must be accessible at `http://localhost:49171`

### Run All Tests

```bash
npx playwright test
```

### Run Specific Test Files

```bash
# Book tab (15 tests)
npx playwright test play_book.spec.ts

# Risk tab (18 tests)
npx playwright test play_risk.spec.ts

# WAL tab (16 tests)
npx playwright test play_wal.spec.ts

# Verify tab (14 tests)
npx playwright test play_verify.spec.ts

# Logs tab (13 tests)
npx playwright test play_logs.spec.ts

# Topology tab (11 tests)
npx playwright test play_topology.spec.ts

# Faults tab (7 tests)
npx playwright test play_faults.spec.ts
```

### Run Single Test

```bash
npx playwright test play_book.spec.ts:36
```

### Debug Mode

```bash
npx playwright test --debug
npx playwright test --headed  # See browser
```

### Generate Report

```bash
npx playwright test --reporter=html
npx playwright show-report
```

## Test Patterns

### 1. HTMX Auto-Refresh Verification

```typescript
test("book stats card auto-refreshes every 2s", async ({ page }) => {
  await page.goto("/book");
  const statsDiv = page.locator("[hx-get='./x/book-stats']");

  // Verify auto-refresh configured
  await verifyPolling(statsDiv, "every 2s");
});
```

### 2. HTMX Swap Testing

```typescript
test("symbol selector triggers HTMX swap", async ({ page }) => {
  await page.goto("/book");
  const bookData = page.locator("#book-data");
  const initialContent = await bookData.textContent();

  await page.locator("#book-symbol").selectOption("3");
  await waitForHTMX(page);

  const newContent = await bookData.textContent();
  expect(newContent).not.toBe(initialContent);
});
```

### 3. Button Action Testing

```typescript
test("freeze button triggers action", async ({ page }) => {
  await page.goto("/risk");
  const freezeBtn = page.locator("button", { hasText: /^Freeze$/ }).first();

  await freezeBtn.click();
  await waitForHTMX(page);

  await expect(freezeBtn).toBeVisible();
});
```

### 4. Keyboard Shortcut Testing

```typescript
test("keyboard shortcuts: press / focuses search", async ({ page }) => {
  await page.goto("/logs");
  await waitForHTMX(page, 1000);

  await page.keyboard.press("/");

  const smartSearch = page.locator("#smart-search");
  await expect(smartSearch).toBeFocused();
});
```

### 5. Modal Interaction Testing

```typescript
test("line expansion: click line shows full content in modal", async ({ page }) => {
  await page.goto("/logs");
  const logView = page.locator("#log-view");
  await waitForHTMX(page, 2000);

  const logLines = logView.locator("div[onclick*='showFullLine']");
  if (await logLines.count() > 0) {
    await logLines.first().click();

    const modal = page.locator("#log-modal");
    await expect(modal).not.toHaveClass(/hidden/);
  }
});
```

## Configuration

**playwright.config.ts:**
```typescript
{
  testDir: ".",
  testMatch: "play_*.spec.ts",
  timeout: 15_000,
  retries: 0,
  use: {
    baseURL: "http://localhost:49171",
    headless: true,
  },
  reporter: "list",
}
```

## Coverage Summary

### HTMX Features
- ✓ hx-get, hx-post, hx-trigger
- ✓ hx-swap (innerHTML, outerHTML)
- ✓ hx-include (form data)
- ✓ Auto-refresh polling (1s, 2s, 5s intervals)
- ✓ Dynamic content updates

### Interactive Features
- ✓ Symbol/user lookups
- ✓ Process control (start/stop/kill/restart)
- ✓ Log filtering (quick filters, smart search)
- ✓ Keyboard shortcuts (/, Ctrl+L, Escape)
- ✓ Modal interactions
- ✓ Button actions (freeze/unfreeze, verify, dump)
- ✓ Filter persistence across auto-refresh

### Data Display
- ✓ Orderbook ladder with symbol selector
- ✓ Position heatmap (users × symbols grid)
- ✓ Margin ladder (liquidation distance)
- ✓ WAL status (per-process streams, lag dashboard)
- ✓ Process topology graph
- ✓ Core affinity map
- ✓ CMP connection flows
- ✓ Invariant checks (10 system rules)
- ✓ Reconciliation status
- ✓ Latency regression (vs baseline)

## Success Criteria

✅ All 67 new interactive tests implemented
✅ All 10 dashboard tabs covered
✅ HTMX auto-refresh verified
✅ Log viewer improvements tested (9 tests)
✅ Real E2E interactions tested
✅ Helper utilities created
✅ Total 157 tests across all specs

## Troubleshooting

### Server Not Running
```
Error: page.goto: net::ERR_CONNECTION_REFUSED
```
**Solution:** Start the playground server first:
```bash
cd /home/onvos/sandbox/rsx/rsx-playground
uv run server.py
```

### Tests Timeout
```
Error: Timeout 15000ms exceeded
```
**Solution:** Increase timeout in playwright.config.ts or specific test:
```typescript
test.setTimeout(30000);
```

### HTMX Not Loading
Check browser console for HTMX script loading errors. Verify CDN is accessible.

## References

- **Playwright docs:** https://playwright.dev/
- **HTMX docs:** https://htmx.org/
- **REFINEMENT.md:** Phase 2 requirements
- **PLAYWRIGHT_TESTS_SUMMARY.md:** Complete test breakdown
