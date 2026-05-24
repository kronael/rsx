# Superseded

Superseded by `.ship/26-CMP-RELIABILITY-V4/SPEC.md` (2026-05-24).

The v4 spec consolidates this entry's three deferred fixes plus the
v2 elaboration in `.ship/25-CMP-RELIABILITY-V2/SPEC.md` into one
implementation target. Key differences:

- v4 uses a ring-buffer reorder (zero heap, mirrors `send_ring`) vs.
  this entry's `BTreeMap<u64, Vec<u8>>` design.
- v4 drops the RESET tier from v2; it's just a more expensive NAK.
- v4 NAK targets the oldest contiguous missing run only (not the full
  window), eliminating retransmit waste.

This SPEC.md is kept for historical reference. Implementation should
read v4 only.
