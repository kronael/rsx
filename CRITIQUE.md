# Critique (Refined, Functionality-First)

This critique reflects the current code and tests. I re-ran `cargo test` and
re-checked README/CLAUDE/PROGRESS/spec claims against actual implementation.

## What Has Improved

- `rsx-matching` now has real building blocks: inbound wire types, fanout
  routing to SPSC rings, and WAL write integration with tests.
- There are now targeted tests for fanout routing, WAL integration, and
  message conversion.
- Config is now env-only in the implemented binaries.

## Remaining Functional Deficiencies

These are real gaps that block a usable pipeline.

- **No real ingress.** `rsx-matching` creates an ingress ring but never exposes
  a producer or a network endpoint, so it still processes zero orders in
  practice.
- **No runnable end-to-end flow.** There is still no gateway, risk engine, or
  market data service. The system cannot accept external orders or produce
  user-visible fills outside of tests.
- **No live DXS streaming in practice.** The matching engine writes WAL, but no
  process runs `DxsReplayService` alongside it, and no plumbing connects the
  producer to a running recorder or other consumers.
- **Timestamps are not wired.** `rsx-matching` writes WAL records with `ts_ns = 0`
  in the main loop, so WAL timestamps are invalid in practice.
- **Order identifiers are placeholders.** WAL records use slab handles and
  user IDs in `oid` fields because the book events don’t carry real order IDs.
  This makes `oid` semantics incorrect for consumers.

## Verified Documentation Mismatches

These are concrete doc → code mismatches that remain.

- **RSX matching status in PROGRESS.** It still calls `rsx-matching` a stub,
  but there is now real WAL integration and fanout logic (even if ingress is
  still missing).

## Test Surface vs Reality

- **Proven:** Orderbook correctness, WAL encoding/decoding, WAL read/write
  behavior, fanout routing, and WAL integration are covered by tests.
- **Not proven:** Any real multi-process or network flow (matching → DXS server
  → recorder), risk checks, gateway ingress, or recovery across processes.

## Minor Build Hygiene

- `cargo test` emits a warning in `rsx-dxs/tests/client_test.rs` about an
  unused loop variable. This is small but contradicts “warnings cleared.”

## Bottom Line

The core building blocks are now more integrated, but the system still doesn’t
run end-to-end. The highest-value next step is to wire a minimal producer and
consumer path (matching → DXS server → recorder) so the pipeline runs outside
unit tests.
