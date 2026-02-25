# TODO

## Critical — data not flowing

- [ ] Maker circuit-breaker crash: "10 consecutive errors;
      aborting maker" — quotes never reach the book.
      No fills, WALs stay 0B, latency stats empty.
      Root cause: maker binary fails to submit orders
      via gateway WS. Check maker.log for error detail,
      verify gateway accepts maker orders.
- [ ] WAL files 0.0B despite processes running — consequence
      of maker crash. ME has no orders to match, nothing
      to write.
- [ ] Trade UI: "Market data WS disconnected" / "Private
      WS disconnected" — trade SPA can't reach gateway
      or marketdata WS. Check CORS, WS upgrade path
      through nginx proxy.

## Invariant check fixes (done this session)

- [x] Position SQL: `p.quantity` → `long_qty - short_qty`
- [x] Position SQL: `taker_uid` → `taker_user_id`,
      include maker side fills
- [x] Funding SQL: `funding_payments` → `funding`
- [x] Stale orders TypeError: `ts` stored as string,
      arithmetic with float

## Latency pipeline (from perf-verification.md)

- [ ] Add `/api/latency` JSON endpoint (p50/p95/p99)
- [ ] Fix play_latency tests: endpoint skips on 404,
      chart area asserts nothing
- [ ] Latency values always "-" in sim mode — need real
      gateway orders for measurement
- [ ] Stress reports show 0% accept rate, 0us p99 —
      orders go to sim fallback, not real gateway

## Trade UI

- [ ] Docs page 502 through nginx (works on direct port)
- [ ] No open positions display
- [ ] WS reconnect logic may be broken or proxy strips
      upgrade headers
