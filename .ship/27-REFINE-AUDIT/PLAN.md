# .ship/27 — Refine + Audit sprint

User ask: 4 rounds of refine on this session's code, then re-run
CTO + CEO playground critique, fix everything, demonstrate.

Master at sprint start: `f373db3`.

## Sequencing (sequential, no worktrees — disk constraint)

Round subs run in series in the main repo. Each commits its
work before the next starts.

### Round 1 — rsx-cast core hygiene
Target: `rsx-cast/src/{cast,wal,replication_*,records,header,encode_utils,config}.rs`.
Look for: dead code, redundant abstractions, magic constants
without names, double-borrow gymnastics, long fns that should
split, structs with parallel arrays that should be SoA.
Cut > add.

### Round 2 — consumer wiring
Target: `rsx-matching/src/{main,replay,wal_integration}.rs`,
`rsx-risk/src/main.rs`, `rsx-gateway/src/main.rs`,
`rsx-marketdata/src/main.rs`. The Framed migration and the
publish_events fan-out are the most recent. Verify the patterns
are consistent, kill any leftover `.append`/`.send` solo paths
that should be `prepare → append_framed → send_framed`.

### Round 3 — specs + ARCHITECTURE sync
Target: `specs/2/*.md` + each crate's `ARCHITECTURE.md`. Sync
to the v0.5.0 state (cast/replication terms, byte-0 version,
Framed pattern, publish_events fan-out). Drop wedge mentions.

### Round 4 — docs hygiene + cross-cut consistency
Target: `README.md`, `BLOG.md`, `ONEPAGER.md`, `CHANGELOG.md`,
`PROGRESS.md`. Stale-fact sweep. Ensure terminology matches
across all surfaces. Ensure cited file paths / commit hashes
still resolve.

### CTO audit (after Round 4)
Strict reviewer: source-and-bench reads, verify 5 specific
claims (rebenchable), propose 3 attack scenarios + trace
through code, numeric 0-100 SLA-bet grade. Mirrors
`.ship/20-CTO-CEO-REVIEW-2/CTO-REPORT.md` shape.

### CEO audit (after Round 4, parallel with CTO)
Run the playground end-to-end: ≥3 injected faults, full demo
flow, time every endpoint, ≥5 NEW findings (not in prior
audits), numeric 0-100 grade.

### Critique fixes
Whatever the audits surface, prioritise + ship in one
follow-up pass.

### Benchmarks + measurements assembly
Re-run every Criterion bench (28 files), capture numbers,
assemble a `RESULTS.md` with a comparison table and per-bench
attribution. Update `bench-baseline.json` if numbers are stable.

## Deferred / out of scope

- Anything requiring a kernel rebuild or external service
  (no DPDK, no AF_XDP, no separate aeron driver).
- Bench numbers from production cluster (no production
  cluster available).
- Anything labelled "design budget" still doesn't become
  "measured" in this sprint (no E2E latency harness yet).

## Reports

Each round drops a short `REPORT-N.md` here. CTO audit →
`CTO-REPORT.md`. CEO audit → `CEO-REPORT.md`. Final state →
`SUMMARY.md`.
