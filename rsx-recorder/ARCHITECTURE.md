# rsx-recorder Architecture

Archival DXS consumer process. Connects to a DXS replay
server, receives all WAL records, writes to date-partitioned
archive files.

## Components

- `RecorderState` -- archive directory, current file handle,
  write buffer, daily rotation
- `DxsConsumer` (from rsx-dxs) -- TCP client with tip
  persistence and exponential backoff

## Data Flow

```
ME WAL --> DxsReplayService --> [TCP] --> DxsConsumer
                                              |
                                         RecorderState
                                              |
                                    {stream_id}_{date}.wal
```

## Record Processing

1. Connects to DXS producer (replay server) using `DxsConsumer`
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
