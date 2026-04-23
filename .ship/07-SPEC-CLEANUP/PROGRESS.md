# PROGRESS.md — Spec Cleanup

## Session log

### Session 1 — 2026-04-23 (in progress)
- Created PROJECT.md with 9-task plan
- Defined 4 buckets (12 specs each) over `specs/2/`
- Spawning 4 parallel Explore subagents for check pass
- Subagent outputs land in `findings-bucket-{1,2,3,4}.md`
- Consolidated summary written to `findings.md`

## Task progress

| # | Task | Status | Notes |
|---|------|--------|-------|
| 1 | Per-spec check pass (4 subagents) | **done** | Session 1; see findings.md |
| 2 | Resolve drift cases (spec-side, mechanical) | pending | (A) in findings.md — ~20 items |
| 3 | Resolve drift cases (code-side, vital) | pending | max_slip_bps, test.skip() |
| 4 | Decide scope for unshipped vital items | pending | user input on REST endpoints |
| 5 | Move unshipped + not-vital → specs/3/ | pending | 7/12/27 dashboards + 33 telemetry prod + 9 stubs + 2 archive |
| 6 | Capture unshipped + vital → ship projects | pending | depends on #4 |
| 7 | Trim bloat | pending | (D) in findings.md — mechanical |
| 8 | Consolidate duplication | pending | (E) in findings.md — higher risk |
| 9 | Delete/archive 31-sim, 43-smrb | pending | (F) in findings.md |
| 10 | Update status frontmatter | pending | — |
| 11 | Regenerate specs/index.md | pending | — |
| 12 | Reference sweep | pending | — |
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
