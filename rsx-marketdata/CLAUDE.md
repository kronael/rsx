# CLAUDE.md — rsx-marketdata

Local to `rsx-marketdata/`; the root `../CLAUDE.md` applies. Marketdata keeps
a shadow `rsx-book` per symbol from the ME casting stream and publishes L2,
BBO and trades over public WebSocket to the dashboard and `rsx-glass`.
Specs: `specs/2/16-marketdata.md`, `59-latency-observability.md`; tests
`40-testing-marketdata.md`.

## Rules

- Off the order critical path: the ME pushes fire-and-forget and the order
  round-trip never waits here. A slow subscriber NEVER back-pressures
  matching: bound the outbound (`RSX_MD_MAX_OUTBOUND`) and drop the
  subscriber, never the ME's packets.
- Pinned (`RSX_MD_CORE_ID`) to keep up with the casting firehose; a receive
  buffer overflow is a FAULTED replay storm, not a slow page.
- A monoio reactor for the many public subscriber fds; the replication
  (replay) client may stay on tokio, it is off the hot path.
- Structurally the mirror of `rsx-gateway` (fan-out where the gateway fans
  in): when you know one you know the other. A structural change is made in
  both or recorded in `BUGS.md`.
- Every snapshot and delta carries `u`, the ME height (last processed event
  seq). A subscription to the delta stream is answered with a snapshot
  first. Snapshots are served here, never by the ME.
- The shadow book is `rsx-book` and L2 iterates a live book; a change to
  `rsx-book`'s scan API runs the workspace tests, not the book's alone.
- Market data streams live to both UIs (the playground dashboard and
  `rsx-glass`); a feature is not done until both show it.
- Latency legs are measured to marketdata too (spec 59); mark and index
  prices carry their observation age.

## Docs (which file answers which question)

| File | Question |
|---|---|
| `README.md` | what it publishes, `Running` (env), wire summary |
| `ARCHITECTURE.md` | how it is built: receivers per ME, shadow book, subscriptions, egress, replay |
| `marketdata.proto` | the public wire schema |
| `compare/README.md` | vs named alternatives |

## Keeper sections — do not regress

- The keep-up statement: a slow consumer never back-pressures matching.
- The `u` = ME height contract on every frame.
