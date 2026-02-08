# Critique (Refreshed)

## What Is Solid

- **Clear dataflow**: Gateway → Risk → Matching, with dedicated market-data service.
- **Deterministic math**: fixed-point price/qty across the stack.
- **WAL + snapshot**: explicit durability path for orderbook and replay for risk.
- **Config scheduling**: metadata is scheduled and synchronized via `CONFIG_APPLIED` events.

## Critical Gaps

1. **Fees still missing**: gRPC and risk do not include fee fields or accounting. PnL and balances will be wrong without this.
2. **Reduce‑only correctness**: safe resting reduce‑only requires matcher‑side per‑user state or a strict policy (IOC/no other orders). Not specified.
3. **Market data resync**: drop‑and‑resnapshot policy exists, but no explicit sequence numbers or recovery guarantees.
4. **Fixed‑record WAL completeness**: field sets are defined, but some records (e.g., cancel reason, fee, client_order_id) may be needed for downstream correctness.

## Risky Assumptions

- **Order intents can be lost** on risk crash (by design). This is acceptable only if clients treat ACK as non‑durable.
- **Backpressure relies on stall**. If a component fails to stall, loss bounds are invalid.
- **Clock skew**: config scheduling uses UTC wall clock; correctness relies on monotonic `config_version` and sane time sync.

## Suggested Next Fixes (Small)

- Add fee fields to `GRPC.md` and fee handling in `RISK.md`.
- Decide reduce‑only enforcement policy and document it.
- Add sequence numbers to market‑data deltas, or document that resync is required after any drop.
- Review fixed‑record structs for missing fields (client_order_id, fee, cancel reason).
