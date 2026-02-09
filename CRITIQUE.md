# Critique (Refined, Functionality-First)

This critique reflects the current code and tests. Updated after
addressing 5 of 7 issues (order IDs, risk binary, DXS sidecar,
panic handlers, production macros).

## What Has Improved

- `rsx-matching` writes WAL records with real timestamps, has
  wire/fanout/WAL integration tests.
- `rsx-risk` and `rsx-mark` code exists with substantial unit
  test coverage.
- Config is env-only across all binaries.
- **[FIXED] Order IDs are real.** `order_id_hi`/`order_id_lo`
  (u64 pair = UUIDv7 128-bit) wired through OrderSlot, Event,
  IncomingOrder, OrderMessage, EventMessage, and WAL records.
  No more `handle as u128` placeholders.
- **[FIXED] rsx-risk has a binary.** `main.rs` with cold start
  from Postgres, WAL replay, persist worker, core pinning, and
  retry loop.
- **[FIXED] DXS sidecar in ME.** If `RSX_ME_DXS_ADDR` is set,
  ME spawns a DxsReplayService thread. Consumers can attach to
  a live WAL stream.
- **[FIXED] Production panic handlers.** All 6 binaries use
  `exit(1)` with info printing. `rsx-types` exports
  `install_panic_handler()` and flow-control macros.
- **[FIXED] Mark aggregator retry loop.** `rsx-mark` wraps its
  main loop in `run() -> Result` with 5s restart on error.
- **[FIXED] Config test isolation.** `config_parse_valid_env`
  no longer fails (mark config tests pass in parallel).

## Remaining Functional Deficiencies

These are real gaps that block a usable pipeline. All require
networking implementation (monoio WS, QUIC transport).

- **No real ingress.** `rsx-matching` creates an ingress ring
  but does not expose a network endpoint. Requires monoio WS
  in gateway + QUIC transport to risk/ME.
- **No end-to-end flow.** `rsx-gateway` and `rsx-marketdata`
  have stub main loops (spin_loop). Requires monoio WS and
  QUIC inter-process transport.

## Test Surface vs Reality

- **Proven:** Orderbook correctness, WAL encode/decode, WAL
  read/write, fanout routing, WAL integration, risk margin/
  funding/persistence, mark aggregation, gateway protocol/
  rate limiting/circuit breaking, marketdata shadow book/
  subscriptions.
- **Not proven:** Multi-process network flow (matching → DXS
  → recorder), gateway WS ingress, cross-process recovery.

## Bottom Line

All pure logic is complete and tested. The remaining blockers
are networking: monoio WS for gateway/marketdata, QUIC for
inter-process transport. These are implementation tasks, not
design gaps.
