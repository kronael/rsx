# FAULTED inbound orders: skip the gap, don't replay

**Domain term.** casting/UDP numbers each record with a `seq`. If a
consumer sees a gap it can't close with in-band NAK recovery, the
receiver returns **FAULTED** with the missing range. The reflex on
FAULTED is "recover or die" — replay the gap, or panic and let a replica
take over. For the risk→ME *order* stream, both reflexes are wrong.

## Problem

An inbound-order gap is not a durability problem here, so paying the
price of replay-or-panic is wasted or actively harmful:

- The dropped records are *pre-ack orders* — the client has not been told
  they were accepted.
- Panicking would take down the matching engine over a loss the system
  already recovers at the application layer.
- Replaying the gap would require pulling from an upstream order-replay
  source the ME no longer depends on.

## Fix

On `CastRecvWith::Faulted`, the loop **skips the gap and resumes live** —
no replay, no panic (`main.rs`):

```
Faulted { gap_start, gap_end_inclusive, .. } =>
  gauges.drops += (gap_end_inclusive - gap_start + 1)
  warn!("skipping unrecoverable order gap […]")
  cast_receiver.reset_after_replay(gap_end_inclusive)   // resume at gap_end+1
  continue
```

This is sound because two other mechanisms already own the recovery:

- **Client retry + WAL dedup = exactly-once.** A dropped pre-ack order is
  re-sent by the client on no-ack-timeout (`specs/2/49-webproto.md`) and
  deduped on the ME's WAL `RECORD_ORDER_ACCEPTED`
  (`notes/dedup-persistence.md`).
- **ME re-sequences on its own output**, so an *inbound* gap is never an
  *output* gap: risk / recorder / marketdata still see a contiguous ME
  stream.

The asymmetry is the point. **ME→risk fill delivery** is the direction
that genuinely needs recovery — a lost fill is unrecoverable — and it has
its own path: the ME runs a WAL-replication *server*
(`RSX_ME_REPLICATION_BIND_ADDR`) that risk pulls from on risk-side
FAULTED. Inbound order gaps are drop-safe; outbound fill gaps are not.

## Cost it removes

A spurious crash (or a needless replay) on a transport blip that the
client-retry + WAL-dedup path already handles — while keeping the
outbound fill stream, which is *not* drop-safe, on its own recovery path.

## Cite

- `main.rs` (the `Faulted` arm, `reset_after_replay`);
  `ARCHITECTURE.md` § "FAULTED → skip-the-gap".
- `GUARANTEES.md` §1.0 (delivery guarantees by stream);
  `specs/2/49-webproto.md` (client resend).
