# One CRC, one seq, three destinations

**Domain term.** Before a record hits the WAL it is *framed*: assigned a
monotonic `seq` and a CRC checksum, producing a `Framed` byte blob. Each
of the three destinations — the WAL file, the cast stream to risk, the
cast stream to marketdata — needs those exact bytes. A *seq* is the
per-stream sequence number a consumer uses to detect a gap (a missing
seq reads as loss → the consumer FAULTs and forces recovery).

## Problem

The obvious code prepares the record once per destination: frame-for-WAL,
frame-for-risk, frame-for-mkt. Two problems fall out:

- **Wasted CRC.** The CRC is recomputed three times over the same bytes.
- **Desynced seqs → false FAULTED storms.** If each stream stamps its
  *own* `seq` counter, the counters drift the moment any record goes to
  one stream but not another. This is the SEQ-1 bug: BBO records were
  sent with `CastSender::send` (the sender's own counter) while every
  other record used the WAL seq via `send_framed`. Because BBO wasn't
  WAL'd, the two counters desynced, the wire seq regressed, consumers
  read "sender reset" → FAULTED — a recurring storm (~2/sec, one per
  accepted order once `OrderAccepted` was WAL-only but not fanned out).

## Fix

Prepare each record **exactly once** (one `seq` from the single WAL
counter + one CRC → one `Framed`), then fan the *same bytes* to all three
sinks. Every record type that consumes a WAL seq is sent to *both* cast
streams even if a consumer ignores it, so no stream ever has a seq hole:

```
FanoutSink::emit(record):
  framed = writer.prepare(record)   // ONE seq (WAL counter) + ONE CRC
  writer.append_framed(&framed)     // WAL
  cmp.send_framed(&framed)          // risk   — same bytes, same seq
  mkt.send_framed(&framed)          // mkt    — same bytes, same seq
```

`emit_events` builds each wire record in exactly one place and hands it
to a sink (`EventSink`); the production `FanoutSink` fans out, the
`WalSink` (replay/bench) writes WAL-only — same record construction, no
re-CRC per destination. BBO now routes through the same WAL seq like
every other record (it is a skipped side effect on replay, but it *must*
occupy a seq to keep the stream contiguous).

## Cost it removes

Two redundant CRCs per record, and the entire class of false-FAULTED
storms caused by per-stream seq counters drifting. One counter, one
checksum, contiguous streams.

## Cite

- `wal.rs::publish_events` / `emit_events` / `FanoutSink` / `EventSink`;
  the SEQ-1 comments on the `Fill` / `OrderFailed` / `BBO` arms.
- `specs/2/6-consistency.md` § "Drain Loop Pseudocode" (SEQ-1 uniform
  fan-out) + invariants #1, #3; `ARCHITECTURE.md` § "Event Fanout".
