# 16-CTO-CEO-REVIEW — Plan

Dual adversarial review per `meta/cto-ceo-review.md`. Run
2026-05-22, after the 28-finding playground audit cleanup
landed (commits `0120806` through `82f096d`, plus
`72bd481` JtiTracker, `0d9d265` publishing rule).

## Trigger

User asked for "oracle CTO and CEO critique of the whole
project" — the natural pre-v0.3 sanity check.

## Scope

### In scope

- All of rsx-* crates (Rust workspace)
- rsx-playground (Python FastAPI dashboard)
- rsx-webui (React trade UI)
- rsx-auth (Python sqlx service)
- All specs/2/*.md
- Existing audit findings F1-F28 (closed) as context
- Live playground at http://localhost:49171

### Out of scope

- v0.3 release planning (separate sprint)
- The wedge decision itself (locked in WEDGE.md)
- External publishing — repo policy forbids it (CLAUDE.md)
- Fixing anything found this round (record only; act later)

## Lenses

### CEO (commercial DD)

- **Tool**: agent-browser CLI against http://localhost:49171
- **Visit**: all UI surfaces (Walkthrough, Overview,
  Topology, Latency, Book, Risk, WAL, Logs, Control, Maker,
  Faults, Verify, Orders, Stress, Docs, Trade)
- **Eyes off**: source code, specs/, internal docs
- **Output**: `.ship/16-CTO-CEO-REVIEW/CEO-REPORT.md`

### CTO (engineering DD)

- **Tool**: `codex exec` for adversarial pass + direct file
  reads
- **Read**: lib.rs of each crate, main.rs of each binary,
  ARCHITECTURE.md per crate + root, key specs (4-cmp,
  6-consistency, 21-orderbook, 28-risk, 45-tiles, 48-wal),
  tests/ for invariant coverage, CHANGELOG.md
- **Eyes off**: the live playground (no curl, no browser)
- **Output**: `.ship/16-CTO-CEO-REVIEW/CTO-REPORT.md`

## Execution

Two parallel general-purpose subagents — disjoint
artifacts, no overlap.

### CEO agent

Spawned with a tight, adversarial brief. Uses
`agent-browser` via Bash. Walks every tab. Records findings
in the agreed structure (verdict, strengths, risks, top-3
fixes, surprises, out-of-scope). Captures screenshots into
`.ship/16-CTO-CEO-REVIEW/screenshots/` when something is
visually load-bearing.

### CTO agent

Spawned with a tight, adversarial brief. Uses `codex exec`
on focused excerpts, plus direct reads. Cites file:line.
Output in the same structure.

## After both return

- Commit each report under `[review]` section.
- A synthesis pass cross-references both reports and tags
  items as: BOTH (top priority), CEO-ONLY (UX/docs),
  CTO-ONLY (engineering), DISAGREE (escalate).
- The synthesis becomes the input to the upcoming refine
  pass (16-REFINE-2 or similar).

## Success criteria

- Two reports exist, each ≥ 30 actionable findings, each
  with file:line or UI-path citations.
- No overlap of literal findings between reports (the
  lenses ARE different).
- Both reports include an explicit "would I fund/hire/ship"
  verdict in section 1.
- Tree clean after commit.
