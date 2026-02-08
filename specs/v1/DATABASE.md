# Database Choice for Positions

## Recommendation

Use **PostgreSQL as the system of record** for positions and, if needed, **Redis only as a cache**.

Rationale:
- You said durability cannot be compromised. PostgreSQL provides strong WAL durability when `fsync` and `synchronous_commit` are enabled.
- Redis durability depends on AOF `appendfsync` policy. The safest mode (`always`) is typically slower; the default (`everysec`) allows a small loss window.

## PostgreSQL (Primary)

- **Role:** Authoritative store for positions.
- **Durability:** Keep `fsync=on` and `synchronous_commit=on`.
- **Stronger failure tolerance:** Use **synchronous replication** if you need commits to survive a primary node failure.

Pros:
- Strong WAL guarantees and crash safety.
- Mature replication and tooling.

Cons:
- Higher commit latency vs memory-only stores.

## Redis (Optional Cache)

- **Role:** Read cache for fast lookups; treat as non-authoritative.
- **Durability:** If used as a primary (not recommended), requires AOF with `appendfsync always`, which is slower.

Pros:
- Very low read latency.
- Good for hot reads and short-lived state.

Cons:
- Durability depends on AOF policy; default settings can lose recent writes.

## Practical Pattern

1. **Write path:** Risk engine writes positions to PostgreSQL (single-writer per user/partition).
2. **Read path:** Risk engine reads from Redis (or in-memory) with cache-aside; on miss, read PostgreSQL.
3. **Recovery:** Rebuild Redis from PostgreSQL after restart.

## Async Persistence with Bounded Loss (e.g., 10ms)

You can get "fast path" latency (comparable to an `rtrb` write) by making the
**application** append to an in-memory ring buffer and persisting to Postgres
in short batches. This does **not** make Postgres faster; it shifts durability
to the application and bounds loss to the buffer window.

### Architecture (Write-Behind Buffer)

```
Risk Engine
  ├─ rtrb (in-memory append, O(1))
  ├─ write-behind worker (flush every 10ms OR when buffer reaches N)
  └─ Postgres (batched INSERT/COPY in a single transaction)
```

Behavior:
- Orders/position updates land in the in-memory buffer immediately.
- A background task flushes every ~10ms (or when size threshold is hit).
- Each flush writes a batch in one transaction and **waits for commit**.
- Worst-case loss on crash = updates still in buffer (bounded by 10ms).

### What This Gives You
- **Fast hot path:** the critical path is an in-memory write.
- **Bounded loss:** at most the current buffer window (e.g., 10ms).
- **Durable once committed:** after commit returns, Postgres WAL guarantees apply.

### What It Does NOT Give You
- It does **not** make durability free. If you don't wait for commit, you are
  accepting loss beyond the buffer window.
- It does **not** guarantee a strict 10ms bound in all cases unless you
  enforce it (flush timer + backpressure when lagging).

### Backpressure Rule (Required)

If the background writer falls behind (e.g., Postgres slow), you must either:
- **Fail fast**: reject new writes; or
- **Block**: slow the risk engine input.

Otherwise the buffer grows and your "10ms bound" is no longer true.

## Are You Rolling Your Own Database?

You are not writing a database, but you **are** writing a small write-behind
log layer with durability implications. That comes with obligations:

- **Ordering guarantees:** if positions must be strictly ordered, the buffer
  and flush logic must preserve order.
- **Idempotency:** retries must not double-apply updates.
- **Crash semantics:** you must accept and clearly define what is lost.
- **Backpressure:** enforce a hard limit to keep the loss bound true.
- **Monitoring:** track flush latency and buffer depth.

This is a reasonable pattern, but it is **not free**. Treat it like a
mini-queue with a strict SLA.

## Critique of the Claims

**Claim: \"Latency of a single memory write to rtrb.\"**  
True for the enqueue step, but not for durable persistence. Your durability
latency is the flush + commit time, which is separate. So the end-to-end path
is fast only if you accept a bounded window of loss.

**Claim: \"Bounded data loss of 10ms.\"**  
Only true if:
1. Flush runs on schedule, and
2. Buffer is capped, and
3. You fail or block when Postgres can't keep up.
If any of these are violated, the bound can be much larger.

**Claim: \"Batch inserts using pipelining.\"**  
Batching is good and aligns with Postgres group commit. But it moves you into
write-behind territory; you must make the loss window explicit and enforce it.

**Claim: \"Reuse the orderbook snapshot/migration logic.\"**  
Conceptually possible for buffering + snapshotting state, but remember:
orderbook snapshotting is for **rebuilding matching state**, while positions
are **durable accounting data**. Reusing algorithms is fine, but you are still
building a persistence layer with its own correctness requirements.

## Open Inputs (if you want a concrete config)

- Target write rate (ops/sec)
- Acceptable commit latency (p50/p99)
- Failure model (single node vs multi-node)
- Replication requirement (must survive primary loss?)

## Five Key Points (Postgres vs RocksDB)

- PostgreSQL is the safest default for durability and operational simplicity; RocksDB is faster but shifts storage complexity to you.
- RocksDB has its own WAL, but you still own compaction tuning, backups, and recovery validation.
- "Async" Postgres can be fast on the hot path, but only if you accept a bounded loss window and enforce backpressure.
- Redis + custom WAL is the highest-risk option; you effectively build a persistence layer from scratch.
- If latency is the top priority and you can invest in embedded storage ops, RocksDB is viable; otherwise choose Postgres.
