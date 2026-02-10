# Matching Engine Service

Matching is per-symbol, single-threaded, and stateless with
respect to user balances. It consumes validated orders from
Risk and emits fills and order lifecycle events.

## Responsibilities

- Maintain orderbook (ORDERBOOK.md)
- Execute matches deterministically
- Emit fills, order inserted/cancelled/done events
- Append events to WAL (DXS.md/WAL.md)

## Inputs / Outputs

Inputs:
- CMP/UDP from Risk: validated orders

Outputs:
- CMP/UDP to Risk: fills and order lifecycle events
- WAL records for replay/marketdata

## Determinism

- Fixed-point arithmetic only
- Single-threaded per symbol
- No external I/O in the core loop

## Config

- Env-only: symbol_id, tick/lot, decimals

## Notes

This spec describes behavior; tile composition lives in
PROCESS.md. Implementation details live in ORDERBOOK.md and
WAL/DXS docs.
