# PROGRESS.md — Spec Cleanup

## Session log

### Session 1 — 2026-04-23
- Created PROJECT.md with 9-task plan
- 4-bucket check-pass completed; findings consolidated
- Phase (d): folded 3 code-side bugs into `.ship/06-PUBLISH/`
  (max_slip_bps, test.skip, liquidator main-loop ordering)
- Phase (a): 18 spec-side drift fixes applied + committed
  (commits: 7a8b8c5, d275f14)
- Phase (c): 2-archive.md moved to specs/3/4-archive.md
  (status: draft; commit: bb5ca00)
- Phase (b): 4 parallel trim subagents running on 18 specs
- Created `.ship/08-REST-ENDPOINTS/PROJECT.md`
- Created `.ship/09-DASHBOARDS/PROJECT.md`

## Task progress

| # | Task | Status | Notes |
|---|------|--------|-------|
| 1 | Per-spec check pass (4 subagents) | **done** | Session 1; findings.md |
| 2 | Resolve drift cases (spec-side, mechanical) | **done** | Phase (a); commits 7a8b8c5, d275f14 |
| 3 | Resolve drift cases (code-side, vital) | handed off | Folded into 06-PUBLISH tasks 7-9 |
| 4 | Decide scope for unshipped vital items | **done** | User: ship all dashboards + full REST |
| 5 | Move unshipped + not-vital → specs/3/ | **done** | Phase (c); only 2-archive moved |
| 6 | Capture unshipped + vital → ship projects | **done** | 08-REST-ENDPOINTS, 09-DASHBOARDS |
| 7 | Trim bloat | **in progress** | Phase (b); 4 subagents on 18 specs |
| 8 | Consolidate duplication | pending | (E) in findings.md — after trim |
| 9 | Delete/archive 31-sim, 43-smrb | pending | (F) in findings.md |
| 10 | Update status frontmatter | pending | after trim + consolidate |
| 11 | Regenerate specs/index.md | pending | after all moves/deletes |
| 12 | Reference sweep | pending | final pass |
| 13 | Second-audit cycle | pending | acceptance gate |

## Bucket assignments (48 specs in specs/2/)

- **Bucket 1** — specs/2/1 through specs/2/12 (arch/transport/ops)
- **Bucket 2** — specs/2/13 through specs/2/24 (components)
- **Bucket 3** — specs/2/25 through specs/2/37 (gateway/risk/testing-early)
- **Bucket 4** — specs/2/38 through specs/2/49 (testing-late/UI/architecture)

## How to resume

Next session:
1. Read this PROGRESS.md
2. Read `findings.md` for consolidated check-pass results
3. Pick the highest-priority task from the `## Task progress`
   table with status `pending`
4. Execute, then update PROGRESS.md

## Artifacts

- `PROJECT.md` — plan + acceptance
- `PROGRESS.md` — this file, session log
- `findings-bucket-1.md` through `findings-bucket-4.md` —
  raw subagent output
- `findings.md` — consolidated findings table
