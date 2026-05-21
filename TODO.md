# TODO

Status as of 2026-05-21 (post-v0.2.0, post-playground audit).

Active ship projects live in `.ship/NN-NAME/`. This file
is the light backlog — items not yet a ship project.

## Critical (from playground audit — `.ship/15-PLAYGROUND-AUDIT/FINDINGS.md`)

### F1: ME restart loop on `AddrInUse` + `flush stalled`
- `rsx-matching/src/main.rs:319` panics when parent's UDP
  socket is still half-open. Cascades to marketdata 45 GB
  OOM allocation and WAL truncation (Finding 4 below).
- Fix candidates: `SO_REUSEPORT` on CMP bind; pre-bind
  liveness check; orderly drain on SIGTERM.
- Spec context: 6-consistency.md invariant #7 ("WAL
  persistence") is being violated when this loop runs.

### F2: Health dashboard reports "100 GREEN" while latency p99 = 6 s
- `/x/health`, `/x/key-metrics`, `/x/pulse` all stale.
- Health computation needs to read process restart count +
  latency-vs-baseline delta.
- Severity: this is the headline dashboard; it is lying.

### F4: Verify says "no trades yet" with 135 fills observed
- `/api/verify/run` invariant 1 ("Fills precede ORDER_DONE")
  silent-skips and returns PASS.
- Either count real fills or fail honestly. Cross-references
  6-consistency.md invariant 1.

## Important (from playground audit)

- **F3** WAL UI shows 0.0 B while disk has 6 KB + Verify
  disagrees. Three views of one WAL must reconcile.
- **F5** Topology pill says "stopped" while `/api/processes`
  says running. Two oracles, one truth.
- **F7** Logs "gateway" quick-filter returns empty; log lines
  prefixed `[gw-0]`. Label-to-prefix consistency.
- **F8** Risk SYSTEM-WIDE METRICS show `--` with 135 fills
  observable elsewhere. Aggregator unwired.
- **F9** ME → Mktdata CMP counter stuck at 0 while ladder
  updates. Counter or wiring broken.
- **F10** Landing-page orderbook row stuck at `data-px="50000000000"`
  after an ME restart. Stale level after recovery.
- **F12** Trade UI shows "Authentication failed" red toast
  on first paint before user tried to sign in.

## Nice-to-have (from audit)

- F6 `/x/topology/mark` detail panel reduces to a stub
  "mark data requires mark process" while mark is running.
- F11 `/stress` scenarios panel renders only "no data".

## Oracle pass (codex, verified) — 7 more lies

### F13 (critical): `/x/pulse` proc pill green if `running > 0`
- `server.py:2961-2963`. 1/8 alive is painted "emerald".
- Health pill must require ALL expected processes running.

### F14 (important): Gateway "circuit breaker: closed" hardcoded
- `server.py:2127` — literal `("circuit breaker", "closed")`.
- Read real breaker state from gateway or remove the row.

### F15 (important): Topology flow rates from Python process memory
- `server.py:2370-2371`. `len(recent_orders)` / `len(recent_fills)`
  / `len(_book_snap)` are dashboard-process state, not cluster
  state. Reset on dashboard restart; can be fake.
- Read from /api/processes counters or WAL.

### F16 (important): Index price = mark × 1.0001 (synthetic)
- `server.py:5697`. `/api/risk/funding` returns fabricated
  index, premium, and rate.
- Either query mark process's real index input or label the
  panel "demo data."

### F17 (important): Reconciliation Mark-vs-Index is "book has bid+ask"
- `server.py:3354-3367`. No index loaded anywhere.
- Either load an index or remove this check.

### F18 (important): Reconciliation Shadow-vs-ME compares two WAL views
- `server.py:3328-3352`. Both sides are downstream of ME.
- Query ME's book over CMP query channel for engine truth.

### F19 (important): `/x/stale-orders` requires numeric `ts`
- `server.py:3419-3423`. UI batch helpers write `ts` as
  `"%H:%M:%S"` strings; they are silently skipped.
- Normalize `ts` to float at order entry, or parse strings.

## Carry from v0.2.0 (CHANGELOG)

- **JtiTracker wire-through** — bounded replay set is built
  but dormant. Decision pending: per-process tracker vs
  shared Redis. TODO at `rsx-gateway/src/ws.rs:108`.
- **6 deeper clippy warnings** — too-many-args refactors
  in matching, maker, risk.
- **Measured E2E latency in `bench-baseline.json`** — first
  capture done (p50 = 11.7 ms, 234× over <50 µs budget).
  Re-capture after F1 fix + risk index fix (commit `3d151f1`)
  to see what the budget actually is.
- **BLOG.md narrative reframe** per WEDGE.md (B+A: SDK on
  open-source orthogonal parts). Editorial; depends on
  finishing F1/F2.
- **2 hot-path `eprintln!` in `rsx-book`** — no `tracing`
  dep on the crate; cross-cutting decision.

## In-flight playwright extensions (matched to audit findings)

| Finding | New / extended spec                        |
|---------|--------------------------------------------|
| F1      | `play_readiness.spec.ts::system_stays_green_for_5m` |
| F2      | new `play_health_truthful.spec.ts`         |
| F3      | `play_wal.spec.ts::wal_size_agrees_with_verify` |
| F4      | `play_guarantees.spec.ts::fills_observed_after_run` |
| F5,F6,F9| `play_topology.spec.ts` extensions         |
| F7      | `play_logs.spec.ts::filter_label_to_prefix` |
| F8      | `play_risk.spec.ts::system_metrics_populated_when_fills` |
| F10     | `play_book.spec.ts::ask_prices_in_tick_band` |
| F11     | `play_stress.spec.ts::scenarios_panel_non_empty` |
| F12     | new `play_trade.spec.ts`                   |

## Backlog (not yet scoped)

- **10-DEPLOY** — public domain, Docker, TLS, one-click
  reviewer demo
- **Replica → main promotion refactor** in `rsx-risk/src/
  main.rs` was shipped as T3.2 (commit `2c58c9e`). Verify
  no follow-up needed.

## Conventions

- Project-level items with concrete acceptance criteria
  graduate to `.ship/NN-NAME/` via `/ship` skill.
- Per-session multi-step tracking uses `TaskCreate`, not
  this file.
- Architectural design questions go to `specs/`.
