---
status: reference
---

# TESTING-SMRB.md — SPSC Ring Buffer Tests

Source specs: [notes/SMRB.md](../../notes/SMRB.md),
[CONSISTENCY.md](CONSISTENCY.md)

The SPSC primitive is provided by the external `rtrb` crate. Its
correctness is covered by rtrb's own test suite. This spec documents
the requirements and integration contracts RSX relies on.

---

## Requirements Checklist

| # | Requirement | Source |
|---|-------------|--------|
| S1 | SPSC push/pop is wait-free (bounded steps) | SMRB.md |
| S2 | FIFO ordering preserved | SMRB.md |
| S3 | Cache-line padded indices (no false sharing) | SMRB.md |
| S4 | Power-of-2 capacity (bitwise AND) | SMRB.md |
| S5 | No heap allocation after creation | SMRB.md |
| S6 | push_spin bare busy-spin on ring full | CONSISTENCY.md §3 |
| S7 | Per-consumer rings (isolation) | CONSISTENCY.md §1 |
| S8 | Producer stalls on full (backpressure) | WAL.md, CONSISTENCY.md |
| S9 | Acquire/release atomics only (no CAS) | SMRB.md |
| S10 | Flat structs, no serialization (same struct both sides) | SMRB.md |
| S11 | Core pinning: producer and consumer on dedicated cores, same NUMA node | SMRB.md |
| S12 | Huge pages (2MB) for shared memory region | SMRB.md |
| S13 | no_std compatible (with alloc) | SMRB.md |
| S14 | Event routing matches CONSISTENCY.md table (per-event, per-consumer) | CONSISTENCY.md §1 |

---

## Integration Points

- SPSC rings are used for in-process handoff only; matching
  fan-out uses CMP/UDP in v1 (CONSISTENCY.md §1)
- Mirrored stream to hot spare ME via SPSC is not implemented in v1
- Recorder connects as DXS consumer for archival (CONSISTENCY.md §1, DXS.md §8)
- Event routing per consumer matches CONSISTENCY.md §1 table:
  Fill to risk/gateway/mktdata, BBO to risk, OrderInserted to
  mktdata, OrderCancelled to gateway/mktdata, OrderDone to
  risk/gateway
- Mark price aggregator pushes SourcePrice via SPSC (MARK.md §1)
- WAL writer backpressure: buf full triggers stall (DXS.md §3)
- System-level: verify no data loss across component boundaries
  under sustained 100K msg/sec load (TESTING.md §6 load tests)

Performance target (SMRB.md): ~50-170ns round-trip per message.
