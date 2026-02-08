# DEFICIENCIES (TLDR)

## Resolved

- **Fees**: added to GRPC.md, RISK.md, WEBPROTO.md (maker
  rebates, pre-trade fee reserve, fills table columns)
- **Margin formulas**: concrete formulas in RISK.md section 3
  (per-position, per-user, pre-trade, liquidation trigger,
  edge cases, MarginState struct)
- **Reduce-only orders**: ME position tracking in ORDERBOOK.md
  section 6.5, reduce-only enforcement before matching,
  GRPC.md/WEBPROTO.md fields, LIQUIDATOR.md updated
- **BBO event routing**: added to CONSISTENCY.md event table
  and drain loop
- **Index price edge cases**: zero-qty, one-sided, no-BBO
  fallbacks in RISK.md section 4
- **Heartbeat semantics**: 5s/10s timeout in WEBPROTO.md
- **Funding settlement timing**: UTC 00/08/16 in RISK.md
- **ME restart dedup**: documented in GRPC.md
- **Liquidation order rejection**: on_order_failed handler
  in LIQUIDATOR.md

## Remaining

- **Binance feed details**: reconnect backoff, staleness
  threshold (minor, 10 lines in RISK.md)
- **Modify order**: v1 deferred (cancel + re-insert)
