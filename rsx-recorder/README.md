# rsx-recorder

Archival consumer binary. Stores WAL records to daily files.

## What It Does

Connects to a DXS replay server, receives all WAL records,
writes them to date-partitioned archive files for historical
storage.

## Running

```
RSX_RECORDER_STREAM_ID=1 \
RSX_RECORDER_PRODUCER_ADDR=127.0.0.1:9200 \
RSX_RECORDER_ARCHIVE_DIR=./archive \
RSX_RECORDER_TIP_FILE=./tmp/recorder.tip \
cargo run -p rsx-recorder
```

## Environment Variables

| Env Var | Purpose |
|---------|---------|
| `RSX_RECORDER_STREAM_ID` | Stream ID to consume |
| `RSX_RECORDER_PRODUCER_ADDR` | DXS replay server address |
| `RSX_RECORDER_ARCHIVE_DIR` | Archive output directory |
| `RSX_RECORDER_TIP_FILE` | Tip persistence file path |

## Deployment

- One instance per stream (typically one per ME)
- Needs network access to DXS replay server
- Archive directory needs write access and sufficient disk
- Tip file enables idempotent restart (resumes from last seq)

## Testing

No dedicated tests. Depends on rsx-dxs client/server tests.
See `specs/v1/TESTING-DXS.md`.

## Dependencies

- `rsx-dxs` -- DxsConsumer, WAL record types

## Gotchas

- No library crate -- `main.rs` only.
- Flush is every 1000 records, not time-based. Low-throughput
  streams may have delayed writes.
- Daily file rotation is at UTC midnight. Clock skew can
  cause records to land in wrong day's file.
- On restart, replays from tip+1. If the tip file is deleted,
  full replay from seq 0 occurs.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) -- data flow, record
  processing, file layout
