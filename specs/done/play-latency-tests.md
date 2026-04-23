---
status: shipped
---

# Playwright: Latency & Performance Tests

## Goal

Playwright tests that verify the playground server
responds within acceptable latency bounds and that
the latency monitoring UI works correctly.

## File

`rsx-playground/tests/play_latency.spec.ts` (new file)

## Tests (~15 tests)

### Endpoint Latency (7 tests)

1. **page load < 500ms for each tab**
   - Measure `page.goto()` duration for each of:
     overview, topology, book, risk, wal, orders, logs
   - Assert all < 500ms

2. **HTMX partial < 200ms**
   - Measure fetch time for `/x/book`, `/x/wal-detail`,
     `/x/processes`, `/x/risk-overview`
   - Assert all < 200ms

3. **order submission < 500ms (sim mode)**
   - POST `/api/orders/test` with timing
   - Assert response < 500ms (sim matching)

4. **10 concurrent orders < 2s total**
   - Fire 10 parallel POST requests
   - Assert all complete within 2s wall time

5. **API JSON endpoints < 100ms**
   - GET `/api/processes`, `/api/book/10`,
     `/api/sessions/status`, `/api/mark/prices`
   - Assert all < 100ms

6. **static assets cached (304)**
   - First load page (populates cache)
   - Second load: verify Tailwind CDN returns 304
     or is served from cache

7. **no endpoint returns > 1s**
   - Hit all known `/x/*` endpoints sequentially
   - Assert none takes > 1000ms

### Latency UI (5 tests)

8. **risk latency card shows histogram**
   - Navigate to /risk
   - Submit 5 orders
   - Verify `/x/risk-latency` shows latency data

9. **order latency percentiles shown**
   - Submit 20 orders
   - Check `/api/latency` returns p50/p99/p999

10. **latency regression chart renders**
    - Navigate to appropriate page
    - Verify latency regression div exists and loads

11. **pulse bar shows ord/s rate**
    - Submit 5 orders quickly
    - Verify pulse bar shows non-zero ord/s within 5s

12. **latency doesn't degrade under load**
    - Submit 50 orders in batch
    - Measure last 10 order latencies
    - Assert p99 < 2x p50 (no degradation)

### Memory/Stability (3 tests)

13. **recent_orders doesn't grow unbounded**
    - Submit 300 orders
    - GET `/api/orders/recent` → count <= 200

14. **recent_fills capped at 200**
    - Submit 150 crossing orders (generates fills)
    - GET `/x/live-fills` → verify table < 200 rows

15. **sim WAL events capped at 500**
    - Submit 300 orders
    - GET `/x/wal-timeline` → verify < 500 events

## Acceptance Criteria

- [ ] All 15 tests pass
- [ ] No test requires running exchange
- [ ] Latency thresholds are generous (2x typical)
- [ ] Tests document measured baselines in comments

## Constraints

- Use `performance.now()` or `Date.now()` for timing
- All thresholds are assertions, not just logs
- Tests must be independent (no ordering dependency)
- Tag: `[latency]` in test.describe
