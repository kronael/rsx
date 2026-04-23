---
status: shipped
---

# ARCHIVE (WAL Offload + Replay)

Archive serves historical WAL records from flat files on disk. It is used when hot DXS retention is insufficient.

## Purpose

- Provide infinite retention beyond the 10‑minute hot WAL window.
- Serve replay streams from archived WAL files.

## Deployment

- **Single authoritative archive per WAL stream.**
- Other nodes may keep mirrors, but only one archive is authoritative.

## API (WAL/TCP)

- TCP byte stream of WAL records (header + payload).
- `ReplayRequest` semantics match DXS.md (from_seq).

## Recovery Lookup Order

When recovering from `from_seq_no`:

1. **Authoritative archive** for that WAL stream.
2. **Primary producer DXS** (hot WAL tail).

The consumer continues from the last received `seq` when switching sources.

## File Layout

Same format as DXS WAL files:

```
archive/{stream_id}/{stream_id}_{first_seq}_{last_seq}.wal
```

Flat files are sequential and length‑free (fixed‑record).

## Gap Handling

- If the archive has a **partial gap** (missing seq range), **fail fast** and require a full snapshot.
- If archive does not contain `from_seq_no`, **fail fast**.

## Notes

- Archive is read‑only.
