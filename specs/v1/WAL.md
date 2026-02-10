# WAL Infrastructure (Risk + Orderbook)

> **Note:** The concrete WAL implementation (file format, writer,
> reader, replay server, consumer) is specified in
> [DXS.md](DXS.md). This document describes the shared design
> principles and backpressure rules.

This document describes the shared WAL architecture for the risk engine and the matching engine. It is optimized for latency by accepting a bounded loss window and enforcing backpressure when persistence falls behind.

## Record Format (v1)

- WAL uses **fixed-size records** (no protobuf, no extra envelope).
- Records are `#[repr(C, align(64))]` with explicit little-endian fields.
- Each record starts with a 16-byte header:
  `{version, record_type, len, stream_id, crc32}`.
- Payloads begin with the CMP prefix (seq/ver/kind/len).
- Header `version` is the WAL wire-format version; payload
  `ver` is the record schema version.
- Concrete record layouts are defined in **DXS.md** and reused for storage + streaming.

### Version Policy

- **Additive changes** (new record types, trailing fields):
  no version bump. Readers ignore trailing bytes beyond known
  payload.
- **Breaking changes** (field reorder, type change): bump
  version in header + fail-fast on unknown version.
- See DXS.md section 1 for full version handling
  specification.

**Scope (v1):** WAL is written by the **matching engine** and contains all
orderbook events (new/cancel/fill/done/failed/config). Order intents at
ingress are **not** WAL’d; they can be lost if risk dies before execution.

## Goals

- **Low latency**: hot path is an in-memory append (ring buffer).
- **Bounded loss**: accept up to 10ms data loss on crash.
- **Hard backpressure**: if persistence or replication lags beyond bounds, the system stalls.
- **Reusable infra**: risk and orderbook use the same WAL pattern, but implementations may diverge if sharing is impractical.

**Formal guarantees:** See [GUARANTEES.md](../../GUARANTEES.md) for complete
specification of durability guarantees, data loss bounds, and recovery
procedures.

## Architecture

```
Producer (matching engine)
  ├─ local WAL buffer (in-memory ring; fast append)
  ├─ WAL flusher (fsync every 10ms or when size threshold reached)
  ├─ Offload worker (batched commit to durable storage)
  └─ replica sync (risk replica consumes WAL stream)
```

### Local WAL Buffer

- Append-only ring buffer with a hard cap.
- Append is O(1) and the only operation on the hot path.
- If the buffer is full, the producer **must stall**. This is required for correctness.

### WAL Flush

- Flush to durable storage every **10ms** or on size threshold (1000 records).
- Each flush is a batch and **must fsync** to make the 10ms bound real.
- If flush falls behind, the producer **must stall** to preserve the bound.

**10ms flush guarantee:** All fills emitted by the matching engine are written
to WAL and flushed to disk within 10ms. This provides the **0ms fill loss
guarantee** — any fill that was emitted can be replayed from WAL, even if Risk
was offline when the fill occurred.

**Backpressure enforcement:** If flush lag exceeds 10ms (disk slow), the
producer (ME) stalls on order processing. This prevents unbounded loss window
and ensures the 10ms bound holds under all conditions.

### Offload Worker (Durable Commit)

- Offload WAL batches in a background worker to a durable store.
- Batches are committed in a single transaction (group commit).
- The durable store can be Postgres or another WAL-backed system.
- Offload buffers can be larger than the local WAL, but they only grow while local WAL is fully synced.

### Replica Sync (Risk Engine)

- A hot replica of the risk engine consumes the WAL stream.
- Replica sync has a looser bound (e.g., **100ms**). If exceeded, the system **must stall**.
- This preserves a bounded failover window without unlimited drift.

## Orderbook Usage

- Matching engine uses the same WAL pattern to persist orders, cancels, and fills.
- WAL flush + snapshot restore is the recovery path (see ORDERBOOK.md).
- The 10ms bound is acceptable to reach target latency; faster durability implies higher latency.

**Crash mid-rotation:** WAL files are append-only. The active
file uses a temporary name (`{stream_id}_active.wal`).
Rotation renames the active file with its seq range. If the
process crashes mid-rotation, the active file remains with
its temporary name. On recovery, the reader treats
`_active.wal` as the last file and replays from the last
complete record (CRC validation truncates at first invalid
record). No data loss.

## Risk Usage

- Risk engine uses the same WAL pattern to persist positions and balance changes.
- Offload to the durable store is asynchronous, but bounded by the local WAL flush.
- Replica sync ensures fast takeover but is also bounded.

## Hard Backpressure Rules

- **Local WAL full:** stall producer.
- **Flush lag > 10ms:** stall producer.
- **Replica lag > 100ms:** stall producer.

These rules keep the loss window bounded and prevent silent latency inflation.

## Critique and Verification

### Claim: "10ms bounded loss with async flush"
- True **only if** each 10ms batch is fsync'd and the system stalls when flush lags.
- If fsync is skipped or delayed, the loss window is larger and unbounded.

### Claim: "Batching makes throughput sufficient"
- Likely true: batching aligns with group commit and amortizes fsync cost.
- But you must measure sustained throughput vs peak bursts to ensure the buffer never grows unbounded.

### Claim: "Offload buffers can be large"
- Acceptable only if the **local WAL is fully synced** and remains the source of truth.
- Large offload buffers do not reduce loss risk, but they can mask downstream slowness.

### Claim: "Replica sync window of 100ms is safe"
- Safe only if you accept losing up to 100ms of updates on replica takeover.
- If this is unacceptable, the replica window must be tightened and will increase stall frequency.

### Claim: "Same WAL infra for risk + orderbook"
- Feasible at the pattern level (ring buffer + flusher + offload),
  but data formats, ordering guarantees, and recovery semantics differ.
- If shared implementation creates coupling or slows either path, split the code.

## Decision Summary

- Use a small, bounded local WAL with fsync every 10ms.
- Enforce stalls if flush or replica sync exceeds bounds.
- Offload to Postgres asynchronously with batching.
- Apply the same pattern to orderbook and risk, but keep the option to diverge.
