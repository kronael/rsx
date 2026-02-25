# Spec: Performance Verification

Close the gap between claims and evidence. Five
deliverables that make every latency/throughput claim
testable and CI-gated.

## Status quo

| Claim | Evidence today |
|---|---|
| <50us GW->ME->GW | Spec only, never measured |
| <500ns ME match | Criterion bench exists, not in CI |
| 10K orders/sec sustained | Spec only, rsx-sim is a stub |
| Criterion regression >10% | make perf runs, no gate |
| Playground uses real GW | Already does when GW is up |
| Playground latency stats | Server tracks order_latencies (us) but no JSON API; /api/latency 404s; play_latency tests skip or assert nothing; latency regression card renders but values never verified |

## Deliverable 1: E2E latency harness

**What:** A Rust integration test that measures actual
GW->Risk->ME->Risk->GW round-trip latency on localhost.

**Where:** `rsx-gateway/tests/e2e_latency_test.rs`

**How:**
- Start ME, Risk, Gateway as child processes (or in-process
  with monoio threads)
- Connect a WS client to gateway
- Submit a crossing order pair (buy above best ask)
- Measure wall-clock from WS send to fill received
- Repeat 1000 times, compute p50/p99/p99.9
- Assert p99 < 100us (2x budget, allows CI noise)

**Constraints:**
- Use `std::time::Instant` (not rdtsc — portable, good
  enough for >1us resolution)
- Single test binary, no external deps besides localhost UDP
- Skip with `#[ignore]` by default (needs processes running)
- `make e2e-latency` target runs it

**Acceptance:**
- [ ] Test passes with 3 processes running on localhost
- [ ] p50 printed to stdout, p99 asserted < 100us
- [ ] `make e2e-latency` works from repo root

## Deliverable 2: Criterion CI gate

**What:** A script that runs `cargo bench`, parses Criterion
JSON output, and fails if any benchmark regresses >10%.

**Where:** `scripts/bench-gate.sh`

**How:**
- `cargo bench --workspace -- --output-format=bencher`
  (machine-readable output)
- Parse each benchmark's time estimate
- Compare against `tmp/bench-baseline.json` (committed or
  generated on first run)
- Exit 1 if any benchmark > 1.10x baseline
- `--save-baseline` flag to update the baseline file

**Constraints:**
- Pure bash + jq (no Python, no npm)
- Baseline stored in `tmp/bench-baseline.json` (gitignored)
- First run with no baseline = save + pass
- Print table: benchmark name, baseline ns, current ns, ratio

**Makefile:**
```
bench-gate:
    bash scripts/bench-gate.sh
bench-save:
    bash scripts/bench-gate.sh --save-baseline
```

**Acceptance:**
- [ ] `make bench-gate` fails if any bench >10% slower
- [ ] `make bench-save` saves new baseline
- [ ] Works with all 10 existing bench crates

## Deliverable 3: rsx-sim load generator

**What:** Fill in the rsx-sim stub. A Rust binary that
connects N WS clients to the gateway and sustains K
orders/sec, reporting latency percentiles.

**Where:** `rsx-sim/src/main.rs` (replace the TODO stub)

**How:**
- CLI: `rsx-sim --users 100 --rate 10000 --duration 60`
- Each user: tokio task, WS connection to gateway
- Order profile: 50% buy/50% sell, random prices near mid,
  IOC to avoid book buildup
- Measure per-order latency (send -> fill/reject received)
- Every 5s print: throughput (orders/sec), p50, p99, p99.9
- At end: summary + exit 0 if p99 < target, exit 1 otherwise

**Constraints:**
- tokio (not monoio) — sim is not latency-critical, tokio
  has better WS client ecosystem (tokio-tungstenite)
- `RSX_SIM_GW_URL` env var (default ws://localhost:8080)
- `RSX_SIM_TARGET_P99_US` env var (default 200)
- No Criterion — just a load driver with built-in reporting
- Reuse existing `rsx-types` for Price/Qty/Side

**Makefile:**
```
load-test:
    cargo run -p rsx-sim -- --users 10 --rate 1000 --duration 30
```

**Acceptance:**
- [ ] `cargo run -p rsx-sim -- --users 1 --rate 10 --duration 5`
      connects, sends orders, prints latency stats
- [ ] 10K orders/sec sustained for 30s with 10 users,
      no connection drops, no 500s
- [ ] p50/p99/p99.9 printed at end

## Deliverable 4: Playground gateway-mode indicator

**What:** The playground already forwards orders to the real
gateway when it's running. Make this visible and testable.

**Where:**
- `rsx-playground/server.py` — add `/api/gateway-mode`
- `rsx-playground/tests/play_guarantees.spec.ts` — add test

**How:**
- `/api/gateway-mode` returns JSON:
  `{"mode": "live"|"sim", "url": "ws://...", "latency_us": N}`
- Mode = "live" if last gateway WS connect succeeded,
  "sim" if last connect failed
- Track last gateway connect result in a module-level var
- Add a Playwright test: when processes are running,
  gateway-mode should be "live"

**Constraints:**
- No behavior change — just observability
- The mode indicator appears on the overview page
  (small badge: "GW: live" green or "GW: sim" amber)

**Acceptance:**
- [ ] `/api/gateway-mode` returns correct mode
- [ ] Overview page shows gateway mode badge
- [ ] Playwright test verifies mode = "live" when GW is up

## Deliverable 5: Playground latency pipeline

**What:** The server already measures per-order latency in
microseconds (`order_latencies` list, `perf_counter_ns`).
But there's no JSON API to read it, the latency tests skip
or assert nothing, and the UI cards are never verified.
Wire it end-to-end: API → UI → Playwright assertions.

**Where:**
- `rsx-playground/server.py` — add `/api/latency`
- `rsx-playground/tests/play_latency.spec.ts` — fix tests
- `rsx-playground/tests/play_guarantees.spec.ts` — add test

**Server: `/api/latency` endpoint**

```python
@app.get("/api/latency")
async def api_latency():
    if not order_latencies:
        return JSONResponse({"count": 0})
    s = sorted(order_latencies)
    n = len(s)
    return JSONResponse({
        "count": n,
        "p50": s[n // 2],
        "p95": s[int(n * 0.95)],
        "p99": s[int(n * 0.99)],
        "min": s[0],
        "max": s[-1],
    })
```

**Fix `play_latency.spec.ts`:**

Current problems:
- "order latency endpoint" skips on 404 (endpoint missing)
- "latency regression chart area exists" asserts `>= 0`
  (always passes, tests nothing)
- "risk latency card visible" just checks page has the
  word "latency" (too loose)

Fixes:
- "order latency endpoint" — calls `/api/latency` (now
  exists), asserts p50 > 0 and p99 < 10000 (10ms, generous
  for WS round-trip)
- Remove the chart area test (asserts nothing)
- "risk latency card shows real values" — submit 10 orders,
  then verify `/x/risk-latency` HTML contains a number in
  microseconds
- "latency regression card shows baseline comparison" —
  verify `/x/latency-regression` HTML contains p50/p99
  values (not just the word "baseline")

**Add to `play_guarantees.spec.ts`:**

```typescript
test("latency stats populated after orders", async ({
  request,
}) => {
  // orders already submitted by prior tests
  const res = await request.get("/api/latency");
  expect(res.ok()).toBeTruthy();
  const data = await res.json();
  expect(data.count).toBeGreaterThan(0);
  expect(data.p50).toBeGreaterThan(0);
  expect(data.p99).toBeLessThan(10_000_000); // <10s
});
```

**Constraints:**
- `/api/latency` returns percentiles in microseconds (same
  unit as `order_latencies` stores)
- No new deps — just sort + index
- `order_latencies` already caps at 1000 entries, so
  percentile calc is cheap
- Fix tests in-place, don't add a new test file

**Acceptance:**
- [ ] `/api/latency` returns JSON with p50/p95/p99/min/max
- [ ] `play_latency.spec.ts` — no more skipped tests, no
      vacuous assertions, all pass
- [ ] `play_guarantees.spec.ts` — latency stats test passes
- [ ] `/x/risk-latency` and `/x/latency-regression` HTML
      contain actual numeric values when orders have been
      submitted

## Verification (all deliverables)

1. `make e2e-latency` — passes, prints p50/p99
2. `make bench-gate` — passes against saved baseline
3. `make load-test` — 1K orders/sec for 30s, no drops
4. `npx playwright test play_guarantees --project=guarantees`
   — all tests pass including gateway-mode + latency checks
5. `npx playwright test play_latency --project=latency`
   — no skipped tests, no vacuous assertions, all pass
6. `curl localhost:49171/api/latency` returns real p50/p99
   after orders submitted
7. No existing tests broken
