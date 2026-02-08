# Critique (Specs-Only, Fresh Read)

This critique is written as if the reader has no prior context, based only on `specs/`.

## What Is Clear

- **System roles are explicit**: Gateway, Risk, Matching, Market Data, Metadata scheduler, WAL/Replay.
- **Core data types are consistent**: fixed‑point price/qty, UUIDv7 IDs, and explicit enums.
- **Durability path exists**: matcher WAL + snapshot with DXS replay; risk persists positions/accounts.
- **Config changes are scheduled**: metadata schedule + `CONFIG_APPLIED` events.

## Gaps / Underspecified Areas (Remaining)

1. **Risk replica promotion invariant**
   - The spec should state in one line: “apply all buffered fills up to last tip before promotion.”

2. **Active user ID mapping**
   - Matching engine uses compact per‑symbol user IDs, but the authoritative source of `active_user_id` assignment and reclaim rules are not explicitly stated.

3. **Replay retention guarantees**
   - DXS retention is 10 minutes, now with infinite offload. The spec should state what happens if replay exceeds the hot window (read from offload archive).

## Risky Assumptions

- **Ingress orders can be lost**: the system is explicitly non‑durable at accept time; clients must tolerate re‑submit.
- **Backpressure correctness depends on stalling**: if any component fails to stall, the bounded‑loss guarantee is broken.
- **Wall‑clock schedule**: metadata changes depend on UTC time; large clock skew can delay or mis‑order changes (mitigated by `config_version`, but still operationally sensitive).

## Recent Fixes Already Applied

- ACK semantics are now explicit in `WEBPROTO.md` and `GRPC.md`.
- Market‑data `seq` is defined in `MARKETDATA.md` with drop+snapshot recovery.
- WAL record fields expanded; cancel reason mapping is defined in `DXS.md`.
