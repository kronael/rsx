---
status: shipped
---

# Playground Feature Audit & Fix Spec

Comprehensive audit of rsx-playground as a minimal viable
exchange demo. Every feature must either work end-to-end
or be removed. No half-baked stubs.

## Files

- `rsx-playground/server.py` (~6k lines)
- `rsx-playground/pages.py` (~4k lines)

---

## A. Broken Features (must fix)

### A1. Scenario selector doesn't propagate selection

**Where**: `pages.py` overview_page(), lines ~204-217

Radio buttons set `document.getElementById('scenario').value`
but the "Build & Start All" button reads it back wrong.
Radio `.value` returns the attribute, not the checked state.

**Fix**: Use `document.querySelector('input[name="scenario"]:checked').value`
in the onclick handler that posts to `./api/processes/all/start`.

### A2. WAL timeline filter is cosmetic-only

**Where**: `pages.py` wal_page(), lines ~726-776

Filter buttons set a hidden input value via JS, but the
HTMX `hx-get="./x/wal-timeline"` never sends it — no
`hx-include` or `hx-vals`.

**Fix**: Add `hx-include="#wal-filter"` to the timeline div,
and in `server.py` `/x/wal-timeline` read `request.query_params.get("filter")`
to filter records by type.

### A3. Risk page buttons hardcoded to user_id=1

**Where**: `pages.py` risk_page(), lines ~628-643

Deposit, freeze, unfreeze, liquidate buttons all hardcode
`/api/users/1/...` instead of reading the lookup input.

**Fix**: Wire buttons to read `document.getElementById('risk-uid').value`
and interpolate into the URL. Use `hx-vals` or JS onclick.

### A4. Book stats card ignores selected symbol

**Where**: `pages.py` book_page(), line ~556

`hx-get="./x/book-stats"` has no symbol_id param while
the book ladder correctly passes it.

**Fix**: Add `hx-include` for the symbol selector or use
the same JS trigger pattern as the book ladder.

### A5. /api/orders/batch never hits gateway

**Where**: `server.py` lines ~3961-3975

Generates 10 synthetic orders, appends to `recent_orders`
in-memory, never sends to gateway WS. User thinks orders
were placed but nothing happened.

**Fix**: Loop `_submit_single_order()` for each, same as
`/api/orders/test` does. Keep synthetic generation for
price/qty/side but actually submit.

### A6. /api/orders/random never hits gateway

**Where**: `server.py` lines ~3978-3993

Same problem as A5: generates 5 random orders, appends
to memory, never sends.

**Fix**: Same — actually submit via gateway WS.

### A7. /v1/fills returns hardcoded zero OIDs

**Where**: `server.py` lines ~5860-5871

```python
taker_hi = 0  # should extract from WAL fill record
taker_lo = 0
maker_hi = 0
maker_lo = 0
```

**Fix**: Extract `taker_oid_hi/lo` and `maker_oid_hi/lo`
from the WAL fill dict. If missing, fall back to 0.

---

## B. Fake Endpoints (make real or remove)

### B1. /api/orders/invalid — remove

Returns a canned rejected order. Only useful for tests.
Tests can use the real endpoint with bad params instead.

**Fix**: Remove endpoint. Update test to POST a real
invalid order (e.g. qty=0) to `/api/orders/test`.

### B2. /api/users/{user_id}/deposit — stub

Stores balance in a Python dict. Never touches Postgres
or risk engine. Misleading in a "working" demo.

**Fix**: If Postgres available, INSERT into balances table.
If not, keep in-memory but label response
`"source": "simulated"` so UI can show a badge.

### B3. /api/risk/liquidate — stub

Appends to `_liquidation_log` list. Never triggers
real liquidation engine.

**Fix**: Same approach — if risk process running, send
CMP message or WS command. Otherwise keep simulated
with clear label.

---

## C. Silent Fallbacks (make visible)

### C1. Book returns empty silently

`/api/book/{symbol_id}` falls through WS → WAL → maker
→ empty `{"bids":[], "asks":[]}` with no indication.

**Fix**: Add `"source"` field to response:
`"ws"`, `"wal"`, `"maker"`, or `"empty"`. UI can show
a subtle badge (e.g., "from WAL" or "no data").

### C2. Candles return synthetic data silently

`/v1/candles` generates fake OHLCV when no WAL fills.

**Fix**: Add `"synthetic": true` field when using
generated data. Trade UI can show "simulated" indicator.

### C3. Insurance fund hardcoded fallback

`/api/risk/insurance` returns seed data for symbols
[1,2,3,10] when Postgres unavailable.

**Fix**: Already has `"source": "simulated"` — just
ensure UI surfaces this (show badge on risk page).

### C4. Funding rate always synthetic

`/api/risk/funding` derives from BBO mid, not from
actual funding engine. `/v1/funding` returns 0.01%
when no data.

**Fix**: Label `"source": "derived"` or `"simulated"`.

---

## D. UI Polish (for working demo)

### D1. Remove stress button from orders page

The "Stress (100)" button on the orders page POSTs to
`/api/stress/run` which expects rate/duration params not
present in the form. It belongs on the stress page only.

**Fix**: Remove the button from `orders_page()`.

### D2. Hardcoded symbol lists

Symbols PENGU/DOGE/WIF/SOL are hardcoded in pages.py
in multiple places (book, orders, stress).

**Fix**: Add `/api/symbols` endpoint returning configured
symbols from `start` module's config. Pages fetch once
on load or use a shared constant.

### D3. Logs page smart search not wired

Smart search parses "gateway error order" into
process=gateway, level=error, search=order — but this
parsing is client-side JS only. The `/x/logs` endpoint
receives raw query params.

**Fix**: Either move parsing to server (parse `q` param)
or ensure the JS sets the correct individual filter
params before triggering HTMX.

### D4. Stale linter-modified test files

`play_overview.spec.ts` and `play_wal.spec.ts` were
modified by linter. Commit them.

---

## E. Test Gaps

### E1. Batch/random orders are never tested end-to-end

Since they're fake (A5/A6), tests only check the fake
response. After fixing, tests should verify fills arrive.

### E2. No test for scenario switch actually changing processes

`play_control.spec.ts` clicks the button but doesn't
verify process list changes.

### E3. WAL timeline filter has no test

Since it's broken (A2), no test covers it. Add one
after fixing.

---

## Priority Order

1. **A1-A4**: Broken UI wiring (pages.py only, fast)
2. **A5-A6**: Batch/random orders never submitted (server.py)
3. **D1**: Remove misplaced stress button
4. **A7**: Fill OID extraction
5. **B1-B3**: Fake endpoints (make real or label)
6. **C1-C4**: Add source labels to fallback responses
7. **D2-D3**: Symbol list, smart search
8. **D4, E1-E3**: Tests and cleanup

## Scope

~15 changes across 2 files. No new files needed except
possibly `/api/symbols`. No architectural changes. All
fixes are surgical — change the specific broken line,
don't refactor surrounding code.
