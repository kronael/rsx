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
Actual counts: 785 Rust, 1034 Python, ~440 Playwright
(exact count post-walkthrough additions). Update:
- `PROGRESS.md`, `TESTING.md`, `FEATURES.md`
- `BLOG.md`, `README.md`
- `specs/1/32-status.md`
- Walkthrough hero (`rsx-playground/pages.py`)

### 3. Fix canonical Playwright count for release-gate
`Makefile` release-gate hardcodes 223 but actual is much
higher. Update canonical to current total.

Files: `Makefile`, `scripts/acceptance-bundle.py`,
`scripts/gen-release-truth.py`, `scripts/ci-guard.py`

### 4. Fix maker mid_override bug
The maker reads `tmp/maker-config.json` correctly but
quotes don't shift. Root cause unknown — debug, fix, and
verify `bbo shift within 6s` Playwright test passes.

Files: `rsx-playground/market_maker.py`

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

### 7. Full `make gate` run + clean-boot verification
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

## Out of scope (new ship projects likely)

- `07-SPEC-CLEANUP`: systematic audit of bloat/stale/dup
  content in `specs/1/` — research each finding against
  shipped code, trim or capture
- `08-DEPLOY`: public deployment (domain, Docker, TLS)
- `09-SIGNUP`: auth flow and onboarding
