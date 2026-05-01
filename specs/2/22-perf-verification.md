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

| Operation                   | ns    | Source                                |
|-----------------------------|-------|---------------------------------------|
| Match single fill           | 54    | `rsx-book/benches/book_bench.rs`      |
| WAL append (in-memory)      | 31    | `rsx-dxs/benches/wal_bench.rs`        |
| WAL flush + fsync 64 KB     | ~24 µs| `rsx-dxs/benches/wal_bench.rs`        |
| CMP encode (one record)     | 43    | `rsx-gateway/benches/gateway_bench.rs`|
| CMP decode (one record)     | 9     | `rsx-gateway/benches/gateway_bench.rs`|
| SPSC `push` / `pop` (rtrb)  | 50–170| `rsx-book/benches/book_bench.rs`      |

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

The baseline lives at `tmp/bench-baseline.json` —
gitignored, developer-local. CI does not currently
enforce regressions across runs (no shared baseline). For
that, see §7 of `.ship/12-SHOWCASE-HONEST/PROJECT.md`
task G.

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

## 4. End-to-end harness (planned)

What's missing today: a continuous test that asserts the
full GW → ME → GW round-trip is below the 50 µs design
budget.

The pieces are already in place:

- Gateway records `ts_ns: time_ns()` on every order
  accept (`rsx-gateway/src/handler.rs`).
- Risk forwards `timestamp_ns` to the matching engine
  (`rsx-gateway/src/route.rs`, `rsx-risk/src/main.rs`).
- ME timestamps fill emission (`rsx-matching/src/main.rs:432,
  819, 825, 830`).
- Fills get the original `ts_ns` echoed back to the
  gateway, where the WS write timestamp closes the loop.

What's missing: correlation. The probe needs to (a)
choose a `cid` ("client order id") for the probe, (b) tag
it as a probe so the gateway records both the accept
timestamp and the fill-receive timestamp, (c) compute
`fill_ts_ns - accept_ts_ns` and append to a separate
`e2e_latencies` ring, (d) surface in `/api/latency` as an
`e2e_p50/p95/p99` block.

**Plan:**

1. Reserve a `cid` prefix `"probe-"` for E2E latency
   probes (gateway already accepts arbitrary cids).
2. In `send_order_to_gateway`, when the cid starts with
   `"probe-"`, record the start_ns, attach a future,
   resolve it in the fill handler, store the delta in
   `e2e_latencies`.
3. Extend `/api/latency` JSON with an `e2e` block:
   `{count, p50, p95, p99}` (microseconds).
4. Add a Playwright test that submits 20 probe orders
   then asserts the `/api/latency` `e2e.p99` is below
   some configurable threshold (default 200 µs to allow
   slack on shared CI runners).

Tracked as task F1 in `.ship/12-SHOWCASE-HONEST/`. The
50 µs design budget stays a budget until this lands.

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

**Microbench numbers** (54 ns match, 31 ns WAL append,
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
