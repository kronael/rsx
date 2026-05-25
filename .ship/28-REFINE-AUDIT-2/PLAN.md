# .ship/28 — Refine + Audit sprint, round 2

User ask: 4 more rounds of refine on what we've created, re-run CTO +
CEO critique, fix everything, demonstrate the system with benchmarks.

Master at sprint start: `5b09b33`.

## Carry-over from .ship/27

Open items the .ship/27 SUMMARY flagged for this sprint:

1. **FAULTED replay wiring for risk + marketdata + gateway** —
   highest-ROI item. Without it, auto-maker overruns rmem → Risk
   FAULTs → panic, so the demo only trades with the maker opt-in.
2. **Playground honesty cluster F-N4–F-N16** —
   synthetic book ladder, IOC-as-resting, false-PASS stress,
   gateway max-conn cap, restarts=0 counter, walkthrough stale
   crate/test counts, /docs Tailwind CDN, pulse 5/10 disagrees
   with /api/processes.
3. **`make tune-host`** to bump rmem_max so maker can be auto-started
   again.
4. **WAL fsync + random-read bench numbers** in rsx-cast README +
   ARCHITECTURE need host-pinned re-measurement before publication.
5. CTO #6 / #7 / #9 / #10 — consumer try_recv allocations, BBO Vec,
   FAULTED recovery, BBO double-CRC.

## Sequencing (sequential, no worktrees — keep disk lean)

### Round 1 — FAULTED recovery (highest-ROI carry-over)

Wire `CastRecv::Faulted` → ReplicationConsumer drain → resume for
risk, marketdata, gateway. Pattern reference is rsx-matching's POC.
After this round, the auto-maker should be safe to re-enable
(separately, see Round 4's `make tune-host`).

### Round 2 — Playground honesty cluster

Fix F-N4 through F-N16 from `.ship/27-REFINE-AUDIT/CEO-REPORT.md`:
- F-N4 synthetic book without badge
- F-N5 probe-gw ok=true with error_code 1007
- F-N6 stress false-PASS
- F-N7 /verify mixes archive + live
- F-N8 /verify PASSes on empty WAL
- F-N9 IOC-as-resting in /x/order-trace
- F-N10 $500M IOC silent
- F-N11 gateway CMP rebind after restart
- F-N12 max-conn-per-user cap
- F-N13 /api/processes restarts=0 counter
- F-N14–F-N16 (walkthrough stale counts, /docs CDN, pulse 5/10
  disagreement)

### Round 3 — CTO carry-over (consumer hot-path hygiene)

- CTO #6: project-wide "zero heap" claim. Either qualify it ("hot
  send path zero heap; receivers allocate a `Vec<u8>` per record")
  or migrate consumers from `try_recv` to `try_recv_with` to make
  the project-wide claim hold.
- CTO #7: rsx-risk BBO scan allocates `Vec<u32>` per BBO (R2 R-N5
  not closed).
- CTO #10: BBO double-CRC (cmp + mkt destinations). Document or fix.

### Round 4 — bench infra + host tuning

- `make tune-host` Makefile target that bumps `net.core.rmem_max`
  (and `wmem_max`) via sysctl. Note in CLAUDE.md / README.
- Re-measure the WAL fsync + random-read benches on a clean host
  (single-pass, pinned to cores 2/3), publish numbers in
  `rsx-cast/ARCHITECTURE.md`.
- Add a `make demo-trade` Makefile target that runs the canonical
  end-to-end demo (start-all minimal + maker + submit one order
  + verify fill in WAL) so the demo is reproducible.

### CTO audit + CEO audit

Use the new `cto-eval` and `ceo-eval` skills. Same shape as
`.ship/27`. Compare grades: CTO 58/100 → ? ; CEO 14/100 → ?.

### Fix-up sprint

Whatever the audits surface. Critical first, then major.

### Bench rerun + RESULTS + SUMMARY

Re-run the perf-table claims, compare to `.ship/27/RESULTS.md`,
publish a `RESULTS.md` + sprint `SUMMARY.md`.

## Out of scope (deferred)

- Anything requiring a kernel rebuild or external service.
- End-to-end production cluster benches (no cluster).
- Public publishing (no — see CLAUDE.md Publishing section).

## Reports

Per round → `REPORT-N.md`. CTO → `CTO-REPORT.md`. CEO →
`CEO-REPORT.md`. Fix → `FIX-REPORT.md`. Bench → `RESULTS.md`.
Final → `SUMMARY.md`.
