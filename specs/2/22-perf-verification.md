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
- [7. Measurement caveats / limitations](#7-measurement-caveats--limitations)
- [8. Future implementation](#8-future-implementation)

---

## 1. Unit benches (shipped)

`cargo bench --workspace` runs 60+ Criterion benches across
8 crates. The headline measurements:

| Operation                                                    | ns    | Source                                  |
|--------------------------------------------------------------|-------|-----------------------------------------|
| Match single fill                                            | 54    | `rsx-book/benches/book_bench.rs`        |
| `WalWriter::prepare` + `append_framed` (Vec extend, no disk I/O) | 31    | `rsx-cast/benches/wal_bench.rs`          |
| WAL flush + fsync 64 KB                                      | ~24 µs| `rsx-cast/benches/wal_fsync_bench.rs`    |
| Protocol-record encode (Nak / CastHeartbeat)                 | 43    | `rsx-cast/benches/cast_bench.rs`         |
| Protocol-record decode (one record)                          | 9     | `rsx-cast/benches/cast_bench.rs`         |
| `FillRecord` encode                                          | 23    | `rsx-messages/benches/encode_bench.rs`  |
| SPSC `push` / `pop` (rtrb)                                   | 50–170| `rsx-book/benches/book_bench.rs`        |

These are **single-thread, no-contention** numbers. They
are CPU costs of the operation in isolation, not throughput
under load.

`make perf` runs the full suite. `make bench-webui` runs the
React render benchmark for orderbook deltas (asserts p95
< 16 ms).

## 2. Regression gate (shipped)

Two gates live in `scripts/`. Both fail on >10% slowdown
against a saved reference, but they measure different
things and have different operational requirements.

**`scripts/bench-gate.sh` — Criterion microbench gate.**
Parses Criterion estimates, compares against
`bench-baseline.json`. No cluster required.

```
make bench-save   # save current Criterion run as baseline
make bench-gate   # diff current run against baseline
```

**`scripts/bench-gate-e2e.sh` — E2E latency gate.**
Drives `latency-publish.sh`, rejects invalid accounting or missing samples,
then reads `e2e_us.p99` from `bench-baseline.json` and
compares against a **sealed** reference in
`bench-reference.json`. Requires the cluster to be live
(`./rsx-playground/playground start-all`).

```
make bench-gate-e2e-save  # snapshot current e2e_us as reference
make bench-gate-e2e       # fail if p99 regresses >10%
```

`bench-baseline.json` is rolling — `make latency-publish`
rewrites the `e2e_us` block on every run. `bench-reference.json`
only changes when the operator explicitly accepts a new
floor with `make bench-gate-e2e-save`. This split prevents
the silent baseline-creep that would hide regressions.

The publisher uses the corrected open-loop stress client against the external
gateway WebSocket route. It requires closed send and response accounting, no
pending outcomes, at least 95% terminal and achieved/offered ratios, and a
configurable accepted-sample floor. It records p50/p95/p99/p99.9/max,
accepted throughput, and loss counters, and labels the result `shared-host`.
Invalid runs cannot update either baseline or reference; performance
regression and measurement invalidity are reported separately. `RATE`,
`DURATION`, `N`, and `MIN_SAMPLES` configure practical local or staircase
steps without introducing a second load runner.

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

The existing `stress.py` workflow is an open-loop measurement: send pacing is
independent of response latency, and responses are drained after the send
window. Its report records offered, submitted, accepted, rejected by reason,
completed, timed out, pending, and error counts. A valid run closes both send
and response accounting, classifies at least 95% of submitted orders, has at
least one accepted latency sample, and leaves no pending orders. Percentiles
are `null` when no samples exist and include p50, p95, p99, p99.9, min, and
max. Correctness validity is checked before the configured p99 target; an
invalid measurement exits non-zero and is never a latency pass.

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
   gateway WS handler + JWT decode + casting encode +
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
`WalWriter::prepare` + `append_framed` to an in-memory
`Vec` before fsync, …). These are the CPU cost of the operation **in
isolation** with warm caches and no contention. They
are real, reproducible, and useful for catching
regressions, but they are not the latency a packet
experiences in production.

**End-to-end design budgets** (<50 µs GW→ME→GW). These
are aspirational targets derived from summing component
budgets (gateway parse + casting encode + UDP roundtrip +
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

## 7. Measurement caveats / limitations

Everything in §1-§4 is real and reproducible, but it was all
collected on the same kind of box, with the same kind of
network path. State that plainly:

- **Loopback / in-process, shared development machine.**
  Every number above — Criterion benches, the playground
  `/api/latency` numbers, and the §4 E2E probe — runs on a
  single shared dev machine, with gateway/risk/ME either
  loopback UDP or in-process. None of it crossed a real NIC
  or a real switch.
- **No tuned-host test.** There is no run with controlled IRQ
  placement, NUMA affinity, isolated CPUs (`isolcpus`), CPU
  frequency control (governor pinned / turbo disabled), or PTP
  time sync. The numbers reflect whatever the shared dev box's
  scheduler and clock happened to be doing at run time, not a
  host configured for latency measurement.
- **DPDK and AF_XDP are not implemented and not measured.**
  Where this repo discusses kernel-bypass networking (see
  `specs/2/56-network-edge-scaling.md`), it is design
  discussion of future work, not a measured code path. No
  latency or throughput number anywhere in this repo comes
  from a DPDK or AF_XDP run.
- **No sustained-load / soak test.** §4 notes the E2E probe is
  one-shot. There is no steady-state throughput test that runs
  the system under realistic sustained load for a realistic
  duration; all latency numbers are point-in-time, low-order
  samples, not a distribution measured under continuous
  production-like load.

Treat every latency number in this repo — including the <50 µs
design budget in §6 — as a development-machine, loopback-path
measurement (or, for the design budget, a component-math
estimate) until the work in §8 lands.

## 8. Future implementation

Not implemented, not scheduled to a specific milestone. Recorded
here so the gap stays visible instead of getting silently
implied by the existence of §1-§4.

- **Real-NIC, tuned-host test.** A dedicated (non-shared) host
  with IRQ affinity pinned off the hot cores, NUMA-local memory
  and NIC queues, `isolcpus` reserving the hot-path cores,
  fixed CPU frequency (governor `performance`, turbo disabled
  or accounted for), and PTP time sync for cross-host timestamp
  comparison. Needed before any cross-host latency number can
  be trusted.
- **DPDK / AF_XDP.** Kernel-bypass I/O as a swap-in under the
  existing tile interfaces (see architecture note in
  `CLAUDE.md` and the userspace-UDP prerequisite in
  `specs/2/56-network-edge-scaling.md` Part B). Today this is
  design discussion only — no implementation, no benchmark.
- **Sustained production-load test.** A soak harness that drives
  steady-state order flow at a fixed target rate for a realistic
  duration (minutes-to-hours, not one probe) and reports the
  resulting latency distribution and any degradation over time.
  This is distinct from the one-shot §4 probe and from the
  Criterion microbenches, both of which measure a single
  operation or a single round-trip, not sustained throughput.
