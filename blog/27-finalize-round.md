# 27. The Finalize Round

When a system reaches "99% done", the last 1% is where
credibility lives. An investor doesn't see your spec
compliance matrix — they see a stack trace on a 404 page,
raw `100000000000000000` where "$1,000" should be, or a
WebSocket that silently returns nothing.

This is the methodology for the finalize round.

## The Multi-Angle Audit

Spawn 4 parallel agents, each playing a different role:

1. **Visual completeness** — hit every HTML endpoint, check
   sizes, look for blank pages, error pages, placeholder
   text. Count bytes. If a page is <100B, it's broken.

2. **Data flow correctness** — submit orders, read them
   back, check account balances, verify candles have OHLCV
   fields, confirm funding returns JSON. Follow the data
   path end to end.

3. **WebSocket health** — find every WS endpoint, try to
   connect, verify graceful degradation (code 1013 when
   upstream is down, not a crash). Check if the frontend
   actually uses WS or polls via HTMX.

4. **Error handling robustness** — throw every bad input
   you can think of: invalid symbols, negative prices,
   non-numeric strings, XSS payloads, path traversal,
   SQL injection. The bar: zero stack traces, zero 500s.

## The Fixed-Point Trap

The biggest class of bugs in a finalize round: raw internal
representation leaking to the API boundary. Our exchange
uses i64 fixed-point everywhere (prices, quantities,
collateral). The matching engine never touches floats.

But the `/v1/account` endpoint was returning:
```json
{"collateral": 100000000000000000}
```
instead of:
```json
{"collateral": "1000000000.0"}
```

Same for `/v1/orders` — prices like `49900000000` instead
of `"49900.0"`. The spec says "conversion at API boundary
only" but we hadn't actually done the conversion.

Rule: every number that crosses the API boundary must be
human-readable. Fixed-point is internal-only.

## The Spec Cross-Reference

After the first round of fixes, spawn 4 more agents:

1. **GUARANTEES.md** — does every stated guarantee have a
   test? If the guarantee says "fills precede ORDER_DONE",
   there must be a test asserting that.

2. **SCREENS specs** — does every screen element described
   in the spec exist in the HTML? Chart, orderbook, order
   form, position display.

3. **PROGRESS.md + CRITIQUE.md** — are items marked "done"
   actually done? Are critique items resolved?

4. **API spec (REST.md)** — does every spec'd endpoint
   exist? Does every existing endpoint have a test?

This second round catches structural gaps: endpoints that
exist in the spec but have no test (`/v1/positions`,
`/v1/fills`), or endpoints in the code that aren't in the
spec (`/v1/candles`).

## The Test-Per-Fix Rule

Every manual verification must produce a test. If you
checked that `/v1/account` returns human-readable numbers
by curling it, write `test_v1_account_returns_human_readable`
that asserts `collateral < 1e12`. If you checked that 404
pages don't show tracebacks, write
`test_no_stack_trace_on_404`.

The test suite is the durable artifact. The walkthrough is
ephemeral.

## Checklist as Living Document

Start with 40 items. The finalize round adds more. Ours
grew from 40 to 46+ as agents found gaps. Each item
has a test reference. When all items have tests and all
tests pass, you're done.

## The Result

460+ tests passing. 46-item audit checklist. Every page
renders, every API returns sane data, every bad input is
handled gracefully. No stack traces, no 500s, no raw i64s.

The investor sees a polished dashboard. What they don't see
is the methodology that got it there.
