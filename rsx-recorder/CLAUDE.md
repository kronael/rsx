# CLAUDE.md — rsx-recorder

Local to `rsx-recorder/`; the root `../CLAUDE.md` applies. The recorder is
the archival replication consumer: it tails a producer's WAL over TCP and
writes date-partitioned archive files. Specs: `specs/2/10-replication.md`,
`48-wal.md`.

## Rules

- Simple by design: a replication client, a tip file, daily files. No
  exchange logic, no transformation; archive bytes are WAL bytes.
- The hot WAL is a short retention tier; the archive owns long-term
  durability. Retention is bounded (`RSX_RECORDER_RETAIN_DAYS`); an unbounded
  archive fills the disk and takes the box down with it.
- Moving or garbage-collecting WAL segments is never the WAL writer's job;
  it is this consumer's and the operator's.
- Replay fallback order: local hot WAL, then archive endpoints in order, back
  up the chain once the active one is exhausted. A list of endpoints serves
  parts of the archive; one of them has the record.
- Reading archives back is a Rust service over the same records, never a
  Python reimplementation of the decoder.
- Health reflects the link: a recorder connected and caught up is healthy
  however quiet the stream; one that is not caught up for
  `RSX_RECORDER_STALL_SECS` is not.

## Docs

`README.md` (what, `Running`), `ARCHITECTURE.md` (client, tip, files),
`compare/README.md` (vs named alternatives).
