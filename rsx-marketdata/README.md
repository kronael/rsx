# rsx-marketdata

Market data binary. Publishes L2 depth, BBO, and trades
over WebSocket.

## What It Does

Maintains a shadow `Orderbook` per symbol (rsx-book) from
ME events received via casting/UDP. Aggregates streams from
multiple matching engines via `RSX_ME_CAST_ADDRS` (comma-
separated list, one CastReceiver per ME). Publishes real-time
L2 snapshots, deltas, BBO updates, and trade messages to subscribed
WebSocket clients as protobuf binary frames
(`marketdata.proto`). On per-stream sequence gap, resends the
full L2 snapshot to all subscribers of the affected symbol.

## Running

```
RSX_MD_LISTEN=0.0.0.0:8081 \
RSX_ME_CAST_ADDRS=127.0.0.1:9100 \
RSX_MD_MAX_SYMBOLS=64 \
RSX_MD_SNAPSHOT_DEPTH=10 \
cargo run -p rsx-marketdata
```

## Environment Variables

| Env Var | Default | Purpose |
|---------|---------|---------|
| `RSX_MD_LISTEN` | `0.0.0.0:8081` | WebSocket listen address |
| `RSX_ME_CAST_ADDRS` | `127.0.0.1:9100` | ME casting addresses, comma-separated, one receiver each |
| `RSX_ME_CAST_ADDR` | | single ME address, read when `RSX_ME_CAST_ADDRS` is unset |
| `RSX_ME_MD_SEND_ADDRS` | the ME cast addresses | each ME's sender socket for the marketdata stream, comma-separated in `RSX_ME_CAST_ADDRS` order; the receiver sends its NAKs there. A count that differs from the ME count exits 2 |
| `RSX_MD_MAX_SYMBOLS` | 64 | max symbol count |
| `RSX_MD_SNAPSHOT_DEPTH` | 10 | levels per side in an L2 snapshot |
| `RSX_MD_BOOK_CAPACITY` | 65536 | order slots per shadow book |
| `RSX_MD_BOOK_MID_PRICE` | 50000 | anchor price of a new shadow book |
| `RSX_MD_TICK_SIZE` | 1 | tick size of every shadow book |
| `RSX_MD_LOT_SIZE` | 1 | lot size of every shadow book |
| `RSX_MD_PRICE_DECIMALS` | 0 | price decimals |
| `RSX_MD_QTY_DECIMALS` | 0 | quantity decimals |
| `RSX_MD_MAX_OUTBOUND` | 1024 | max queued frames per client |
| `RSX_MD_REPLAY_ADDR` | unset | replication server for bootstrap and gap replay |
| `RSX_MD_STREAM_ID` | 1 | stream id for replay |
| `RSX_MD_TIP_FILE` | `./tmp/md.tip` | persisted replay tip |
| `RSX_MD_HEARTBEAT_INTERVAL_S` | 5 | server heartbeat interval |
| `RSX_MD_IDLE_TIMEOUT_S` | 10 | client idle timeout |
| `RSX_MD_CORE_ID` | unset | pin the main thread to this cpu |
| `RSX_MD_HEALTH_ADDR` | unset | health endpoint address |

## Deployment

- No auth (public feed) -- separate process from gateway
- Single-threaded monoio runtime; the main thread pins to
  `RSX_MD_CORE_ID` when it is set. Not on the GW→ME→GW critical path
- One casting/UDP receiver per matching engine
  (`RSX_ME_CAST_ADDRS`, comma-separated)
- No durable state (shadow book rebuilt from ME events)
- Optional replication bootstrap on startup for fast recovery
- Uses monoio (io_uring) -- requires Linux kernel 5.1+

## Testing

```
cargo test -p rsx-marketdata
```

14 integration test files under `tests/` (config, event routing,
heartbeat, ME cast addresses, multi-ME aggregation, records, replay,
replay drain, replay end to end, replay gap, shadow book, snapshot
consistency, state resync, subscription) and four unit test modules in
`src/`. Seq gap detection resends the L2 snapshot.
See `specs/2/40-testing-marketdata.md`.

## Dependencies

- `rsx-types` -- shared types
- `rsx-book` -- orderbook (shadow book)
- `rsx-cast` -- casting receiver, replication consumer
- `rsx-ws` -- WebSocket accept loop, upgrade and frame I/O

## Gotchas

- Shadow book is ephemeral. On restart without replication,
  the book starts empty and rebuilds from live ME events.
  Clients will see stale data until the book catches up.
- Sequence gaps in casting trigger automatic L2 snapshot resend.
  During the gap, the shadow book may be inconsistent.
- A client whose outbound queue is full gets the queue cleared and a
  fresh L2 snapshot. A BBO frame that meets a full queue is dropped.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) -- data flow, casting
  decode loop, publishing, seq gap detection
- `specs/2/16-marketdata.md`
