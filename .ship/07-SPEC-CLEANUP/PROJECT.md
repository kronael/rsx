# PROJECT.md — Spec Cleanup

## Goal

Every spec in `specs/2/` accurately reflects shipped code.
Unshipped design is either captured as a new ship project or
moved to `specs/3/` (future/planned). Status frontmatter is
accurate. No duplicated or drifted content.

## Non-goals

- Changing the phase layout (already done in prior session)
- Touching `specs/1/` (historical ship logs — frozen)
- Rewriting content beyond trimming / reorganizing

## Method

Per spec in `specs/2/`, three-step process:

**Step A — Check against code.** For each claim in the
spec, grep the codebase. Categorize each section:
- `match` — spec matches code, keep
- `drift` — spec claims something code doesn't do
- `unshipped` — spec describes design not yet in code
- `bloat` — implementation detail that should be in code,
  not spec (struct definitions, pseudocode, test name lists)

**Step B — Act per category.**
- `match` + `bloat` → trim to WHY + code pointer
- `match` → keep
- `drift` → either fix spec to match code, OR fix code to
  match spec (case-by-case)
- `unshipped` + vital for publish → keep in `specs/2/`, new
  ship project to resolve
- `unshipped` + not vital → move section to `specs/3/`

**Step C — Update frontmatter status.**
- `shipped` — all content matches code
- `partial` — some sections drift or unshipped
- `draft` — largely unshipped, being captured for later
- `reference` — analysis/notes, not design

## Tasks

### 1. Per-spec check pass (parallel subagents)
Buckets of ~12 specs each, spawn 4 parallel Explore
agents. Each reports per-section categorization.

Files: all 48 files in `specs/2/`

### 2. Resolve drift cases (manual)
For each `drift` finding, decide: fix spec or fix code.
Capture code-fix items as tasks in `06-PUBLISH` or new ship
projects if vital.

### 3. Move unshipped + not-vital to specs/3/
Use `git mv` + renumber. Update status to `draft` or
`planned`. Add cross-ref in `specs/2/` spec pointing
forward.

### 4. Capture unshipped + vital as ship projects
Create `.ship/NN-NAME/PROJECT.md` per finding. Reference
from the `specs/2/` spec.

### 5. Trim bloat (structs, pseudocode, test lists)
Replace with WHY + 1-line code pointer. Target worst
offenders first (audit identified):
- `21-orderbook.md §§2.5-2.7` (400 lines compression)
- `29-rpc.md` lines 586-641 (micro-benchmarks)
- `4-cmp.md §6` (Rust pitfalls)
- `8-database.md` "Five Key Points"
- `28-risk.md §§2, 3, 6, 7, 8` (structs, pseudocode, files)
- `13-liquidator.md §1, §8` (structs, SQL DDL)
- `34-testing-book.md`, `41-testing-matching.md`,
  `36-testing-dxs.md`, `35-testing-cmp.md`,
  `42-testing-risk.md` (hundreds of test names)

### 6. Consolidate duplication
- Process/tile arch: ARCHITECTURE + TILES + PROCESS → one
  canonical (likely 45-tiles.md)
- Message flow: MESSAGES + RPC → split by concern
- Dashboard boilerplate: 7-dashboard, 12-health, 14-mgmt,
  23-playground, 27-risk → shared "Platform" section +
  per-module deltas
- Config propagation: one spec
- Correctness invariants: 44-testing.md canonical, others
  cross-ref

### 7. Update frontmatter status per file
Per Step C rules above. STATUS-style files already deleted
(32-status.md). Watch for similar "audit snapshots" that
aren't design specs.

### 8. Regenerate specs/index.md
After all renames/moves. Keep the script idempotent in
`/tmp/make-index2.py`.

### 9. Second-pass reference sweep
Find + fix any `specs/2/NN-oldname.md` references that
moved to `specs/3/`.

## Acceptance

- Every spec in `specs/2/` passes Step A check (every
  claim has a matching code reference or is marked
  `unshipped` with a captured ship project / specs/3 move)
- `status:` frontmatter accurate per spec
- No duplicated content (same topic in >1 spec)
- No `bloat` sections (>50 lines of struct/pseudocode
  without WHY)
- `specs/index.md` current
- `specs/2/` passes a focused re-audit (same 4-bucket
  pattern) with zero stale/drift findings

## Blocks

`06-PUBLISH` final acceptance (clean-boot public demo).
Spec cleanup must be done before publicizing the repo.
