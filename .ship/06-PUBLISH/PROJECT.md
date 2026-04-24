# PROJECT.md — Publish-Readiness

## Goal

Make RSX actually work end-to-end from a clean boot so it
can be safely published as a public GitHub repo + live
demo. "Works and runs" before we do deployment / signup /
marketing work.

## Non-goals (deferred)

- Deployment infrastructure (Docker, cloud, domain, TLS)
- Auth/signup UI (JWT scaffolding exists but no flow)
- Marketing landing page
- Onboarding tour

## IO Surfaces

- Playground FastAPI on :49171 (walkthrough, REST, WS proxy)
- rsx-webui SPA at `/trade/` (built to dist/)
- 8 Rust processes (gateway, risk, ME, marketdata, mark,
  recorder, postgres, + maker python)
- Playwright test harness (cd rsx-playground/tests)

## Tasks

### 1. .gitignore .env files
Add `**/.env` and `.env` to `.gitignore`. Verify
`rsx-playground/.env` with Postgres creds is not in a
committed state.

Files: `.gitignore`, `rsx-playground/.env`

### 2. Fix stale test counts across all docs
Actual counts: ~1035 Rust, 1034 Python, ~440 Playwright
(exact count post-walkthrough additions). Update:
- `PROGRESS.md`, `TESTING.md`, `FEATURES.md`
- `BLOG.md`, `README.md`
- Walkthrough hero (`rsx-playground/pages.py`)
- `specs/2/44-testing.md` (check-pass flagged 877 vs 1035 drift)

### 3. Fix canonical Playwright count for release-gate
`Makefile` release-gate hardcodes 223 but actual is much
higher. Update canonical to current total.

Files: `Makefile`, `scripts/acceptance-bundle.py`,
`scripts/gen-release-truth.py`, `scripts/ci-guard.py`

### 4. Fix frozen-margin leak (was: maker mid_override)

**Root cause found** — order flow is fundamentally healthy:
gateway → risk → ME → WAL → marketdata all work correctly
from a clean DB state. The observed "orders don't reach ME"
symptom was caused by a **frozen-margin leak** in `accounts`:
across runs, `frozen_margin` accumulated beyond collateral,
so every pre-trade margin check rejected with
`InsufficientMargin`. Zero orders reached ME, WAL stayed
empty, the `/api/book` fallback showed `_maker_book`
synthetic state.

After `UPDATE accounts SET frozen_margin = 0` + cold start,
orders flow end-to-end: WAL grew to 500KB+ in seconds,
ORDER_ACCEPTED/INSERTED/CANCELLED events stream correctly,
maker mid_override is reflected in actual ME quotes
(50000 mid → 49900/50100 in WAL; changed to 51000 → shifts
to 50898/51102).

**Still open**:
- **Frozen margin release bug** — release_frozen_for_order()
  only fires on ORDER_DONE / ORDER_CANCELLED events. If an
  order is accepted by risk but then dropped somewhere
  before completing a full lifecycle, frozen margin leaks.
  Need to audit: every accepted order must eventually
  release frozen margin (on fill, cancel, or reject after
  accept). Add a reconciliation check at risk that warns
  if `sum(frozen_per_order) != acct.frozen_margin`.
- Consider adding `RSX_RISK_RESET_FROZEN_ON_START=true`
  dev-only env flag to zero frozen_margin on cold start.

Files: `rsx-risk/src/shard.rs` (`release_frozen_for_order`,
main loop), `rsx-risk/src/replay.rs`

### 4b. `/api/book` shows stale data
Playground `_book_snap` (from MD WS subscription) isn't
updating after book changes. `/api/book` falls through to
`_maker_book` fallback. This is a DISPLAY bug, not a core
bug — WAL / ME have correct state.

Files: `rsx-playground/server.py` (marketdata WS
subscriber), possibly `rsx-marketdata/` shadow book
broadcast.

### 5. Fix WS F/U frames test
`play_maker.spec.ts:163` — user 1 cross-fill expects F + U
frames within 5s, times out. Investigate gateway WS proxy
forwarding direction and private channel routing.

Files: `rsx-playground/server.py` (WS proxy),
`rsx-gateway/` (private WS handler)

### 6. Marketdata WS pipeline (optional / may defer)
Public `/ws/public` subscribers currently get no BBO frames
from live marketdata. Papered over with `_maker_book`
fallback for depth widgets. Fix root cause OR explicitly
document the limitation.

Files: `rsx-marketdata/`, `rsx-playground/server.py`

### 7. Wire RSX_LIQUIDATION_MAX_SLIP_BPS into LiquidationConfig
Check-pass (`findings-bucket-2.md`) found 13-liquidator
spec advertises `RSX_LIQUIDATION_MAX_SLIP_BPS=500` as
slippage cap, but `LiquidationConfig` has no `max_slip_bps`
field — cap is unenforced. Wire the env var, add field to
config, enforce cap in price clamping logic.

Files: `rsx-risk/src/liquidation.rs`, config loader

### 8. Remove test.skip() from play_latency.spec.ts
Lines 245, 298, 335 still have `test.skip()` — vacuous
assertions. Either implement the check or delete the
skipped test. Found by check-pass on 22-perf-verification.

Files: `rsx-playground/tests/play_latency.spec.ts`

### 9. Reconcile liquidator main-loop ordering
13-liquidator spec §7 says liquidation runs between funding
+ lease renewal; code runs it at shard tick step 1b. Either
update spec to match code (preferred if code is correct) or
move code to match spec (if spec's ordering was intentional
for a correctness reason). Investigate + decide.

Files: `specs/2/13-liquidator.md` or `rsx-risk/src/shard.rs`

### 10. Full `make gate` run + clean-boot verification
- `make gate-1-startup` through `gate-4-playwright` pass
- `make release-gate` exits 0
- Clean boot: kill all, start playground, open /walkthrough,
  click "Start All", depth renders in <30s
- Document the 60-second demo path in a new
  `docs/DEMO.md`

## Acceptance

- `grep -rn "398" docs/ *.md` returns no stale test counts
- `make gate` exits 0 end-to-end
- `cat .gitignore | grep -E '^\*?\*/\.env'` matches
- Running `bunx playwright test` in rsx-playground/tests
  completes with 0 failures (skipped acceptable)
- Clean-boot demo: from `pkill -f rsx-`, a new user can
  follow a documented path and see live depth in <60s

## Out of scope (tracked as separate ship projects)

- `07-SPEC-CLEANUP`: in progress, see its PROJECT.md
- `08-REST-ENDPOINTS`: FULL gateway REST impl (5 endpoints,
  JWT, rate limits, CORS, tests)
- `09-DASHBOARDS`: ship all 5 dashboards
- `10-DEPLOY`: public deployment (domain, Docker, TLS)
- `11-SIGNUP`: auth flow + onboarding
