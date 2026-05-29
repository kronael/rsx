# Risk engine: structure + hot-path clone analysis (2026-05-29)

Goal: better-structured, minimal, orthogonal rsx-risk; hot path clone-less
("almost one copy off the network buffer"); **no speed regression**.

## Hot-path copy audit (order/fill ingest)

Path: monoio UDP recv → `CastReceiver::try_recv` → `decode_payload` →
**SPSC ring** → `shard.run_once` pops → `process_order`/`process_fill`.

Copies per message today:
1. **`decode_payload::<T>` (`encode_utils.rs:53`)** — `ptr::read_unaligned`
   copies `size_of::<T>()` bytes out of the network buffer into a typed
   value. This is the ONE legitimate copy (alignment + typing). Keep.
2. **`order_prod.push(decoded)` (`main.rs:712`, and `fill_prod`/`bbo_prod`)**
   — moves the decoded struct into the ring's backing array. **Redundant.**

`process_order(&order)` / `process_fill(&fill)` already take refs — the
shard processing itself is clone-less. So copy #2 (the ring) is the only
removable copy on the hot path.

## Why copy #2 is removable

The recv loop and the consumer are the **same thread**. `shard.run_once`
(which pops `order_cons`/`fill_cons`/`bbo_cons`) is called *inside* the
recv loop (`main.rs:727`, `892`, `1054`) — there is no separate consumer
thread. The input rings (`order`, `fill`, `bbo`, `mark`) are therefore
**same-thread queues**: the loop pushes, then the same loop pops and
processes. Pure overhead: an extra 128-ish-byte copy + ring atomics +
drop-on-full bookkeeping + the `run_once` dispatch indirection, per msg.

This is the open MEMORY item: *"Risk simplification: drop SPSC rings,
call shard methods directly (like ME)."*

## Proposed change (performance-positive, minimal)

Replace the 4 **input** rings with direct calls from the recv handlers:

```
CastRecv::Data → decode_payload::<OrderRequest> → shard.process_order(&o)
                 decode_payload::<FillRecord>    → shard.process_fill(&f)
                 decode_payload::<BboRecord>     → shard.process_bbo(&b)
                 decode_payload::<MarkPrice...>  → shard.process_mark(&m)
```

Result: hot path = **decode (1 copy) → process directly**. Removes copy
#2, the `order`/`fill`/`bbo`/`mark` rings + their producers/consumers, the
`RingSet` plumbing for them, the `run_once` pop loops, and the
drop-on-full / backpressure-counter logic those rings carried.

**Keep:**
- **`persist` ring** — shard → tokio PG write-behind worker (`main.rs:460`
  is a real separate thread). Genuine cross-thread boundary. Must stay.
- **`response` / `accepted` (egress) rings** — judgment call: they buffer
  the sync shard → async casting send (same tile, but sync→I/O boundary).
  Either keep, or have the recv handler send directly after `process_*`
  returns the response. Lower priority; do separately and carefully.

Speed: strictly faster (one fewer copy, no ring atomics, no dispatch
hop). Satisfies "no impact on speed" (it's an improvement). Correctness:
back-pressure semantics change — with direct calls the recv loop *is* the
processing, so a slow shard naturally stalls the recv (the socket buffer
absorbs, NAK/replay recovers) — same end-state as ring-full stall, fewer
moving parts.

## Structural / orthogonality findings

- **`main.rs` is 1540 lines** — tangles the hot recv loop with failover
  machinery (lease thread, replica promotion, persist-worker spawn,
  replay-on-fault). Split the bootstrap/failover plumbing into a module
  (e.g. `failover.rs` / `bootstrap.rs`) so `main.rs` is a thin entrypoint
  + the hot loop. Orthogonalizes I/O-hot-path from lifecycle.
- **`rings.rs` / `RingSet`** shrinks to just `persist` (+ egress) once
  input rings go — delete the dead ring fields.
- **`shard.rs` is 1228 lines** but cohesive (the state machine). Leave its
  size; it's one concern. Only trim genuinely dead helpers if found.
- Off-hot-path allocs (`config.rs`, `replay.rs`, `persist.rs` drain,
  `liquidation.rs` `Vec::with_capacity(4)`) are init/batch — leave them.

## Sequencing

The orphan-freeze sub-agent is currently editing `shard.rs` / `main.rs` /
`replay.rs`. **Do not edit those concurrently.** Order:
1. Land + verify the orphan-freeze fix.
2. Implement input-ring removal (touches `main.rs` recv loop + `rings.rs`
   + `RingSet` in the shard wiring). Bench risk before/after to confirm
   no regression.
3. (Optional) split failover/bootstrap out of `main.rs`.
4. (Optional) egress-ring simplification.
