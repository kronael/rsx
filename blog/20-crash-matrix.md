# Audit Crash Safety by Tracing Value, Not Components

The standard crash audit lists components and asks "what happens if
this crashes?" That's the wrong question. The right question is "what
value was in flight, and where is it now?"

Value in this system means margin, fills, and positions. Everything
else is recoverable noise.

## The Matrix

We wrote a 12-scenario matrix. Each row is a failure mode. Columns are
the same for every row: which value is in flight, what recovers it,
and what's the residual risk if recovery fails.

Most of the interesting findings are in the residual risk column.
Five rows produced actionable findings:

- C1: Gateway crash. Value: order intent. Recovers on reconnect.
  Residual: duplicate submit pressure.
- C2: Risk crash after reserve. Value: frozen margin. Partial
  recovery from order_freezes. Residual: stranded forever if no ME
  terminal event. P0.
- C3: Risk crash mid-fill. Value: position delta. Recovers from ME
  WAL replay. Residual: replay gap for non-fill lifecycle events.
- C4: DB transient during flush. Value: persistence lag. In-memory
  retry. Residual: unbounded delay, no operator signal.
- C5: ORDER_FAILED replay path. Value: frozen margin. Fixed — replay
  now handles it. Residual: low.
- C11: Dual ME replicas promote. Value: fill seq integrity. Fixed
  with unique constraint. Residual: duplicate fills prevented.

## C2: The Frozen Margin Trap

Risk freezes margin in memory, then forwards the order to ME. Two
separate operations, no transaction between them.

```
process_order()    <- reserve margin here
  accepted_cons.pop()
  send_raw(order)  <- crash window here
```

If Risk crashes in that window, the order never reaches ME. ME never
emits a lifecycle event. Replay restores `order_freezes` from Postgres
and waits for a terminal event that will never arrive.

No error. No log. The frozen margin is stranded until an operator
manually clears it or the account is closed.

This is a P0 correctness bug. The user's collateral is locked against
a position that does not exist. The system considers itself consistent
because the freeze record is present and replay found no terminal event
to contradict it.

The fix is an outbox: atomically persist reserve-and-send-intent, then
replay the outbox on restart with a timeout-based compensating release.
Not shipped yet.

## C4: Persist Retry Blindness

When a Postgres flush fails, the persist worker retries the same
in-memory batch. Correct for transient errors.

The problem is "in-memory only." If the DB is down for 30 minutes,
the batch retries for 30 minutes. No spill file. No timeout. No alert.
Backpressure will eventually stall the hot path, but by then the
commit lag is already measured in minutes, not milliseconds.

The initial write in the audit said "pending." That undersells it.
"Unbounded delay with no operator signal" is closer.

Fix: add `persist_flush_failed_batches_total` and
`persist_oldest_unflushed_ms` metrics. Add a spill file when retries
exceed a threshold. Neither of these require touching the hot path.

## C11: Split-Brain Fills

Two ME replicas promote simultaneously. Both accept orders. Both write
fills to Postgres. Fills from both use the same per-symbol seq counter.

Before the fix, the `fills` table had no uniqueness constraint on
`(symbol_id, seq)`. The second insert silently succeeded, leaving
duplicate fills in the ledger. Position = sum of fills. Duplicate fill
= double-counted position change.

Fix applied: `UNIQUE(symbol_id, seq)`. The second insert now fails
with a constraint violation. The duplicate is rejected. The violation
fires an alert. The operator intervenes.

This is the cheaper half of the fix. The expensive half is preventing
dual promotion in the first place, which requires a fencing token in
the advisory lock path.

## C5: Already Fixed

`ORDER_FAILED` in the live path released frozen margin. The replay path
did not. Crash before flush, restart, replay: margin stayed frozen.

Fixed by adding `RECORD_ORDER_FAILED` handling to `replay.rs`. Regression
test needed: fail-order → crash → replay → verify frozen margin released.

## The Lesson

Silent failures cluster at reserve→send boundaries. That's where one
durable operation hands off to another and no transaction spans both.

Audit by asking "what value was reserved but not yet confirmed?" for
every handoff in the system. The answer is always either "nothing, we
have a transaction" or "this specific value, for this specific window."
If it's the second, you have a crash scenario to fill in.

The components-first audit misses C2 entirely because the component
(Risk) looks fine: it's running, it reserved margin, it logged the
acceptance. The value-first audit finds it immediately: margin was
reserved, order was not sent, no release path exists.

The rule: for every `reserve()` call, identify the paired `release()`
and the crash window between them. If the window exists, the residual
risk is stranded value.

## See Also

- `CRASH.md` - Full 12-scenario matrix with remediation status
- `rsx-risk/src/shard.rs` - Reserve-to-send gap (C2)
- `rsx-risk/src/persist.rs` - Retry behavior (C4)
- `rsx-risk/src/replay.rs` - ORDER_FAILED replay (C5)
