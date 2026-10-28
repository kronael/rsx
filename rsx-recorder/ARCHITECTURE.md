# rsx-recorder Architecture

Archival replication consumer process. Connects to a replication
server, receives all WAL records, writes to date-partitioned
archive files.

## Components

- `RecorderState` -- archive directory, current file handle,
  write buffer, daily rotation
- `ReplicationConsumer` (from rsx-cast) -- TCP client with tip
  persistence, driven one connection at a time by `follow_loop`, which
  reconnects after 1 s
- `watchdog` -- health: 200 while caught up, 503 after
  `RSX_RECORDER_STALL_SECS` without being caught up

## Data Flow

```
ME WAL --> ReplicationService --> [TCP] --> ReplicationConsumer
                                              |
                                         RecorderState
                                              |
                                    {stream_id}/{stream_id}_{date}.wal
```

## Record Processing

1. Connects to replication producer (replay server) using `ReplicationConsumer`
2. Receives `RawWalRecord` via callback
3. Skips a record whose seq is at or below the last seq written (seeded
   from the last whole record of the day's file at open), then buffers it
   and flushes to disk every 1000 records
4. Rotates output file daily (`{stream_id}_{date}.wal`)
5. Persists consumption tip for idempotent restart

## Health

The producer sends `CAUGHT_UP` once the replay reaches the live tail; the
recorder does not archive it and marks itself caught up. The mark clears when
the connection ends or an archive write fails, and returns at the next
`CAUGHT_UP`. An idle stream sends nothing more, so the mark holds and the
recorder stays healthy. The replication link has no idle heartbeat: a peer
that vanishes without closing its socket is not detected.

## File Layout

```
archive/{stream_id}/{stream_id}_{YYYY-MM-DD}.wal
```

Daily rotation at UTC midnight. Same binary WAL format as
source (no transformation).

## Architectural Decisions

**Runtime: tokio.** The recorder is a TCP-only replay
consumer — a single `ReplicationConsumer` covers historical catch-up
and the live tail indefinitely, with built-in exponential
backoff on disconnects. tokio is the right pick because the
work is async file I/O plus one long-lived TCP connection;
there is no hot loop to pin, no SPSC ring to drive.

The recorder explicitly trades latency (TCP head-of-line
blocking, kernel cwnd) for operational simplicity (one
socket, no NAK state machine, no UDP rmem tuning). Archival
runs offline of the GW→ME→GW critical path, so the tradeoff
is the obvious one. See [`../docs/concepts/02-tiles-and-pinning.md`](../docs/concepts/02-tiles-and-pinning.md)
for when each runtime applies.
