# rsx-marketdata

Market data binary. Publishes L2 depth, BBO, and trades
over WebSocket.

## What It Does

Maintains a shadow `Orderbook` per symbol (rsx-book) from
ME events received via casting/UDP. Aggregates streams from
multiple matching engines via `RSX_ME_CAST_ADDRS` (comma-
separated list, one CastReceiver per ME). Publishes real-time
L2 snapshots, BBO updates, and trade messages to subscribed
WebSocket clients. On per-stream sequence gap, resends the
full L2 snapshot to all subscribers of the affected symbol.

## Running

```
RSX_MKT_LISTEN_ADDR=0.0.0.0:8081 \
RSX_MKT_CAST_ADDR=127.0.0.1:9300 \
RSX_ME_CAST_ADDR=127.0.0.1:9100 \
RSX_MKT_MAX_SYMBOLS=64 \
RSX_MKT_SNAPSHOT_DEPTH=20 \
cargo run -p rsx-marketdata
```

## Environment Variables

| Env Var | Purpose |
|---------|---------|
| `RSX_MKT_LISTEN_ADDR` | WebSocket listen address |
| `RSX_MKT_CAST_ADDR` | casting bind address |
| `RSX_ME_CAST_ADDR` | ME casting address |
| `RSX_MKT_MAX_SYMBOLS` | Max symbol count |
| `RSX_MKT_SNAPSHOT_DEPTH` | L2 snapshot depth |
| `RSX_MKT_MAX_OUTBOUND` | Max queued messages per client |
| `RSX_MKT_REPLAY_ADDR` | replication server (optional) |
| `RSX_MKT_STREAM_ID` | Stream ID for replay |
| `RSX_MKT_HEARTBEAT_INTERVAL_MS` | Server heartbeat interval |
| `RSX_MKT_HEARTBEAT_TIMEOUT_MS` | Client heartbeat timeout |

## Deployment

- No auth (public feed) -- separate process from gateway
- Single-threaded monoio runtime, busy-spin loop (no
  `core_affinity`; not on the GW→ME→GW critical path)
- One casting/UDP receiver per matching engine
  (`RSX_ME_CAST_ADDRS`, comma-separated)
- No durable state (shadow book rebuilt from ME events)
- Optional replication bootstrap on startup for fast recovery
- Uses monoio (io_uring) -- requires Linux kernel 5.1+

## Testing

```
cargo test -p rsx-marketdata
```

12 test files: config, event_routing, heartbeat,
me_cmp_addrs, protocol, replay, replay_e2e, seq_gap,
shadow, snapshot_consistency, state_resync, subscription.
Seq gap detection with automatic L2 snapshot resend.
See `specs/2/40-testing-marketdata.md`.

## Dependencies

- `rsx-types` -- shared types
- `rsx-book` -- orderbook (shadow book)
- `rsx-cast` -- casting receiver, replication consumer

## Gotchas

- Shadow book is ephemeral. On restart without replication,
  the book starts empty and rebuilds from live ME events.
  Clients will see stale data until the book catches up.
- Sequence gaps in casting trigger automatic L2 snapshot resend.
  During the gap, the shadow book may be inconsistent.
- Slow clients get messages dropped silently. They rely on
  seq gap detection to trigger snapshot resync.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) -- data flow, casting
  decode loop, publishing, seq gap detection
- `specs/2/16-marketdata.md`
