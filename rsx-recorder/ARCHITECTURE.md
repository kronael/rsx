# rsx-recorder Architecture

Archival replication consumer process. Connects to a replication
server, receives all WAL records, writes to date-partitioned
archive files.

## Components

- `RecorderState` -- archive directory, current file handle,
  write buffer, daily rotation
- `ReplicationConsumer` (from rsx-cast) -- TCP client with tip
  persistence and exponential backoff

## Data Flow

```
ME WAL --> ReplicationService --> [TCP] --> ReplicationConsumer
                                              |
                                         RecorderState
                                              |
                                    {stream_id}_{date}.wal
```

## Record Processing

1. Connects to replication producer (replay server) using `ReplicationConsumer`
2. Receives `RawWalRecord` via callback
3. Buffers records, flushes to disk every 1000 records
4. Rotates output file daily (`{stream_id}_{date}.wal`)
5. Persists consumption tip for idempotent restart

## File Layout

```
archive/{stream_id}_{YYYY-MM-DD}.wal
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
is the obvious one. See [`../notes/tiles.md`](../notes/tiles.md)
for when each runtime applies.
