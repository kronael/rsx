# Critique (Current)

## What Is Solid

- Clear dataflow: Gateway → Risk → Matching, plus separate market-data service.
- Deterministic fixed-point arithmetic across all critical paths.
- WAL + snapshot for orderbook; replay for risk.
- Metadata scheduling with `CONFIG_APPLIED` sync events.
- Reduce‑only enforcement is specified at the matcher with per‑user net positions.
- Fee fields and accounting exist in `GRPC.md` + `RISK.md`.

## Critical Gaps

1. **Market-data resync semantics** are underspecified (no sequence numbers; drop+snapshot policy needs clearer guarantees).
2. **Fixed-record WAL completeness**: fixed records do not yet carry fee or `client_order_id`, so downstream replay can lose accounting/context.
3. **Intent loss is accepted**: order intents can be lost on risk crash (by design). This is fine only if client ACKs are explicitly non‑durable.

## Risky Assumptions

- Backpressure correctness depends on strict stalling; any bypass breaks loss bounds.
- Clock skew: metadata scheduling relies on UTC wall clock + monotonic config_version.

## Small Next Fixes

- Add fee and client_order_id to fixed WAL records where needed (Fill/OrderDone/OrderCancelled).
- Add a minimal sequence field to market‑data deltas or formalize drop+snapshot recovery.
- Make “ACK is non‑durable” explicit in gateway docs.
