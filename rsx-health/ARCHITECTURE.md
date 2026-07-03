# rsx-health Architecture

Two halves that share one `Arc<LoadGauges>`: the daemon writes
atomics on its hot path; a separate health thread reads them and
serves HTTP. Nothing crosses between them except relaxed atomic
loads/stores — no channel, no lock, no shared allocator traffic.

## The gauge model

`LoadGauges` is a flat struct of atomics — `AtomicBool` for
`live`/`ready`, `AtomicU64` for ring used/cap pairs and
throughput counters, `AtomicI64` for signed lag, and an
`AtomicU64` `state_idx`. The daemon allocates one at startup via
`LoadGauges::new() -> Arc<Self>`, keeps a clone, and passes
another clone into the health server.

Hot path is **store-only**:

```rust
gauges.orders_processed.fetch_add(1, Ordering::Relaxed);
gauges.resp_ring_used.store(n, Ordering::Relaxed);
gauges.live.store(true, Ordering::Relaxed);
```

`Relaxed` is deliberate: the gauges are advisory telemetry, not
synchronization. No gauge gates a correctness decision, so no
acquire/release fence is required, and the store stays a single
cheap instruction with no memory-barrier cost on the hot loop.

Capacity fields (`*_ring_cap`) are written once at startup before
the health thread starts, so the snapshot closure can treat them
as effectively constant.

## DaemonState

`state_idx` holds a `DaemonState` (`Boot=0`, `WarmCatchup=1`,
`Live=2`, `Faulted=3`, `Running=4`) as a `u64`. The discriminants
are wire-visible — `/metrics` exposes the decoded label — so they
are stable integers. `set_state(s)` stores `s as u64`;
`state_label()` matches it back to `"warm_catchup"` / `"live"` /
`"faulted"` / `"running"` / `"unknown"`.

## Snapshot and JSON

`HealthSnapshot` is the read-side view the daemon assembles in
the `snapshot` closure: `live`, `ready`, `saturation` (highest
ring occupancy, for an HPA), a `Vec<QueueGauge>`, a
`Vec<CounterGauge>`, and the `state` label. `to_json()` builds a
flat JSON string by hand with `format!` into a preallocated
`String` — the structure is fixed and shallow, so serde would be
pure overhead.

## The health thread

`spawn_health_server(addr, snapshot)` spawns a named
`std::thread` that:

1. `TcpListener::bind(addr)`. On failure, `warn!` and return —
   the endpoint is optional; the daemon keeps running.
2. Loop over `listener.incoming()`; on accept error, `warn!` and
   continue.
3. `serve_one` per connection: read up to 256 bytes, parse the
   request line for the path (`parse_path`), strip any query
   string, and match:
   - `/health` → 200 if `snapshot().live` else 503
   - `/ready` → 200 if `snapshot().ready` else 503
   - `/metrics` | `/loadz` → 200 + `snapshot().to_json()`
   - else → 404
4. Write a hand-built HTTP/1.1 response (`Connection: close`,
   `Content-Length`, JSON body).

The whole path is `accept` + `read` + atomic loads + `write` on
this dedicated thread. A slow client blocks only the health
thread, never the daemon's hot loop. The request buffer is a
fixed 256-byte stack array; a request larger than that is
truncated, which is fine for the three known paths.

## Runtime

None. The server is blocking `std::net` on a plain
`std::thread`; the daemon's runtime (busy-spin, monoio, or tokio)
is irrelevant because the two sides communicate only through the
`Arc<LoadGauges>` atomics.
