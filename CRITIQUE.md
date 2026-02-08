# Critique (Specs-Only, Fresh Read)

This critique is written as if the reader has no prior context, based only on `specs/`.

## What Is Clear

- **System roles are explicit**: Gateway, Risk, Matching, Market Data, Metadata scheduler, WAL/Replay.
- **Core data types are consistent**: fixed‑point price/qty, UUIDv7 IDs, and explicit enums.
- **Durability path exists**: matcher WAL + snapshot with DXS replay; risk persists positions/accounts.
- **Config changes are scheduled**: metadata schedule + `CONFIG_APPLIED` events.

## Gaps / Underspecified Areas (No Obvious Tradeoff)

1. **Gateway ACK semantics**
   - Specs say order intents can be lost on risk crash, but they don’t explicitly define what a client ACK means (durable vs best‑effort). This must be stated clearly.

2. **Market‑data resync contract**
   - `MARKETDATA.md` says “drop deltas and resend snapshot,” but there is no sequence numbering or explicit resync rule for clients. A minimal sequence or resync contract is needed.

3. **Fixed‑record WAL field completeness**
   - WAL records now include more fields, but the spec doesn’t explicitly define which records must carry **fees** and **client_order_id** for downstream accounting. The required field set must be listed per record type.

4. **Cancel reason mapping**
   - `CancelReason` is defined, but mapping from cancel sources (user cancel, reduce‑only clamp, expiry, post‑only reject) to concrete reason codes isn’t stated.

5. **Risk replica promotion invariants**
   - Risk replica behavior is described, but the spec does not explicitly state the invariant for “safe promotion” (e.g., apply all buffered fills up to last tip before takeover). This should be stated as a one‑line invariant.

6. **Active user ID mapping**
   - Matching engine uses compact per‑symbol user IDs, but the authoritative source of `active_user_id` assignment (gateway vs matcher) and its lifetime rules are not stated.

7. **Replay retention guarantees**
   - DXS retention is 10 minutes, but the spec doesn’t state the maximum allowed downtime for recovery or what happens if replay horizon is exceeded.

## Risky Assumptions

- **Ingress orders can be lost**: the system is explicitly non‑durable at accept time; clients must tolerate re‑submit.
- **Backpressure correctness depends on stalling**: if any component fails to stall, the bounded‑loss guarantee is broken.
- **Wall‑clock schedule**: metadata changes depend on UTC time; large clock skew can delay or mis‑order changes (mitigated by `config_version`, but still operationally sensitive).

## Suggested Minimal Fixes

- Add a short “ACK semantics” paragraph to `WEBPROTO.md` and `GRPC.md`.
- Add a **sequence number** to market‑data deltas (or specify forced resync rules).
- Add a **per‑record field checklist** to `DXS.md` (fees, client IDs, cancel reason).
- State the **replica promotion invariant** in `RISK.md`.
- Document `active_user_id` assignment and reclaim rules in `ORDERBOOK.md`.
- State replay horizon behavior when DXS retention is exceeded.
