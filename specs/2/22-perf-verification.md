---
status: partial
---

# Performance verification

How the project measures, gates, and surfaces latency
numbers. The split is:

- **Unit benches** (Criterion) for component-level numbers.
  Already comprehensive (60+ benches across 8 crates).
- **Regression gate** (`scripts/bench-gate.sh`) catches
  >10% slowdowns locally. Already shipped.
- **Playground latency endpoint** for live observation.
  Already shipped (`/api/latency`, `/x/risk-latency`,
  `/x/latency-regression`, with backing Playwright tests).
- **End-to-end harness** that asserts GW→ME→GW round-trip.
  **Not yet shipped.** Plan in §4.
- **Gateway mode endpoint** for the live/offline badge.
  **Not yet shipped.** Plan in §5.

The previous version of this spec said "deferred" for the
E2E harness without specifying a plan. This version
specifies one.

## Table of contents

- [1. Unit benches (shipped)](#1-unit-benches-shipped)
- [2. Regression gate (shipped)](#2-regression-gate-shipped)
- [3. Playground latency pipeline (shipped)](#3-playground-latency-pipeline-shipped)
- [4. End-to-end harness (planned)](#4-end-to-end-harness-planned)
- [5. Gateway mode endpoint (planned)](#5-gateway-mode-endpoint-planned)
- [6. What the numbers mean](#6-what-the-numbers-mean)

---

## 1. Unit benches (shipped)

`cargo bench --workspace` runs 60+ Criterion benches across
8 crates. The headline measurements:

| Operation                                                    | ns    | Source                                  |
|--------------------------------------------------------------|-------|-----------------------------------------|
| Match single fill                                            | 54    | `rsx-book/benches/book_bench.rs`        |
| `WalWriter::append` (Vec extend, no disk I/O)                | 31    | `rsx-dxs/benches/wal_bench.rs`          |
| WAL flush + fsync 64 KB                                      | ~24 µs| `rsx-dxs/benches/wal_bench.rs`          |
| Protocol-record encode (StatusMessage / Nak / Heartbeat)     | 43    | `rsx-dxs/benches/cmp_bench.rs`          |
| Protocol-record decode (one record)                          | 9     | `rsx-dxs/benches/cmp_bench.rs`          |
| `FillRecord` encode                                          | 23    | `rsx-messages/benches/encode_bench.rs`  |
| SPSC `push` / `pop` (rtrb)                                   | 50–170| `rsx-book/benches/book_bench.rs`        |

These are **single-thread, no-contention** numbers. They
are CPU costs of the operation in isolation, not throughput
under load.

`make perf` runs the full suite. `make bench-webui` runs the
React render benchmark for orderbook deltas (asserts p95
< 16 ms).

## 2. Regression gate (shipped)

`scripts/bench-gate.sh` parses Criterion estimates and
fails on >10% slowdown vs a saved baseline.

```
make bench-save   # save current run as baseline
make bench-gate   # diff current run against baseline
```

Implementation: pure bash + jq, no Python or npm. Reads
`target/criterion/<name>/new/estimates.json`, extracts
`mean.point_estimate`, compares against
`tmp/bench-baseline.json`. Prints a per-bench table with
ratio and PASS/FAIL.

The baseline lives at `bench-baseline.json` at the repo
root and is checked in. Update it locally with
`make bench-save`, then commit the result so CI runs
have a stable reference. The script exits cleanly with a
guidance message when no baseline exists yet, so a fresh
clone doesn't fail the gate before anyone has run
`bench-save`.

## 3. Playground latency pipeline (shipped)

The playground records gateway round-trip latency on every
order submission. State and endpoints:

- `order_latencies: list[float]` — bounded ring of the
  most recent 1 000 latencies, in microseconds. Populated
  by `send_order_to_gateway()` (`server.py:3757-3790`)
  with `time.perf_counter_ns()` deltas.
- `GET /api/latency` (`server.py:5409-5422`) — returns
  `{count, p50, p95, p99, min, max}` over the ring.
- `GET /x/risk-latency` — HTMX partial, renders a
  latency card (uses `pages.render_risk_latency()`).
- `GET /x/latency-regression` — HTMX partial, renders
  the regression-comparison card.

Tests: `rsx-playground/tests/play_latency.spec.ts` has 13
tests, no skips, with real assertions. Notable:
- `latency endpoint returns stats after orders` —
  submits 5 orders, asserts stats populated.
- `risk latency card renders` and
  `latency regression card renders` — assert HTMX partials
  render without error.

Plus the broader endpoint-latency suite (page load < 4 s,
HTMX partial < 500 ms, order submit < 1 s, JSON API
< 200 ms, etc.).

## 4. End-to-end harness (shipped)

A continuous probe that asserts the full GW → ME → GW
round-trip and surfaces the result in `/api/latency`.

**What it does:**

1. `POST /api/latency-probe?symbol_id=10` opens a
   WebSocket to the gateway with a fresh JWT, submits a
   probe order at `bestAsk * 1.01` so it crosses the
   maker book, and waits for the `F` (fill) frame on
   the same WebSocket.
2. The round-trip is `time.perf_counter_ns()` from the
   instant before `ws.send_json` to the instant after
   the F frame is received. Includes Python overhead +
   gateway WS handler + JWT decode + CMP encode +
   risk pre-trade + ME match + reverse path.
3. The result is appended to the `e2e_latencies` ring
   (capped at 1 000) and exposed as an `e2e` block in
   `/api/latency`:
   ```json
   { "count": 20, "p50": 47, "p95": 78,
     "p99": 153, "min": 41, "max": 161,
     "e2e": { "count": 20, "p50": 220, "p95": 340,
              "p99": 480, "min": 180, "max": 510 } }
   ```
4. The Latency dashboard tab at `./latency` renders the
   probe results live with a "Run one probe" button so
   visitors can see a real round-trip number.

**Limits:**
- The probe orders are real orders with a `probe-` cid
  prefix; they consume real liquidity and produce real
  fills. The maker must be running.
- Python + aiohttp adds 50–200 µs to the floor compared
  to a native Rust client, so the probe number is an
  upper bound on the gateway-to-gateway path.
- The probe path uses the dev `RSX_GW_JWT_SECRET`. The
  loopback assumption holds because the playground
  binds to localhost.

**What's still missing:**
- A native-Rust probe that excludes Python overhead.
  Future work — when it lands, the existing `e2e` block
  in `/api/latency` should grow a `native_e2e` peer.
- Throughput-under-load test. The current probe is a
  one-shot; a sustained-load harness would need to run
  N probes/second concurrently and report tail latency.
  This is the next step on the perf roadmap.

## 5. Gateway mode endpoint (planned)

A single endpoint the dashboard polls to decide whether
to display "GW: live" (green) or "GW: offline" (amber).

```python
@app.get("/api/gateway-mode")
async def api_gateway_mode():
    reachable = await _probe_gateway_tcp()
    return {
        "mode": "live" if reachable else "offline",
        "url": GATEWAY_URL,
    }
```

`_probe_gateway_tcp()` already exists in `server.py`. The
endpoint itself does not. Adding it is task F2 in the
ship project; it's grouped with the latency dashboard
work because both surface the same "is the system
actually running" question.

Test:

```python
def test_api_gateway_mode(client):
    resp = client.get("/api/gateway-mode")
    assert resp.status_code == 200
    data = resp.json()
    assert data["mode"] in ("live", "offline")
    assert "url" in data
```

## 6. What the numbers mean

Three classes of numbers appear in this repo. Be careful
not to confuse them.

**Microbench numbers** (54 ns match, 31 ns
`WalWriter::append` to an in-memory `Vec` before fsync,
…). These are the CPU cost of the operation **in
isolation** with warm caches and no contention. They
are real, reproducible, and useful for catching
regressions, but they are not the latency a packet
experiences in production.

**End-to-end design budgets** (<50 µs GW→ME→GW). These
are aspirational targets derived from summing component
budgets (gateway parse + CMP encode + UDP roundtrip +
risk + match + WAL append + reverse path). The total
fits inside 50 µs by component math, but until the §4
harness lands, the system has not been measured under
contention end-to-end. Treat these as goals, not
measurements.

**Playground gateway-roundtrip** (the `/api/latency`
numbers). These measure the time from `send_order_to_gateway`
in the Python server until the gateway responds with
HTTP. That includes Python, aiohttp, gateway WS handler,
JSON parse, and JWT validation, but **not** ME or risk
— the gateway can respond before the order is matched.
These numbers are useful for "is the gateway alive,"
not for the matching latency claim.

The §4 E2E harness is the bridge between microbench
numbers and the design budget. Until it lands, the spec
keeps the budget but explicitly disclaims it as
unmeasured.
