# RSX Documentation Index

**224 total markdown files** (as of 2026-02-13)

## Quick Navigation

- [Project Entry Points](#project-entry-points)
- [Specifications](#specifications)
- [Crate Documentation](#crate-documentation)
- [Blog Content](#blog-content)
- [Operational Documentation](#operational-documentation)
- [Technical Notes](#technical-notes)
- [Reference Materials](#reference-materials)
- [Generated/Archived](#generatedarchived)

---

## Project Entry Points

Primary documentation for new contributors or Claude Code sessions.

| File | Purpose | Status |
|------|---------|--------|
| `README.md` | Project overview, build instructions | Active |
| `CLAUDE.md` | Claude Code AI assistant instructions | Active |
| `ARCHITECTURE.md` | System architecture overview | Active |
| `PROGRESS.md` | Implementation status tracking | Active |
| `TODO.md` | Remaining work items | Active |

---

## Specifications

Located in `specs/v1/` (v1 is current, v2 is future planning).

### Core System Specs

| File | Component | Status |
|------|-----------|--------|
| `ARCHITECTURE.md` | Overall system design | Active |
| `NETWORK.md` | Networking stack, protocols | Active |
| `TILES.md` | Tile-based thread architecture | Active |
| `CMP.md` | C Message Protocol (UDP transport) | Active |
| `DXS.md` | Data Exchange System (WAL + replay) | Active |
| `WAL.md` | Write-Ahead Log format/operations | Active |

### Component Specs

| File | Component | Dependencies |
|------|-----------|--------------|
| `ORDERBOOK.md` | Shared orderbook structure | ARCHITECTURE.md |
| `MATCHING.md` | Matching engine logic | ORDERBOOK.md, CMP.md |
| `RISK.md` | Risk engine, margin, positions | MATCHING.md, DXS.md |
| `GATEWAY.md` | WebSocket gateway | NETWORK.md, CMP.md |
| `MARKETDATA.md` | Market data broadcast | MATCHING.md, WEBPROTO.md |
| `MARK.md` | Mark price aggregation | MARKETDATA.md |
| `LIQUIDATOR.md` | Liquidation engine | RISK.md, MATCHING.md |

### Testing Specs

All `TESTING-*.md` files reference their main component specs.

| File | Component Tested |
|------|------------------|
| `TESTING.md` | Overall testing strategy |
| `TESTING-BOOK.md` | Orderbook tests |
| `TESTING-MATCHING.md` | Matching engine tests |
| `TESTING-DXS.md` | WAL/DXS tests |
| `TESTING-RISK.md` | Risk engine tests |
| `TESTING-GATEWAY.md` | Gateway tests |
| `TESTING-MARKETDATA.md` | Market data tests |
| `TESTING-MARK.md` | Mark price tests |
| `TESTING-LIQUIDATOR.md` | Liquidator tests |
| `TESTING-CMP.md` | CMP protocol tests |
| `TESTING-SMRB.md` | SPSC ring tests |

### Protocol Specs

| File | Purpose |
|------|---------|
| `WEBPROTO.md` | WebSocket message format |
| `RPC.md` | RPC protocol |
| `REST.md` | REST API |
| `MESSAGES.md` | Internal message types |

### Edge Cases & Validation

| File | Purpose |
|------|---------|
| `VALIDATION-EDGE-CASES.md` | Input validation edge cases |
| `POSITION-EDGE-CASES.md` | Position calculation edge cases |
| `CONSISTENCY.md` | Correctness invariants |

### Operational Specs

| File | Purpose |
|------|---------|
| `DEPLOY.md` | Deployment process |
| `TELEMETRY.md` | Metrics and logging |
| `DATABASE.md` | Database schema |
| `METADATA.md` | Symbol/user metadata |
| `ARCHIVE.md` | Data archival strategy |
| `PROCESS.md` | Process management |

### Dashboard Specs

| File | Purpose |
|------|---------|
| `DASHBOARD.md` | General dashboard design |
| `PLAYGROUND-DASHBOARD.md` | Trading playground UI |
| `HEALTH-DASHBOARD.md` | System health monitoring |
| `RISK-DASHBOARD.md` | Risk monitoring |
| `MANAGEMENT-DASHBOARD.md` | Operations management |

### Future Specs (v2)

Located in `specs/v2/`, not yet implemented.

| File | Purpose |
|------|---------|
| `FUTURE.md` | v2 planning roadmap |
| `IMPLEMENTATION.md` | v2 implementation notes |
| `ORDERBOOKv2.md` | Next-gen orderbook design |

---

## Crate Documentation

Each crate has `README.md` (user-facing) and `ARCHITECTURE.md` (internals).

| Crate | Purpose | Docs |
|-------|---------|------|
| `rsx-types` | Shared types, newtypes | README, ARCHITECTURE |
| `rsx-book` | Orderbook data structure | README, ARCHITECTURE |
| `rsx-matching` | Matching engine logic | README, ARCHITECTURE |
| `rsx-risk` | Risk engine, margins | README, ARCHITECTURE |
| `rsx-dxs` | WAL writer/reader | README, ARCHITECTURE |
| `rsx-gateway` | WebSocket gateway | README, ARCHITECTURE |
| `rsx-marketdata` | Market data broadcast | README, ARCHITECTURE |
| `rsx-mark` | Mark price aggregator | README, ARCHITECTURE |
| `rsx-recorder` | Archival DXS consumer | README, ARCHITECTURE |
| `rsx-cli` | WAL inspection tool | README, ARCHITECTURE |

**Note:** Most crate ARCHITECTURE.md files are small (20-50 lines).
Consider consolidating into README or removing if redundant.

---

## Blog Content

Located in `blog/`. Mix of published posts and drafts.

### Published Posts (01-18)

| File | Title | Topic |
|------|-------|-------|
| `01-design-philosophy.md` | Design Philosophy | Overall approach |
| `02-matching-engine.md` | Matching Engine | Order matching |
| `03-risk-engine.md` | Risk Engine | Margin, liquidation |
| `04-wal-and-recovery.md` | WAL and Recovery | Crash recovery |
| `05-development-journey.md` | Development Journey | Project history |
| `06-test-suite-archaeology.md` | Test Suite Archaeology | Testing evolution |
| `07-port-binding-toctou.md` | Port Binding TOCTOU | Race condition fix |
| `08-tempdir-over-tmp.md` | TempDir Over /tmp | Test isolation |
| `09-poll-dont-sleep.md` | Poll Don't Sleep | Busy-wait patterns |
| `10-build-system-limits.md` | Build System Limits | Cargo limitations |
| `11-parallel-agent-audits.md` | Parallel Agent Audits | AI-assisted dev |
| `12-deleted-serialization.md` | Deleted Serialization | Removing serde |
| `13-15mb-orderbook.md` | 15MB Orderbook | Memory layout |
| `14-testing-hostility.md` | Testing Hostility | Test design |
| `15-backpressure-or-death.md` | Backpressure or Death | Flow control |
| `16-dxs-no-broker.md` | DXS No Broker | Broker-free WAL |
| `17-asymmetric-durability.md` | Asymmetric Durability | Durability tradeoffs |
| `18-100ns-matching.md` | 100ns Matching | Performance |

### Drafts / Unpublished

| File | Topic |
|------|-------|
| `cmp.md` | CMP protocol deep dive |
| `picking-a-wire-format.md` | Wire format selection |
| `dont-yolo-structs-over-the-wire.md` | Serialization safety |
| `flatbuffers-isnt-free.md` | FlatBuffers overhead |
| `your-wal-is-lying-to-you.md` | WAL correctness issues |

### Session Logs (NOT blog posts)

These are AI session transcripts, should be moved to archive:

- `BLOG-UPDATE-2026-02-13.md`
- `BLOG-UPDATE-2026-02-13-v2.md`

---

## Operational Documentation

### Runbooks & Monitoring

| File | Purpose | Status |
|------|---------|--------|
| `MONITORING.md` | Monitoring strategy | Active |
| `RECOVERY-RUNBOOK.md` | Crash recovery procedures | Active |
| `CRASH.md` | Crash scenario analysis | Active |
| `CRASH-SCENARIOS.md` | Specific crash cases | Active |
| `GUARANTEES.md` | System guarantees | Active |

### Shipping & Project Management

| File | Purpose | Status |
|------|---------|--------|
| `SHIP.md` | Shipping checklist | Active |
| `SHIP-STATUS.md` | Current shipping status | Session log |
| `TASKS.md` | Task tracking | Active |
| `PROJECT.md` | Project management | Active |

### Testing & Validation

| File | Purpose | Status |
|------|---------|--------|
| `TEST-VALIDATION-REPORT.md` | Test suite validation | Active |
| `PLAYWRIGHT_TESTS_SUMMARY.md` | Playwright E2E summary | Active |

### Design & Analysis

| File | Purpose | Status |
|------|---------|--------|
| `DEFICIENCIES.md` | Known issues/limitations | Active |
| `LEFTOSPEC.md` | Spec items not implemented | Active |
| `KNOWLEDGE.md` | Domain knowledge | Active |
| `SPEEDUP.md` | Performance optimization notes | Active |
| `REJECTION.md` | Rejected design decisions | Active |
| `REPLICATION-IMPL.md` | Replication implementation | Active |

### Session Logs (Archive)

Historical AI-assisted development session outputs:

- `CRITIQUE.md` - Original critique (all 36 items resolved)
- `CRITIQUE-FINDINGS.md` - Findings from critique
- `REFINEMENT.md` - Refinement session log
- `REFINEMENT-COMPLETE.md` - Refinement completion

---

## Technical Notes

Located in `notes/`. Implementation details and patterns.

| File | Topic |
|------|-------|
| `SMRB.md` | SPSC ring buffer implementation |
| `ALIGN.md` | Memory alignment patterns |
| `ARENA.md` | Arena allocator design |
| `HOTCOLD.md` | Hot/cold path separation |
| `UDS.md` | Unix domain sockets |
| `PQ.md` | Priority queue notes |

---

## Reference Materials

Located in `refs/`. External project documentation (150MB).

### Barter-rs (Rust Trading Framework)

78 MD files documenting a Rust trading ecosystem:

- `refs/barter-rs/README.md` - Main project
- `refs/barter-rs/barter-data/README.md` - Market data
- `refs/barter-rs/barter-execution/README.md` - Order execution
- `refs/barter-rs/barter-instrument/README.md` - Instruments
- `refs/barter-rs/barter-integration/README.md` - Integration
- `refs/barter-rs/barter/README.md` - Core library

### Firedancer (Solana Validator)

88 MD files from Jump Crypto's Solana validator:

- Core docs: README, CONTRIBUTING, SECURITY
- Book: guides, API docs, monitoring, tuning
- Internal tile architecture (similar to RSX tiles)

### RustX

- `refs/RustX/README.md` - Another Rust trading project

**Recommendation:** Move `refs/` to separate repository or external
documentation links. These are reference materials, not part of RSX
codebase. If kept, document specific learnings applied to RSX.

---

## Generated/Archived

These files should be ignored by git or moved to archive.

### Python Virtual Environment

**26 files** in `rsx-playground/.venv/lib/python3.14/site-packages/*/`:

- Package LICENSE.md files
- Should be ignored (already in .gitignore)

### Node Modules

**10 files** in `rsx-playground/tests/node_modules/playwright/`:

- Playwright agent prompts
- Should be ignored (already in .gitignore)

### Pytest Cache

- `rsx-playground/.pytest_cache/README.md`
- Should be ignored (already in .gitignore)

### Test Results

**10 error-context.md files** in test-results/:

- Playwright test failure contexts
- Should be ignored (not currently in .gitignore)

---

## Claude Code Plans

Located in `.claude/plans/`. Completed implementation plans.

| File | Topic | Status |
|------|-------|--------|
| `eighty-percent.md` | 80% completion plan | Completed |
| `gateway-heartbeat.md` | Gateway heartbeat | Completed |
| `gateway-wiring.md` | Gateway wiring | Completed |
| `marketdata-mark.md` | Market data + mark | Completed |
| `marketdata-ws-broadcast.md` | WebSocket broadcast | Completed |
| `me-fanout-marketdata.md` | ME fanout | Completed |
| `post-only.md` | Post-only orders | Completed |
| `risk-liquidation-wiring.md` | Liquidation wiring | Completed |
| `risk-mark-consumer.md` | Mark consumer | Completed |

**Status:** Already ignored by .gitignore (.claude/ excluded).

---

## Duplicate/Obsolete Files

### Playground Detailed Specs

Located in `specs/playground/` - detailed implementation specs for
the playground dashboard (developer control plane).

- `BLOG.md` - Playground concept/philosophy (blog draft)
- `IDEAS.md` - Feature brainstorm
- `SCREENS.md` - Dashboard screen layouts (detailed)
- `SPEC.md` - REST API specification (detailed)

**Relationship:** These provide implementation details for the
conceptual overview in `specs/v1/PLAYGROUND-DASHBOARD.md`.

**Recommendation:** Keep but reorganize:
- Move BLOG.md to blog/DRAFT-playground.md
- Move others to specs/v1/PLAYGROUND-*.md
- Remove empty specs/playground/ directory

### UI/UX Specs

- `FRONTEND.md` - Frontend design (root level)
- `SCREENS.md` - Screen designs (root level)
- `index.md` - Appears to be index page

**Recommendation:** Consolidate into specs/v1/DASHBOARD.md or
separate frontend/ directory.

### Project Meta

- `AGENTS.md` - Agent usage patterns
- `CODEPATHS.md` - Code path analysis

**Recommendation:** Keep if actively maintained, archive if historical.

---

## Maintenance Recommendations

### 1. Immediate .gitignore Updates

Add to `.gitignore`:

```
# Test artifacts
rsx-playground/tests/test-results/
```

### 2. Archive Session Logs

Move to `archive/sessions/`:

- `CRITIQUE.md`
- `CRITIQUE-FINDINGS.md`
- `REFINEMENT.md`
- `REFINEMENT-COMPLETE.md`
- `SHIP-STATUS.md`
- `blog/BLOG-UPDATE-*.md`

### 3. Reference Materials

Option A: Delete `refs/` entirely (150MB, external projects)
Option B: Move to separate repository
Option C: Keep but add refs/README.md explaining relevance

### 4. Consolidate Duplicate Specs

- Compare `specs/playground/*` with `specs/v1/*`
- Delete if duplicates, merge if unique content
- Compare `FRONTEND.md`, `SCREENS.md`, `index.md`

### 5. Consolidate Crate ARCHITECTURE.md

Many crate ARCHITECTURE.md files are <50 lines. Options:

- Merge into crate README.md
- Delete if redundant with code comments
- Keep only for complex crates (rsx-dxs, rsx-matching)

### 6. Blog Cleanup

- Move session logs out of blog/
- Add blog/README.md with post ordering
- Mark drafts clearly (prefix with `DRAFT-`)

### 7. Create Maintenance Ownership

Add `DOCS-MANIFEST.md` (see below) tracking:

- Document purpose
- Last updated date
- Maintenance owner
- Dependencies

---

## Documentation Dependencies

Key dependency chains:

```
ARCHITECTURE.md
├── NETWORK.md
│   ├── TILES.md
│   │   └── SMRB.md (notes/)
│   ├── CMP.md
│   └── GATEWAY.md
├── DXS.md
│   ├── WAL.md
│   └── CMP.md
├── ORDERBOOK.md
│   └── MATCHING.md
│       ├── RISK.md
│       │   └── LIQUIDATOR.md
│       └── MARKETDATA.md
│           └── MARK.md
└── CONSISTENCY.md
```

Testing specs mirror component specs:

```
TESTING.md
├── TESTING-BOOK.md → ORDERBOOK.md
├── TESTING-MATCHING.md → MATCHING.md
├── TESTING-DXS.md → DXS.md
├── TESTING-RISK.md → RISK.md
├── TESTING-GATEWAY.md → GATEWAY.md
├── TESTING-MARKETDATA.md → MARKETDATA.md
├── TESTING-MARK.md → MARK.md
├── TESTING-LIQUIDATOR.md → LIQUIDATOR.md
├── TESTING-CMP.md → CMP.md
└── TESTING-SMRB.md → SMRB.md (notes/)
```

---

## Recommended Reading Order

### For New Developers

1. `README.md` - Project overview
2. `ARCHITECTURE.md` - System design
3. `specs/v1/TILES.md` - Thread architecture
4. `specs/v1/NETWORK.md` - Networking
5. `specs/v1/ORDERBOOK.md` - Core data structure
6. `CLAUDE.md` - Development conventions
7. `PROGRESS.md` - Current status

### For Component Work

1. Read component spec: `specs/v1/<COMPONENT>.md`
2. Read testing spec: `specs/v1/TESTING-<COMPONENT>.md`
3. Read crate README: `rsx-<component>/README.md`
4. Check PROGRESS.md for status

### For Operations

1. `MONITORING.md` - Monitoring strategy
2. `RECOVERY-RUNBOOK.md` - Recovery procedures
3. `specs/v1/DEPLOY.md` - Deployment
4. `specs/v1/TELEMETRY.md` - Metrics

---

## Summary Statistics

- **Total:** 224 markdown files
- **Active docs:** ~70 files
- **Specs (v1):** 39 files
- **Blog posts:** 18 published + 5 drafts
- **Crate docs:** 20 files (10 crates × 2 docs each)
- **References:** 78 files (should be external)
- **Generated:** 26 files (should be ignored)
- **Session logs:** ~10 files (should be archived)
- **Technical notes:** 6 files
- **Operational:** ~15 files

**Cleanup potential:** Remove 114 files (refs + generated + archived),
reducing to ~110 active documentation files.
