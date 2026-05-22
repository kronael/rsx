# 17-REFINE-2 — Closeout

The 9-item plan in `PLAN.md` shipped across 4 parallel
buckets. All target work landed; convergence checks below.

## Commits (9 total since plan)

| # | Hash | Bucket | Subject |
|---|------|--------|---------|
| 1 | `605b317` | 3 | F3.1: i64 formatter for risk/topology/maker |
| 2 | `f4ff065` | 2 | F2.1/F2.2/F2.3: emit jti + reject missing jti |
| 3 | `59965c5` | 4 | F4.1: native Rust E2E latency probe (bench-probe) |
| 4 | `f93ade1` | 4 | F4.2: gateway-only RTT probe |
| 5 | `4aa554e` | 3 | F3.2/F3.3: TTL cache + verify-fail → red health |
| 6 | `aec96ec` | 3 | F3.4: per-pipe CMP counters from per-process WAL |
| 7 | `daccaba` | 4 | F4.3: per-stage latency tracing on GW→ME→GW path |
| 8 | `5032085` | (merge race, 1↔4) | F1.3 first attempt — book MAX_EVENTS |
| 9 | `9159639` | 1 | F1.3 re-applied cleanly — book MAX_EVENTS |
| 10 | `cbfdb8d` | 1 | F1.1 — risk stalls on full fill/order rings |
| 11 | `ee30c37` | 1 | F1.2 — workspace `let _ =` sweep (19 → 0) |

The race at `5032085` happened because Bucket 4 had to add a
small cargo invocation that ended up holding files Bucket 1
intended to ship under F1.3. Bucket 1 re-applied F1.3 as
`9159639`. Both commits are in the tree (`5032085` carries
Bucket 4's bench-gate scaffolding, `9159639` carries the
actual MAX_EVENTS code change). Per repo rules (no
destructive rewrite, never amend) we kept both.

## Coverage vs plan

### Bucket 1 — Rust correctness

- ✅ F1.1 silent fill drop on risk: spin-stall loop drains
  ring between attempts; counter + power-of-two WARN.
  Regression test `fill_flood_position_matches_sum_of_fills`.
- ✅ F1.2 `let _ =` sweep: 19 → 0 hits across rsx-risk,
  rsx-matching, rsx-marketdata, rsx-gateway, rsx-cli.
  MEMORY.md updated to retract the stale "0 violations"
  claim and document the real audit history.
- ✅ F1.3 ME event drop: MAX_EVENTS 10_000 → 65_536,
  `event_buf` heap-boxed, `emit` asserts on overflow.

### Bucket 2 — Auth + JTI

- ✅ F2.1 rsx-auth emits `jti` (uuid4 hex per token).
- ✅ F2.2 gateway rejects missing jti at handshake
  boundary with 401 "missing jti".
- ✅ F2.3 replay integration test +
  `test_ws_handshake_rejects_missing_jti` for the
  null-defeat case the CTO flagged in R3.

### Bucket 3 — Playground UX truthfulness

- ✅ F3.1 i64 tick-size formatter on /risk, /topology,
  /maker; /wal stays raw with column header "(raw)".
- ✅ F3.2 TTL cache on `/x/health`, `/x/key-metrics`,
  `/x/pulse`, `/x/cmp-flows` (1 s) + `_tail_lines`
  bounded log read.
- ✅ F3.3 failing `verify_results` entry forces health
  score ≤ 49 (RED).
- ✅ F3.4 `/x/cmp-flows` reads three independent WAL
  streams per pipe — structurally cannot collapse to
  identical numbers.

### Bucket 4 — Probes + CI gate

- ✅ F4.1 `rsx-cli bench-probe` (native Rust client) —
  same WS handshake/N order shape as Python probe; for
  side-by-side numbers.
- ✅ F4.2 `/api/latency-probe-gw` — gateway-only RTT
  (rejects fast-path; no risk/ME involvement). Isolates
  Python overhead.
- ✅ F4.3 per-stage `tracing::info!(target = "latency",
  stage, oid, t_us)` at 6 sites: gateway_in / risk_in /
  me_in / me_out / risk_out / gateway_out. Dashboard
  endpoint `/x/latency-stages` parses + medians.
- ✅ F4.4 `make bench-gate-e2e` + sealed
  `bench-reference.json`; fails CI on > 10 % p50
  regression from the sealed baseline.

## Convergence checks

- `make lint`: **clippy clean** (-D warnings)
- `cargo test --workspace --lib --tests`: **885 passed /
  0 failed / 46 ignored** (was 883 / 0 / 46 before refine;
  +2 for the new regression tests).
- Tree clean.

## Deployment gap (honest)

The shared dev cluster on this host is held by another
session at uptime 18 m+ — port 49171 is owned by that
session's playground. `stop-all` correctly REFUSED to clobber
(F20 SIGTERM-first design holding the line). Result:

- The live cluster is still running **pre-refine binaries**.
- A fresh `make latency-publish` capture would not exercise
  the F1.1 stall, F1.3 expanded event buffer, or F2.2
  jti rejection. The bench-reference.json sealed at
  p50=11780µs reflects the OLD code path.
- The CEO re-audit (re-run agent-browser against the new UI)
  is similarly blocked — the cached i64 formatter, red-on-
  fail health, and three-distinct-pipes CMP counters live
  in code, not in the running process.

To validate the deployment-tier behavior, the next action
is to either:
- Coordinate with the other claude session to do a
  cluster restart, OR
- Start a parallel cluster on a different port set
  (RSX_GW_WS_ADDR / RSX_PG_HOST overrides) for an
  isolated capture.

Neither is blocking the v0.3 RC tag — the code is shipped
and tested. The runtime validation is a separate gate.

## Next moves

1. **Restart the dev cluster** in coordination → re-run
   `make latency-publish` for a post-fix baseline → snapshot
   numbers in `bench-baseline.json`.
2. **Run CEO re-audit** via agent-browser → one-page diff
   vs `CEO-REPORT.md`. Acceptance: the three repro curls
   the CEO flagged now return the formatted / cached /
   red-on-fail / distinct values.
3. **Tag v0.3.0-rc1** once 1+2 are green.

## What's still open (post-v0.3 RC)

- Trade UI ("Loading..." forever) — separate sprint.
- The 234× / 4669× over-budget situation — kernel-bypass
  swap or aggressive risk PG write-behind hardening, both
  multi-quarter.
- 12 pytest connection-refused failures in
  `stress_integration_test.py` (Bucket 3 noted them; they
  need gateway on port 28080 which the playground doesn't
  start).
