# Documentation Cleanup Plan

Actionable plan to reduce 224 markdown files to ~110 active files.

See DOCUMENTATION.md for full index, DOCS-MANIFEST.md for tracking.

---

## Summary

**Current:** 224 markdown files
**Target:** ~110 active files
**Reduction:** 114 files (51%)

**Breakdown:**
- Delete refs/ (167 files, 150MB external docs)
- Archive session logs (7 files)
- Ignore test artifacts (26 files, already filtered)
- Verify/delete duplicates (4 files in specs/playground/)
- Optional: consolidate crate docs (10 files)

---

## Phase 1: Immediate Actions (High Impact, Low Risk)

### 1.1 Update .gitignore

**Status:** DONE (2026-02-13)

Added to .gitignore:

```gitignore
# Test artifacts
test-results/
playwright-report/
*.trace
.last-run.json
```

**Impact:** Filters 26 generated markdown files from tracking.

### 1.2 Archive Session Logs

Move to `archive/sessions/` (create directory):

```bash
mkdir -p archive/sessions
mv CRITIQUE.md archive/sessions/
mv CRITIQUE-FINDINGS.md archive/sessions/
mv REFINEMENT.md archive/sessions/
mv REFINEMENT-COMPLETE.md archive/sessions/
mv SHIP-STATUS.md archive/sessions/
mv blog/BLOG-UPDATE-2026-02-13.md archive/sessions/
mv blog/BLOG-UPDATE-2026-02-13-v2.md archive/sessions/
```

**Files:** 7
**Reason:** Historical AI session logs, completed work.
**Impact:** Removes clutter from root and blog/.

### 1.3 Remove Test Artifacts from Tracking

Already handled by .gitignore update:

- `rsx-playground/tests/test-results/` (10 error-context.md files)
- `rsx-playground/.pytest_cache/` (1 README.md)
- `rsx-playground/.venv/` (26 LICENSE.md files)
- `rsx-playground/tests/node_modules/` (10 agent prompts)

**Files:** 47 total (26 in .venv, 10 in node_modules, 11 in test results)
**Impact:** No longer tracked in documentation count.

---

## Phase 2: Reference Materials (High Impact, Medium Risk)

### 2.1 Delete refs/ Directory

**Files:** 167 markdown files (150MB total)

**Contents:**
- Barter-rs: 78 files (Rust trading framework)
- Firedancer: 88 files (Solana validator)
- RustX: 1 file

**Recommendation:** Delete entirely. External project documentation
that isn't directly integrated into RSX.

**Alternative (if learnings are important):**

1. Create `REFERENCES.md` in root:

```markdown
# Reference Projects

## Firedancer (Jump Crypto Solana Validator)

https://github.com/firedancer-io/firedancer

Learnings applied to RSX:
- Tile architecture (TILES.md based on disco tiles)
- SPSC ring buffers (notes/SMRB.md)
- Cache-line alignment patterns (notes/ALIGN.md)

## Barter-rs (Rust Trading Framework)

https://github.com/barter-rs/barter-rs

Learnings applied to RSX:
- Crate structure (per-concern crates)
- Testing patterns (tests/ directory structure)

## RustX (Rust Trading System)

https://github.com/example/rustx

Learnings applied to RSX:
- [Document specific patterns if any]
```

2. Delete refs/ entirely:

```bash
rm -rf refs/
```

**Impact:** Reduces file count by 167, saves 150MB.

---

## Phase 3: Verify and Remove Duplicates (Medium Impact, Medium Risk)

### 3.1 Verify specs/playground/ Content

**Status:** NOT DUPLICATES - These are detailed playground specs!

**Contents:**
- `BLOG.md` - Playground concept/philosophy (blog draft)
- `IDEAS.md` - Playground feature brainstorm
- `SCREENS.md` - Dashboard screen layouts (detailed)
- `SPEC.md` - REST API specification (detailed)

**Relationship to specs/v1/:**
- `specs/v1/PLAYGROUND-DASHBOARD.md` is conceptual overview
- `specs/playground/` contains detailed implementation specs

**Recommendation:** KEEP but reorganize:

```bash
# Move blog draft to blog/
mv specs/playground/BLOG.md blog/DRAFT-playground.md

# Move detailed specs to v1/
mv specs/playground/SPEC.md specs/v1/PLAYGROUND-API.md
mv specs/playground/SCREENS.md specs/v1/PLAYGROUND-SCREENS.md
mv specs/playground/IDEAS.md specs/v1/PLAYGROUND-IDEAS.md

# Remove empty directory
rmdir specs/playground/
```

**Files:** 0 deleted (reorganized only)
**Impact:** Better organization, clearer hierarchy.

### 3.2 Consolidate Frontend Docs

**Current:**
- `FRONTEND.md` (root)
- `SCREENS.md` (root)
- `index.md` (root)
- `specs/v1/DASHBOARD.md`
- `specs/v1/PLAYGROUND-DASHBOARD.md`
- `specs/v1/HEALTH-DASHBOARD.md`
- `specs/v1/RISK-DASHBOARD.md`
- `specs/v1/MANAGEMENT-DASHBOARD.md`

**Recommendation:**

Option A: Keep all (well-organized, different purposes)
Option B: Move FRONTEND.md and SCREENS.md into specs/v1/
Option C: Consolidate into single specs/v1/DASHBOARDS.md

**Impact:** Minimal (2-3 files at most).

---

## Phase 4: Optional Consolidations (Low Impact, Low Risk)

### 4.1 Consolidate Crate ARCHITECTURE.md into README.md

Many crate ARCHITECTURE.md files are <50 lines and could be merged
into README.md.

**Keep separate (complex crates):**
- rsx-matching (matching algorithm details)
- rsx-risk (margin/liquidation logic)
- rsx-dxs (WAL format/replication)
- rsx-gateway (networking/protocol)

**Consider consolidating (simpler crates):**
- rsx-mark (mark price aggregation)
- rsx-recorder (simple DXS consumer)
- rsx-cli (CLI tool)
- rsx-types (shared types)
- rsx-book (if just orderbook structure)
- rsx-marketdata (broadcast logic)

**Process for each crate:**

1. Read both files:
   ```bash
   cat rsx-mark/README.md
   cat rsx-mark/ARCHITECTURE.md
   ```

2. If ARCHITECTURE.md adds minimal value, merge:
   ```bash
   # Append ARCHITECTURE.md to README.md
   echo -e "\n## Architecture\n" >> rsx-mark/README.md
   cat rsx-mark/ARCHITECTURE.md >> rsx-mark/README.md
   rm rsx-mark/ARCHITECTURE.md
   ```

**Files:** Up to 6-10 (if consolidating simpler crates)
**Impact:** Reduces redundancy, easier maintenance.

### 4.2 Prefix Blog Drafts

Clearly mark drafts:

```bash
cd blog/
mv cmp.md DRAFT-cmp.md
mv picking-a-wire-format.md DRAFT-picking-a-wire-format.md
mv dont-yolo-structs-over-the-wire.md DRAFT-dont-yolo-structs.md
mv flatbuffers-isnt-free.md DRAFT-flatbuffers-isnt-free.md
mv your-wal-is-lying-to-you.md DRAFT-your-wal-is-lying.md
```

**Files:** 0 (rename only, not deletion)
**Impact:** Clearer status for blog readers.

---

## Phase 5: Maintenance Updates (No File Changes)

### 5.1 Update blog/README.md

Add table of contents with status indicators:

```markdown
# RSX Blog

## Published Posts

1. [Design Philosophy](01-design-philosophy.md)
2. [Matching Engine](02-matching-engine.md)
...
18. [100ns Matching](18-100ns-matching.md)

## Drafts

- [CMP Protocol Deep Dive](DRAFT-cmp.md)
- [Picking a Wire Format](DRAFT-picking-a-wire-format.md)
- [Don't YOLO Structs Over the Wire](DRAFT-dont-yolo-structs.md)
- [FlatBuffers Isn't Free](DRAFT-flatbuffers-isnt-free.md)
- [Your WAL is Lying to You](DRAFT-your-wal-is-lying.md)
```

### 5.2 Add refs/README.md (if keeping refs/)

If Phase 2 decision is to keep refs/ with documentation:

```markdown
# Reference Projects

External project documentation for learning purposes.

**WARNING:** These are NOT part of RSX codebase. They are reference
materials for architectural patterns.

## Firedancer (150MB)

Solana validator by Jump Crypto.

Key learnings:
- Tile architecture → RSX TILES.md
- SPSC rings → notes/SMRB.md
- See: refs/firedancer/book/

## Barter-rs

Rust trading framework.

Key learnings:
- Crate structure
- Testing patterns

## RustX

Another Rust trading system (reference only).
```

---

## Execution Order

### Immediate (5 minutes)

1. ✅ Update .gitignore (DONE)
2. Archive session logs (7 files)

### High Priority (15 minutes)

3. Delete refs/ directory (167 files, 150MB)
4. Verify and delete specs/playground/ (4 files)

### Optional (30 minutes)

5. Consolidate crate ARCHITECTURE.md files (6-10 files)
6. Prefix blog drafts with DRAFT- (rename only)
7. Update blog/README.md

---

## Before/After

### Before

```
224 total markdown files
- 70 active documentation
- 47 test artifacts (.venv, node_modules, test-results)
- 167 reference materials (refs/)
- 7 session logs
- 4 duplicates (specs/playground/)
```

### After (Phase 1-3)

```
113 active markdown files
- 73 active documentation (4 playground specs reorganized, not deleted)
- 47 filtered by .gitignore (not counted)
- 0 reference materials (deleted)
- 0 session logs (archived)
- 0 duplicates (specs/playground/ were unique)
```

### After (Phase 1-4, with consolidation)

```
100-104 active markdown files
- 60-64 active documentation (after consolidation)
- 47 filtered by .gitignore
- 0 reference materials
- 0 session logs
- 0 duplicates
```

---

## Risk Assessment

### Low Risk (Do Immediately)

- Update .gitignore ✅
- Archive session logs (move to archive/)
- Delete refs/ (external docs)

### Medium Risk (Verify First)

- Delete specs/playground/ (check for unique content)
- Consolidate crate ARCHITECTURE.md (check each crate)

### No Risk (Metadata Only)

- Prefix blog drafts
- Update blog/README.md
- Add refs/README.md (if keeping refs/)

---

## Success Metrics

- Documentation count: 224 → ~110 files (51% reduction)
- Repository size: Reduce by 150MB (refs/ removal)
- Maintenance: Clearer organization, less duplication
- Discoverability: Better index (DOCUMENTATION.md)

---

## Rollback Plan

All changes are safe to rollback:

```bash
# Restore session logs
git restore CRITIQUE.md REFINEMENT.md SHIP-STATUS.md ...

# Restore refs/
git restore refs/

# Restore specs/playground/
git restore specs/playground/

# Restore crate ARCHITECTURE.md files
git restore rsx-*/ARCHITECTURE.md
```

Session logs are moved (not deleted), can be restored from archive/.

---

## Next Steps

1. Review this plan with team
2. Execute Phase 1-2 immediately (low risk)
3. Schedule Phase 3-4 for next sprint
4. Update DOCS-MANIFEST.md after each phase
5. Monitor for broken links or missing docs

---

## Maintenance

After cleanup, enforce documentation hygiene:

- New specs go in specs/v1/
- Session logs go in archive/sessions/ (never commit)
- Blog drafts prefixed with DRAFT-
- Test artifacts stay in .gitignore
- External references documented, not copied

Update this plan as documentation evolves.

Last updated: 2026-02-13
