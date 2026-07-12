# 46 — Integrated load, latency, and recovery gates

This work strengthens RSX itself. It does not add a portfolio harness, a second
report tree, or a `make evidence` command. The hiring audit reads the same
artifacts developers use to decide whether ordinary stress, latency, recovery,
smoke, demo, and release gates are green.

The contracts live in `specs/2/22-perf-verification.md`,
`specs/2/44-testing.md`, and `specs/2/59-latency-observability.md`. Update the
relevant spec in the same change as each behavior below. Bare-metal/NIC tuning
and component sign-off remain out of scope.

## 1. Make the existing stress result truthful

Extend `rsx-playground/stress_client.py` and `rsx-playground/stress.py`; keep
reports in the existing `rsx-playground/tmp/stress/` location consumed by the
Stress page and API.

- Schedule sends open-loop instead of waiting for one response before sending
  the next. Track each run/cid until accepted, rejected, timed out, or pending
  at the deadline.
- Report offered, submitted, accepted, rejected-by-reason, completed, timed
  out, errors, achieved rate, and sample count. Use `null`, not zero, when a
  percentile has no samples; add p99.9 alongside p50/p95/p99/max.
- Fail the normal stress command when accepted/completed samples are zero,
  accounting does not close, terminal outcomes are below 95%, loss is
  unclassified, or Verify reports a failed invariant. Evaluate a latency goal
  only after those correctness checks pass.
- Add regression cases to `rsx-playground/tests/test_stress_api.py` and
  `rsx-playground/tests/stress_integration_test.py` for zero acceptance,
  partial acceptance, timeout, malformed results, and a valid run. Extend
  `rsx-playground/tests/play_stress.spec.ts` only where the existing report UI
  must render the new fields and failed status.

**Green gate:** `make api-stress` exits non-zero for a zero-acceptance fixture,
and a valid report has a closed accounting sum, non-zero completed samples,
non-null percentiles, and no failed Verify row.

## 2. Turn the latency publisher into the sustained-load path

Extend `scripts/latency-publish.sh`; do not create another load runner. It must
drive the real external WebSocket route (GW→Risk→ME→Risk→GW), use seeded users
and maker liquidity, and reuse the corrected stress accounting.

- Support a rate staircase of 1k, 5k, 10k, 25k, 50k, and 100k offered orders/s
  with configurable warm-up and measurement windows. Stop after the first
  correctness failure or achieved/offered ratio below 95%.
- Require at least 100,000 completed samples for a publishable step. Record
  p50/p95/p99/p99.9/max, accepted throughput, queue/loss counters, and the
  timestamp legs already defined by spec 59.
- Keep `bench-baseline.json` as the normal rolling characterization artifact.
  Store staircase runs beside the existing temporary benchmark outputs, not
  under a new evidence directory. A shared-host result is labelled as such.
- Add a long-run mode that repeats the highest stable rate for ten minutes
  three times. If the three p99 values vary by more than 10%, mark the result
  unstable and fail publication.

Extend `scripts/bench-gate-e2e.sh` so `make bench-gate-e2e` first enforces the
accounting/sample-validity contract, then compares latency to
`bench-reference.json`. A performance target miss and an invalid measurement
are separate fields; invalid measurement always fails.

**Green gates:** `make latency-publish` cannot update `bench-baseline.json`
from an invalid run; `make bench-gate-e2e` fails on missing samples, open
accounting, instability, or a regression beyond its configured threshold.

## 3. Extend the existing fault and recovery tests under traffic

Use the current fault APIs, recovery feed, and `/api/verify/run-json`; do not
add a recovery controller. Extend:

- `rsx-playground/tests/acceptance_test.py` for service-level assertions;
- `rsx-playground/tests/play_guarantees.spec.ts` for the existing live
  crash/recovery path;
- `rsx-playground/tests/api_verify_test.py` where Verify needs a regression
  assertion;
- `rsx-playground/server.py` only if an existing fault/recovery endpoint lacks
  a machine-readable timestamp or readiness state required by the tests.

At 50% of the stable rate found by the normal latency run, test one fault per
fresh stack: gateway restart, risk-primary loss/promotion, matching restart
from WAL, ME→marketdata gap/replay, and recorder stop/catch-up. Run Verify
before injection, after readiness returns, and after two further minutes of
healthy traffic.

Each case fails for duplicate terminal events/fills, an unaccounted accepted
order, WAL/book/position/recorder disagreement, missed readiness budget,
decreasing tips, panic/fatal/invariant log lines, or failure to resume normal
traffic. Keep diagnostics in the existing pytest/Playwright artifact outputs
and process logs.

**Green gates:** `make integration` contains the service recovery cases;
`make shards-gated` contains the browser-visible fault/recovery and Verify
assertions. Both exit non-zero on the first correctness failure.

## 4. Make smoke and demo prove one complete trade

Strengthen `scripts/smoke.sh`, `scripts/demo-trade.sh`, and their existing test
coverage rather than adding a clean-demo command.

- `make smoke` requires the declared scenario's full process set to become
  ready; a partial count such as 6/7 is failure.
- `make demo` remains the turnkey startup. `make demo-trade` waits for maker
  liquidity, submits one taker, and proves the same trade in the client
  response, WAL, risk position, and marketdata view.
- Both paths run all non-optional Verify checks, reject panic/fatal/missing
  required configuration, and always stop processes they started after a
  failed assertion.
- Add the missing assertions to existing API/Playwright suites, chiefly
  `rsx-playground/tests/play_guarantees.spec.ts`,
  `rsx-playground/tests/api_verify_test.py`, and the current smoke coverage.

**Green gate:** from reset, `make demo` followed by `make demo-trade` succeeds
three consecutive times without manual repair; `make smoke` fails on any
missing process, missing observation, or failed invariant.

## 5. Put the stronger checks into the current development lanes

Modify `Makefile`, without introducing a new top-level evidence lane:

1. Keep fast unit regressions in `make gate-3-api`.
2. Keep Docker/live service recovery in `make integration`.
3. Keep visible fault/recovery behavior in `make shards-gated` and therefore
   `make ci-full`.
4. Keep sustained performance opt-in through `make latency-publish` and
   regression enforcement through `make bench-gate-e2e`.
5. Keep deployed-system and walkthrough confidence in `make smoke`,
   `make demo`, and `make demo-trade`.

Update `README.md` and the existing benchmark/report interpretation only after
the gates produce real results. The wording must identify shared-host results,
sample counts, accepted throughput, and known misses. The hiring evaluator
must consume `bench-baseline.json`, normal stress reports, normal test reports,
and logs; it must not require portfolio-only output.

## 6. Reduce the public Make surface without losing test levels

The canonical workflow in `specs/2/44-testing.md` is `test`, `e2e`,
`integration`, `smoke`, and `perf`. Keep those meanings. Simplify aliases in
`Makefile` around them instead of inventing more top-level targets:

- Keep `gate` as the ordered Playground release check and `ci` / `ci-full` as
  automation lanes. Treat `gate-1-startup` through `gate-4-playwright` and the
  `shard-*` targets as internal building blocks: rename their recipes to
  phony private implementation targets (for example `_gate-startup` and
  `_shard-routing`) so they do not appear as public help entries. Do not weaken
  the dependency order or the three-green infra-smoke lock.
- Make `e2e` the only public entry point for the complete Rust + API + browser
  suite. Deprecate the duplicate public `play`, `play-full`, and individual
  `play-*` aliases. For one release they print the replacement command and
  delegate (`play`/`play-full` → `e2e`; individual `play-*` → the documented
  focused Playwright command); remove them after callers and docs migrate.
- Keep the microbench and cross-process distinctions but use one naming family:
  `perf`, `perf-gate`, `perf-save`, `perf-load`, `perf-e2e-gate`, and
  `perf-e2e-save`. For one release, retain `bench-gate`, `bench-save`,
  `latency-publish`, `bench-gate-e2e`, and `bench-gate-e2e-save` as
  compatibility aliases that print the canonical replacement. The underlying
  scripts and baseline files do not move.
- Keep `shards-gated` callable for focused browser debugging, but document it
  as an advanced lane rather than a normal developer command. `ci-full`
  remains its ordinary caller.

Update target references together in `CLAUDE.md`, `README.md`,
`specs/2/22-perf-verification.md`, `specs/2/44-testing.md`, and scripts/help
comments. Search the repository for every removed name before deleting an
alias; CI configuration and local documentation must use canonical names
first.

**Green gate:** `make help` shows one obvious command per test level and one
coherent `perf-*` family; every compatibility alias reaches the same recipe
and exit code as its replacement; `make gate`, `make ci`, and `make ci-full`
retain their current ordering and fail-fast behavior.

## Approval defaults

- Rate steps: 1k, 5k, 10k, 25k, 50k, 100k orders/s.
- Valid run: at least 95% terminal outcomes and achieved/offered throughput.
- Stable-rate check: ten minutes, repeated three times, p99 spread at most 10%.
- Recovery set: gateway, risk primary, matching, marketdata gap, recorder.
- A measured performance miss is retained honestly; an invalid measurement or
  correctness failure fails its normal gate.

## Close-out

Shipped into the normal RSX workflow:

- truthful, correlated stress accounting and failed-report rendering;
- validity-gated single, staircase, and repeated long-run E2E measurements;
- p99 regression gating with separate Criterion and E2E baseline files;
- ME crash/restart proof with resumed fills and post-recovery Verify checks;
- complete demo/smoke trade proof across client, WAL, risk, marketdata, Verify,
  and logs;
- a smaller public Make surface with truthful gate failure propagation.

Deferred because the current system lacks authoritative test controls, not
because they are portfolio-only: risk-primary promotion, forced marketdata
gap/replay, and recorder-behind-retention catch-up. Bare-metal measurements and
component sign-off remain outside this project as originally scoped.
