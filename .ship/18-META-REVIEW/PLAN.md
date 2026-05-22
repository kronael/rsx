# 18-META-REVIEW — Plan

Adversarial meta-review of the work since v0.2.0 (75 commits,
all on 2026-05-21 / 2026-05-22). Reads the prior CTO+CEO
reports, the 17-REFINE-2 arc, the playground audit findings,
the actual code at HEAD, and the diff between docs and reality.

## In scope

- Pattern of skipped verifications (F22, broken probes, sealed
  bench reference, doc drift)
- Architecture drift from spec/WEDGE.md (rsx-maker phantom,
  rsx-log token spent, FillRecord wire change without
  version bump, MAX_EVENTS panic-on-overflow)
- Over-investment in the latency arc (~17 commits = 23% of
  the round) vs unshipped CEO findings
- Tooling gaps: shared dev cluster collisions, commit
  message lying, MEMORY drift, missing diary, 5 different
  test counts in 5 different files, dual-source-of-truth on
  the sealed bench reference vs the live e2e_us
- Specific commits to undo or rethink: the duplicate
  5032085/9159639 MAX_EVENTS pair, the sealed-on-broken-probe
  bench-reference.json, the rsx-log innovation token spend
- Forced-rank "next two weeks" for course correction (not
  feature delivery)
- ≥40 findings, each with file:line / commit-hash citation
- Final 250-word verdict

## Out of scope

- Code-level fixes (this is record-only, per the user prompt)
- Trade UI separate sprint
- The 22× over-budget itself (multi-quarter)
- Publishing decisions (founder-level)
