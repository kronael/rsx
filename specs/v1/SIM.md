# SIM.md — rsx-sim Load Generator

## Purpose

`rsx-sim` drives synthetic order flow against the gateway to
verify latency targets under load. It is not latency-critical
itself; it uses tokio (not monoio) so async machinery is fine.

Primary deliverable: confirm p99 new-order-to-fill/reject RTT
< `RSX_SIM_TARGET_P99_US` microseconds under `RSX_SIM_USERS`
concurrent sessions at sustained throughput.

---

## CLI / Environment

All config via env vars (no TOML, no flags at runtime):

| Variable | Default | Description |
|---|---|---|
| `RSX_SIM_GW_URL` | `ws://127.0.0.1:8080` | Gateway WS endpoint |
| `RSX_SIM_USERS` | `10` | Concurrent user sessions |
| `RSX_SIM_RATE` | `1000` | Target orders/sec across all users |
| `RSX_SIM_DURATION_S` | `60` | Run duration in seconds |
| `RSX_SIM_TARGET_P99_US` | `50000` | p99 latency target (us) |
| `RSX_SIM_SYMBOL` | `BTC-PERP` | Symbol to trade |
| `RSX_SIM_JWT` | (required) | JWT for auth (shared by all users) |

Exit codes: `0` if final p99 < target, `1` otherwise.

Makefile target:

```
make load-test
```

---

## Connection Model

One tokio task per user session:

```
main
 ├─ spawn user_task(0, config)
 ├─ spawn user_task(1, config)
 │   ...
 └─ spawn user_task(N-1, config)
```

Each user_task:
1. Opens a WS connection to `RSX_SIM_GW_URL` with
   `Authorization: Bearer <JWT>` upgrade header.
2. Spawns a read loop (tokio task) for incoming frames.
3. Sends orders at its share of the global rate
   (`RSX_SIM_RATE / RSX_SIM_USERS` orders/sec per user).
4. Reconnects with 1s backoff on disconnect.

Rate control: `tokio::time::interval` per user task. No
token-bucket; interval drift is acceptable for load testing.

Heartbeat: gateway sends `{H:[ts]}` every 5s. User task must
echo `{H:[ts]}` within 10s or gateway closes the connection.
The read loop handles heartbeats transparently.

---

## Order Profile

Each sent order:

- **Symbol**: `RSX_SIM_SYMBOL`
- **Side**: alternating or random, 50% buy / 50% sell
- **Price**: mid-price +/- random offset within 5 ticks.
  Mid is fetched from the public WS `{B:[...]}` BBO frame
  on first connect, then updated on each BBO update.
  If no BBO yet, use a hard-coded default (100000 for BTC-PERP).
- **Qty**: 1 lot (raw qty = 1)
- **TIF**: `IOC` — avoids resting orders and book buildup
- **cid**: monotonic counter per session (string, <=20 chars)
- **ro/po**: both false

Wire frame sent:

```json
{"N":["BTC-PERP","B","99995","1","sim-0-42","IOC",false,false]}
```

Prices are sent as raw i64 strings (fixed-point), matching
gateway expectation. Tick size from `RSX_SIM_SYMBOL` config.

---

## Latency Measurement

Latency = wall-clock delta from send to first response for
that cid (fill `{F:[...]}`, update `{U:[...]}`, or error
`{E:[...]}`).

Each user task maintains a `HashMap<cid, Instant>`. On send,
record `Instant::now()`. On receive, look up cid and compute
elapsed. Push to a shared `Arc<Mutex<LatencyAccumulator>>`.

`LatencyAccumulator` collects raw samples (u64 microseconds)
in a `Vec`. No windowing; final stats computed once at end.
Periodic reports use a snapshot clone.

---

## Reporting

Every 5 seconds, print a stats line to stdout:

```
Feb 27 14:00:05 [sim] sent=5000 recv=4987 p50=1200us p99=8400us
```

Final summary on exit:

```
Feb 27 14:01:00 [sim] DONE duration=60s sent=60000 recv=59820
  p50=1150us p99=8200us p99.9=12000us target=50000us PASS
```

`PASS` if p99 < target, `FAIL` otherwise. Exit code follows.

Log format: Unix log prefix (`Mon DD HH:MM:SS`), lowercase
message, values inline.

---

## Implementation Plan

Changes to `rsx-sim/src/main.rs`:

1. Parse env vars (already done for `GW_URL`, `USERS`).
   Add `RATE`, `DURATION_S`, `TARGET_P99_US`, `SYMBOL`, `JWT`.

2. Build `tokio::runtime::Runtime`, spawn N `user_task` futures.

3. `user_task`: connect WS, read loop task, send loop with
   interval. Reconnect on error.

4. Shared `Arc<Mutex<Vec<u64>>>` for latency samples.
   Periodic reporter task clones and computes percentiles.

5. Main waits `DURATION_S`, then signals shutdown (via
   `CancellationToken` from `tokio_util`). Collect final stats,
   print summary, exit with code.

6. Percentile helper: sort clone, index by fraction. No
   histogram crate needed for a load tester.

New dependencies (add to `rsx-sim/Cargo.toml`):
- `tokio-util` (cancellation)
- `rand` (price jitter, side selection)

---

## Acceptance Criteria

- Connects N sessions concurrently, maintains connections.
- Sends at target rate (+/- 10%).
- All responses matched to sends by cid; unmatched logged.
- Heartbeat echoed; no disconnect due to missing echo.
- p99 reported correctly (validated against wall clock).
- Exit 0 when p99 < target on a local gateway under no load.
- Exit 1 when target deliberately set to 1us.
- No panics, no unbounded memory growth over 60s run.

---

## Testing

Unit tests (`tests/sim_test.rs`):
- Percentile calculation on known distributions.
- cid generation uniqueness across sessions.
- Frame serialization matches WEBPROTO.md format.

Smoke test (`make load-test`):
- Starts local gateway + ME, runs sim for 10s, checks exit 0.
- Requires `RSX_SIM_JWT` set in environment.

No mocks for WS; unit tests cover pure functions only.
