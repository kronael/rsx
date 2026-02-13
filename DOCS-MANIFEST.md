# Documentation Manifest

Tracks purpose, ownership, and maintenance status of all documentation.

Format:

```
| File | Purpose | Owner | Last Updated | Status | Dependencies |
```

- **Owner:** spec (specification team), dev (developers), ops (operations)
- **Status:** active, draft, archive, obsolete
- **Dependencies:** Other docs this file references

---

## Root Level Documentation

| File | Purpose | Owner | Last Updated | Status | Dependencies |
|------|---------|-------|--------------|--------|--------------|
| README.md | Project overview, build instructions | dev | 2026-02 | active | - |
| CLAUDE.md | AI assistant instructions | dev | 2026-02 | active | specs/v1/ |
| ARCHITECTURE.md | System architecture overview | spec | 2026-02 | active | specs/v1/ARCHITECTURE.md |
| PROGRESS.md | Implementation status tracking | dev | 2026-02-13 | active | - |
| TODO.md | Remaining work items | dev | 2026-02 | active | PROGRESS.md |
| GUARANTEES.md | System correctness guarantees | spec | 2026-02 | active | specs/v1/CONSISTENCY.md |
| MONITORING.md | Monitoring strategy | ops | 2026-02 | active | specs/v1/TELEMETRY.md |
| RECOVERY-RUNBOOK.md | Crash recovery procedures | ops | 2026-02 | active | CRASH-SCENARIOS.md |
| CRASH.md | Crash scenario analysis | spec | 2026-02 | active | specs/v1/DXS.md |
| CRASH-SCENARIOS.md | Specific crash cases | spec | 2026-02 | active | CRASH.md |
| SHIP.md | Shipping checklist | dev | 2026-02 | active | TODO.md |
| TASKS.md | Task tracking | dev | 2026-02 | active | TODO.md |
| PROJECT.md | Project management | dev | 2026-02 | active | - |
| DEFICIENCIES.md | Known limitations | dev | 2026-02 | active | PROGRESS.md |
| LEFTOSPEC.md | Unimplemented spec items | dev | 2026-02 | active | specs/v1/ |
| KNOWLEDGE.md | Domain knowledge | spec | 2026-02 | active | - |
| SPEEDUP.md | Performance optimization notes | dev | 2026-02 | active | - |
| REJECTION.md | Rejected design decisions | spec | 2026-02 | active | - |
| REPLICATION-IMPL.md | Replication implementation | dev | 2026-02 | active | specs/v1/DXS.md |
| TEST-VALIDATION-REPORT.md | Test suite validation | dev | 2026-02-13 | active | - |
| PLAYWRIGHT_TESTS_SUMMARY.md | Playwright E2E summary | dev | 2026-02 | active | - |
| DOCUMENTATION.md | Documentation index (this) | dev | 2026-02-13 | active | - |
| DOCS-MANIFEST.md | Documentation tracking | dev | 2026-02-13 | active | DOCUMENTATION.md |
| AGENTS.md | Agent usage patterns | dev | 2026-02 | active | - |
| FRONTEND.md | Frontend design | spec | 2026-02 | active | specs/v1/DASHBOARD.md |
| SCREENS.md | Screen designs | spec | 2026-02 | active | FRONTEND.md |
| index.md | Documentation index page | dev | ? | active | - |

### Archive Candidates

| File | Purpose | Owner | Last Updated | Status | Action |
|------|---------|-------|--------------|--------|--------|
| CRITIQUE.md | Original critique (resolved) | dev | 2026-02-12 | archive | Move to archive/sessions/ |
| CRITIQUE-FINDINGS.md | Critique findings | dev | 2026-02-12 | archive | Move to archive/sessions/ |
| REFINEMENT.md | Refinement session log | dev | 2026-02-13 | archive | Move to archive/sessions/ |
| REFINEMENT-COMPLETE.md | Refinement completion | dev | 2026-02-13 | archive | Move to archive/sessions/ |
| SHIP-STATUS.md | Shipping status snapshot | dev | 2026-02-12 | archive | Move to archive/sessions/ |

---

## Specifications (specs/v1/)

### Core System

| File | Purpose | Owner | Last Updated | Status | Dependencies |
|------|---------|-------|--------------|--------|--------------|
| README.md | Specs overview | spec | 2026-02 | active | - |
| ARCHITECTURE.md | System design | spec | 2026-02 | active | - |
| NETWORK.md | Networking stack | spec | 2026-02 | active | TILES.md, CMP.md |
| TILES.md | Tile thread architecture | spec | 2026-02 | active | notes/SMRB.md |
| CMP.md | C Message Protocol | spec | 2026-02 | active | NETWORK.md |
| DXS.md | Data Exchange System | spec | 2026-02 | active | WAL.md, CMP.md |
| WAL.md | Write-Ahead Log | spec | 2026-02 | active | DXS.md |
| CONSISTENCY.md | Correctness invariants | spec | 2026-02 | active | ORDERBOOK.md |

### Components

| File | Purpose | Owner | Last Updated | Status | Dependencies |
|------|---------|-------|--------------|--------|--------------|
| ORDERBOOK.md | Orderbook structure | spec | 2026-02 | active | ARCHITECTURE.md |
| MATCHING.md | Matching engine | spec | 2026-02 | active | ORDERBOOK.md, CMP.md |
| RISK.md | Risk engine | spec | 2026-02 | active | MATCHING.md, DXS.md |
| GATEWAY.md | WebSocket gateway | spec | 2026-02 | active | NETWORK.md, WEBPROTO.md |
| MARKETDATA.md | Market data broadcast | spec | 2026-02 | active | MATCHING.md, WEBPROTO.md |
| MARK.md | Mark price aggregation | spec | 2026-02 | active | MARKETDATA.md |
| LIQUIDATOR.md | Liquidation engine | spec | 2026-02 | active | RISK.md, MATCHING.md |

### Testing

| File | Purpose | Owner | Last Updated | Status | Dependencies |
|------|---------|-------|--------------|--------|--------------|
| TESTING.md | Overall testing strategy | dev | 2026-02 | active | - |
| TESTING-BOOK.md | Orderbook tests | dev | 2026-02 | active | ORDERBOOK.md |
| TESTING-MATCHING.md | Matching engine tests | dev | 2026-02 | active | MATCHING.md |
| TESTING-DXS.md | WAL/DXS tests | dev | 2026-02 | active | DXS.md, WAL.md |
| TESTING-RISK.md | Risk engine tests | dev | 2026-02 | active | RISK.md |
| TESTING-GATEWAY.md | Gateway tests | dev | 2026-02 | active | GATEWAY.md |
| TESTING-MARKETDATA.md | Market data tests | dev | 2026-02 | active | MARKETDATA.md |
| TESTING-MARK.md | Mark price tests | dev | 2026-02 | active | MARK.md |
| TESTING-LIQUIDATOR.md | Liquidator tests | dev | 2026-02 | active | LIQUIDATOR.md |
| TESTING-CMP.md | CMP protocol tests | dev | 2026-02 | active | CMP.md |
| TESTING-SMRB.md | SPSC ring tests | dev | 2026-02 | active | notes/SMRB.md |

### Protocols

| File | Purpose | Owner | Last Updated | Status | Dependencies |
|------|---------|-------|--------------|--------|--------------|
| WEBPROTO.md | WebSocket message format | spec | 2026-02 | active | GATEWAY.md |
| RPC.md | RPC protocol | spec | 2026-02 | active | WEBPROTO.md |
| REST.md | REST API | spec | 2026-02 | active | GATEWAY.md |
| MESSAGES.md | Internal message types | spec | 2026-02 | active | CMP.md |

### Edge Cases

| File | Purpose | Owner | Last Updated | Status | Dependencies |
|------|---------|-------|--------------|--------|--------------|
| VALIDATION-EDGE-CASES.md | Input validation edges | spec | 2026-02 | active | All TESTING-*.md |
| POSITION-EDGE-CASES.md | Position calculation edges | spec | 2026-02 | active | RISK.md |

### Operational

| File | Purpose | Owner | Last Updated | Status | Dependencies |
|------|---------|-------|--------------|--------|--------------|
| DEPLOY.md | Deployment process | ops | 2026-02 | active | PROCESS.md |
| TELEMETRY.md | Metrics and logging | ops | 2026-02 | active | - |
| DATABASE.md | Database schema | spec | 2026-02 | active | - |
| METADATA.md | Symbol/user metadata | spec | 2026-02 | active | DATABASE.md |
| ARCHIVE.md | Data archival strategy | ops | 2026-02 | active | DXS.md |
| PROCESS.md | Process management | ops | 2026-02 | active | TILES.md |

### Dashboards

| File | Purpose | Owner | Last Updated | Status | Dependencies |
|------|---------|-------|--------------|--------|--------------|
| DASHBOARD.md | General dashboard design | spec | 2026-02 | active | TELEMETRY.md |
| PLAYGROUND-DASHBOARD.md | Trading playground UI | spec | 2026-02 | active | DASHBOARD.md |
| HEALTH-DASHBOARD.md | System health monitoring | ops | 2026-02 | active | TELEMETRY.md |
| RISK-DASHBOARD.md | Risk monitoring | ops | 2026-02 | active | RISK.md |
| MANAGEMENT-DASHBOARD.md | Operations management | ops | 2026-02 | active | PROCESS.md |

---

## Specifications (specs/v2/)

Future planning, not yet implemented.

| File | Purpose | Owner | Last Updated | Status | Dependencies |
|------|---------|-------|--------------|--------|--------------|
| FUTURE.md | v2 planning roadmap | spec | 2026-02 | draft | - |
| IMPLEMENTATION.md | v2 implementation notes | spec | 2026-02 | draft | FUTURE.md |
| ORDERBOOKv2.md | Next-gen orderbook | spec | 2026-02 | draft | specs/v1/ORDERBOOK.md |

---

## Specifications (specs/playground/)

**Status:** Appears duplicate of specs/v1/ content. Verify and remove.

| File | Purpose | Owner | Last Updated | Status | Action |
|------|---------|-------|--------------|--------|--------|
| BLOG.md | ? | ? | ? | obsolete | Verify vs blog/, delete |
| IDEAS.md | ? | ? | ? | obsolete | Verify vs root docs, delete |
| SCREENS.md | ? | ? | ? | obsolete | Verify vs SCREENS.md, delete |
| SPEC.md | ? | ? | ? | obsolete | Verify vs specs/v1/, delete |

---

## Crate Documentation

Each crate has README.md (user-facing) and ARCHITECTURE.md (internals).

| Crate | README | ARCHITECTURE | Owner | Status | Notes |
|-------|--------|--------------|-------|--------|-------|
| rsx-types | Active | Active | dev | active | - |
| rsx-book | Active | Active | dev | active | Consider consolidating |
| rsx-matching | Active | Active | dev | active | Keep separate (complex) |
| rsx-risk | Active | Active | dev | active | Keep separate (complex) |
| rsx-dxs | Active | Active | dev | active | Keep separate (complex) |
| rsx-gateway | Active | Active | dev | active | Keep separate (complex) |
| rsx-marketdata | Active | Active | dev | active | Consider consolidating |
| rsx-mark | Active | Active | dev | active | Consider consolidating |
| rsx-recorder | Active | Active | dev | active | Consider consolidating |
| rsx-cli | Active | Active | dev | active | Consider consolidating |

**Recommendation:** Consolidate ARCHITECTURE.md into README.md for
simpler crates (mark, recorder, cli). Keep separate for complex crates.

---

## Blog Content (blog/)

### Published Posts

| File | Title | Owner | Published | Status |
|------|-------|-------|-----------|--------|
| 01-design-philosophy.md | Design Philosophy | dev | 2026-02 | active |
| 02-matching-engine.md | Matching Engine | dev | 2026-02 | active |
| 03-risk-engine.md | Risk Engine | dev | 2026-02 | active |
| 04-wal-and-recovery.md | WAL and Recovery | dev | 2026-02 | active |
| 05-development-journey.md | Development Journey | dev | 2026-02 | active |
| 06-test-suite-archaeology.md | Test Suite Archaeology | dev | 2026-02 | active |
| 07-port-binding-toctou.md | Port Binding TOCTOU | dev | 2026-02 | active |
| 08-tempdir-over-tmp.md | TempDir Over /tmp | dev | 2026-02 | active |
| 09-poll-dont-sleep.md | Poll Don't Sleep | dev | 2026-02 | active |
| 10-build-system-limits.md | Build System Limits | dev | 2026-02 | active |
| 11-parallel-agent-audits.md | Parallel Agent Audits | dev | 2026-02 | active |
| 12-deleted-serialization.md | Deleted Serialization | dev | 2026-02 | active |
| 13-15mb-orderbook.md | 15MB Orderbook | dev | 2026-02 | active |
| 14-testing-hostility.md | Testing Hostility | dev | 2026-02 | active |
| 15-backpressure-or-death.md | Backpressure or Death | dev | 2026-02 | active |
| 16-dxs-no-broker.md | DXS No Broker | dev | 2026-02 | active |
| 17-asymmetric-durability.md | Asymmetric Durability | dev | 2026-02 | active |
| 18-100ns-matching.md | 100ns Matching | dev | 2026-02 | active |
| README.md | Blog index | dev | 2026-02 | active |

### Drafts

| File | Topic | Owner | Status | Action |
|------|-------|-------|--------|--------|
| cmp.md | CMP protocol | dev | draft | Prefix with DRAFT- |
| picking-a-wire-format.md | Wire format | dev | draft | Prefix with DRAFT- |
| dont-yolo-structs-over-the-wire.md | Serialization | dev | draft | Prefix with DRAFT- |
| flatbuffers-isnt-free.md | FlatBuffers | dev | draft | Prefix with DRAFT- |
| your-wal-is-lying-to-you.md | WAL correctness | dev | draft | Prefix with DRAFT- |

### Session Logs (NOT blog posts)

| File | Purpose | Owner | Status | Action |
|------|---------|-------|--------|--------|
| BLOG-UPDATE-2026-02-13.md | Session transcript | dev | archive | Move to archive/sessions/ |
| BLOG-UPDATE-2026-02-13-v2.md | Session transcript | dev | archive | Move to archive/sessions/ |

---

## Technical Notes (notes/)

Implementation details and patterns.

| File | Topic | Owner | Last Updated | Status | Dependencies |
|------|-------|-------|--------------|--------|--------------|
| SMRB.md | SPSC ring buffer | dev | 2026-02 | active | specs/v1/TILES.md |
| ALIGN.md | Memory alignment | dev | 2026-02 | active | - |
| ARENA.md | Arena allocator | dev | 2026-02 | active | - |
| HOTCOLD.md | Hot/cold path separation | dev | 2026-02 | active | - |
| UDS.md | Unix domain sockets | dev | 2026-02 | active | - |
| PQ.md | Priority queue | dev | 2026-02-13 | active | - |

---

## Reference Materials (refs/)

**150MB of external project documentation.**

### Barter-rs (78 files)

| Category | Files | Status | Recommendation |
|----------|-------|--------|----------------|
| Main | 6 READMEs | reference | Move to external link |
| All | 78 total | reference | Delete or separate repo |

### Firedancer (88 files)

| Category | Files | Status | Recommendation |
|----------|-------|--------|----------------|
| Core | 3 (README, CONTRIBUTING, SECURITY) | reference | Move to external link |
| Book | 16 guides | reference | Extract learnings, delete |
| Internals | 69 technical | reference | Extract learnings, delete |

### RustX (1 file)

| File | Status | Recommendation |
|------|--------|----------------|
| README.md | reference | Move to external link |

**Overall recommendation:** Delete refs/ entirely. If specific patterns
from Firedancer tiles or Barter-rs were used, document them in RSX docs
with citations rather than keeping full external codebases.

---

## Generated/Test Artifacts

Should be ignored by git, not tracked.

### Python .venv (26 files)

- `rsx-playground/.venv/lib/python3.14/site-packages/*/LICENSE.md`
- **Status:** Already in .gitignore
- **Size:** 42MB
- **Action:** None (properly ignored)

### Node Modules (10 files)

- `rsx-playground/tests/node_modules/playwright/lib/agents/*.md`
- **Status:** Already in .gitignore
- **Size:** 14MB
- **Action:** None (properly ignored)

### Pytest Cache (1 file)

- `rsx-playground/.pytest_cache/README.md`
- **Status:** Already in .gitignore
- **Action:** None (properly ignored)

### Test Results (10 files)

- `rsx-playground/tests/test-results/*/error-context.md`
- **Status:** NOT in .gitignore (should be)
- **Action:** Add to .gitignore

---

## Other Files

| File | Purpose | Owner | Status | Action |
|------|---------|-------|--------|--------|
| specs/CODEPATHS.md | Code path analysis | dev | active | Verify relevance |

---

## Maintenance Schedule

### Weekly

- Update PROGRESS.md with completed work
- Review TODO.md and TASKS.md
- Check test reports are current

### Monthly

- Review DEFICIENCIES.md for resolved issues
- Update DOCS-MANIFEST.md with new docs
- Archive completed session logs
- Check for obsolete documentation

### Per Release

- Update README.md with new features
- Review all specs/ for accuracy
- Update blog/ with release post
- Check GUARANTEES.md still holds

---

## Ownership Guidelines

### spec (Specification Team)

- Maintains all specs/v1/ and specs/v2/
- Reviews changes to ARCHITECTURE.md, GUARANTEES.md
- Approves changes to core invariants

### dev (Developers)

- Maintains crate documentation
- Updates PROGRESS.md, TODO.md, TASKS.md
- Writes blog posts
- Updates technical notes/

### ops (Operations)

- Maintains runbooks and monitoring docs
- Updates deployment documentation
- Manages telemetry specs

---

## Last Updated

This manifest: 2026-02-13

Next review: 2026-03-13 (monthly)
