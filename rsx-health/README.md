# rsx-health

A dependency-light `/health` `/ready` `/metrics` HTTP endpoint
for low-latency daemons, updated from the hot path with nothing
but relaxed atomic stores.

The daemon holds one `Arc<LoadGauges>` — a flat struct of
atomics — and updates it from its busy-spin loop with single
relaxed stores (`fetch_add`, `store`): no mutex, no allocation,
no syscall per message. A health server runs on a *separate*
`std::thread`, reads those atomics with relaxed loads when a
request arrives, and answers `GET /health` / `/ready` / `/metrics`
with hand-rolled HTTP. The HTTP is written by hand precisely so
the crate pulls in no web framework and no async runtime — just
`tracing` for the two log lines it emits.

## What it provides

- **`LoadGauges`** — the atomics a daemon updates on the hot path
  (liveness/readiness bools, ring used/cap pairs, throughput
  counters, a `state_idx`). `LoadGauges::new()` returns an
  `Arc<Self>`; the daemon keeps one clone, the health thread the
  other.
- **`DaemonState`** — `Boot`/`WarmCatchup`/`Live`/`Faulted`/
  `Running`, stored as a `u64` in `state_idx`. `set_state`
  writes it; `state_label` decodes it to a string.
- **`HealthSnapshot`** — a point-in-time view (`live`, `ready`,
  `saturation`, per-ring `QueueGauge`s, named `CounterGauge`s,
  `state`). `to_json()` renders it as a flat JSON string with no
  serde.
- **`QueueGauge` / `CounterGauge`** — the per-ring and named-
  counter entries inside a snapshot.
- **`spawn_health_server(addr, snapshot)`** — spawn the health
  thread. `snapshot: Fn() -> HealthSnapshot` is called once per
  request; it reads the daemon's `Arc<LoadGauges>`. If the bind
  fails, it logs a warning and returns — the daemon runs on
  without health endpoints.

## Endpoints

| Path | Status | Purpose |
|---|---|---|
| `GET /health` | 200 / 503 | liveness — restart the pod on 503 |
| `GET /ready` | 200 / 503 | readiness — shed traffic (remove from Service) on 503 |
| `GET /metrics` (alias `/loadz`) | 200 + JSON | full snapshot for HPA / manual inspection |
| anything else | 404 | |

## Usage

```rust
use rsx_health::LoadGauges;
use rsx_health::HealthSnapshot;
use rsx_health::QueueGauge;
use rsx_health::spawn_health_server;
use std::sync::atomic::Ordering;

let gauges = LoadGauges::new();
gauges.resp_ring_cap.store(cap, Ordering::Relaxed);

// Health thread: reads the same Arc on each request.
let g = gauges.clone();
spawn_health_server(addr, move || {
    let used = g.resp_ring_used.load(Ordering::Relaxed);
    let cap = g.resp_ring_cap.load(Ordering::Relaxed);
    HealthSnapshot {
        live: g.live.load(Ordering::Relaxed),
        ready: g.ready.load(Ordering::Relaxed),
        saturation: if cap > 0 { used as f64 / cap as f64 } else { 0.0 },
        queues: vec![QueueGauge { name: "resp", used, cap }],
        counters: vec![],
        state: g.state_label(),
    }
});

// Hot path: relaxed stores only.
gauges.orders_processed.fetch_add(1, Ordering::Relaxed);
gauges.resp_ring_used.store(n, Ordering::Relaxed);
gauges.live.store(true, Ordering::Relaxed);
```

## Design guarantees

- **Hot path is store-only.** The daemon never reads the gauges,
  never locks, never allocates — every update is one relaxed
  atomic store on a counter it already tracks. The health thread
  owns all the reads and all the formatting.
- **Health server is off the hot path.** It does `accept` +
  `read` + atomic loads + `write` on its own thread; a slow or
  hung client cannot touch the daemon's loop.
- **Bind failure is non-fatal.** A failed bind logs a warning and
  returns; the endpoint is optional and the daemon keeps serving.
- **`Relaxed` ordering is intentional.** Gauges are advisory
  telemetry, not synchronization; no gauge gates correctness, so
  no fence is needed.

## Dependencies

- `tracing` — the two log lines (bind failure, listening). That
  is the whole dependency list: the HTTP request parse and
  response are hand-rolled, and the JSON is built with
  `format!`. No web framework, no serde, no async runtime.

## Testing

```
cargo test -p rsx-health
```

Unit tests in `src/lib.rs` (`#[cfg(test)]`): JSON rendering,
200/503/404 status mapping, relaxed-gauge round-trip, and an
end-to-end server test that binds a dynamic port and drives
`/health` `/ready` `/metrics` `/unknown` over a real `TcpStream`.

## MSRV

Rust 1.78+ on stable, edition 2021. No nightly features.

## See also

- ARCHITECTURE.md — the gauge model, the request loop, and the
  hand-rolled HTTP.

## License

Internal-use crate within the wider rsx exchange project.
Licensed under `MIT OR Apache-2.0`. Not published to
crates.io; distribution is the maintainer's decision.
