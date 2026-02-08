# WAL Infrastructure (Risk + Orderbook)

> **Note:** The concrete WAL implementation (file format, writer,
> reader, replay server, consumer) is specified in
> [DXS.md](DXS.md). This document describes the shared design
> principles and backpressure rules.

This document describes the shared WAL architecture for the risk engine and the matching engine. It is optimized for latency by accepting a bounded loss window and enforcing backpressure when persistence falls behind.

## Goals

- **Low latency**: hot path is an in-memory append (ring buffer).
- **Bounded loss**: accept up to 10ms data loss on crash.
- **Hard backpressure**: if persistence or replication lags beyond bounds, the system stalls.
- **Reusable infra**: risk and orderbook use the same WAL pattern, but implementations may diverge if sharing is impractical.

## Architecture

```
Producer (risk/orderbook)
  ├─ local WAL buffer (in-memory ring; fast append)
  ├─ WAL flusher (fsync every 10ms or when size threshold reached)
  ├─ Offload worker (batched commit to durable storage)
  └─ replica sync (risk replica consumes WAL stream)
```

### Local WAL Buffer

- Append-only ring buffer with a hard cap.
- Append is O(1) and the only operation on the hot path.
- If the buffer is full, the producer **stalls** (or rejects new work, per component policy).

### WAL Flush

- Flush to durable storage every **10ms** or on size threshold.
- Each flush is a batch and **must fsync** to make the 10ms bound real.
- If flush falls behind, the producer stalls to preserve the bound.

### Offload Worker (Durable Commit)

- Offload WAL batches in a background worker to a durable store.
- Batches are committed in a single transaction (group commit).
- The durable store can be Postgres or another WAL-backed system.
- Offload buffers can be larger than the local WAL, but they only grow while local WAL is fully synced.

### Replica Sync (Risk Engine)

- A hot replica of the risk engine consumes the WAL stream.
- Replica sync has a looser bound (e.g., **100ms**). If exceeded, the system stalls.
- This preserves a bounded failover window without unlimited drift.

## Orderbook Usage

- Matching engine uses the same WAL pattern to persist orders, cancels, and fills.
- WAL flush + snapshot restore is the recovery path (see ORDERBOOK.md).
- The 10ms bound is acceptable to reach target latency; faster durability implies higher latency.

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
