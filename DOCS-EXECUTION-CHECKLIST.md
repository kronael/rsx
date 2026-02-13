# Documentation Cleanup Execution Checklist

Step-by-step commands to execute the cleanup plan.

**Before starting:** Review DOCS-CLEANUP-PLAN.md and get team approval.

---

## Phase 1: Immediate Actions (DONE)

### ✅ 1.1 Update .gitignore (COMPLETED 2026-02-13)

```bash
# Already done - test artifacts now filtered
git diff .gitignore
```

---

## Phase 2: Archive Session Logs (5 minutes)

### 2.1 Create Archive Directory

```bash
cd /home/onvos/sandbox/rsx
mkdir -p archive/sessions
```

### 2.2 Move Session Logs

```bash
# Move root session logs
mv CRITIQUE.md archive/sessions/
mv CRITIQUE-FINDINGS.md archive/sessions/
mv REFINEMENT.md archive/sessions/
mv REFINEMENT-COMPLETE.md archive/sessions/
mv SHIP-STATUS.md archive/sessions/

# Move blog session logs
mv blog/BLOG-UPDATE-2026-02-13.md archive/sessions/
mv blog/BLOG-UPDATE-2026-02-13-v2.md archive/sessions/

# Verify
ls -lh archive/sessions/
# Should show 7 files
```

### 2.3 Create Archive README

```bash
cat > archive/sessions/README.md << 'EOF'
# Archived Session Logs

Historical AI-assisted development session transcripts.

These are completed work logs, kept for reference.

## Contents

- CRITIQUE.md - Original critique session (all 36 items resolved)
- CRITIQUE-FINDINGS.md - Detailed findings from critique
- REFINEMENT.md - Refinement session transcript
- REFINEMENT-COMPLETE.md - Refinement completion summary
- SHIP-STATUS.md - Shipping status snapshot
- BLOG-UPDATE-2026-02-13.md - Blog update session
- BLOG-UPDATE-2026-02-13-v2.md - Blog update session v2

## Policy

- Never commit new session logs to root/blog/
- Archive immediately after completion
- Keep for historical reference only
- Not part of active documentation

Last archived: 2026-02-13
EOF
```

### 2.4 Verify Archive

```bash
# Check file count
find archive/sessions/ -name "*.md" | wc -l
# Should be 8 (7 logs + 1 README)

# Verify root is clean
ls -1 *.md | grep -E "(CRITIQUE|REFINEMENT|SHIP-STATUS)"
# Should be empty

# Verify blog is clean
ls -1 blog/*.md | grep "BLOG-UPDATE"
# Should be empty
```

---

## Phase 3: Remove Reference Materials (2 minutes)

### 3.1 Backup refs/ (Optional)

If you want to keep a backup before deletion:

```bash
# Option A: Create tarball
tar czf /tmp/rsx-refs-backup-$(date +%Y%m%d).tar.gz refs/

# Option B: Move to external location
# mv refs/ /home/onvos/archive/rsx-refs-$(date +%Y%m%d)/
```

### 3.2 Create References Document

```bash
cat > REFERENCES.md << 'EOF'
# Reference Projects

External projects that influenced RSX design. Original documentation
removed (150MB, 167 files) on 2026-02-13.

## Firedancer (Jump Crypto Solana Validator)

https://github.com/firedancer-io/firedancer

Learnings applied to RSX:
- Tile architecture (specs/v1/TILES.md based on disco tiles)
- SPSC ring buffers (notes/SMRB.md)
- Cache-line alignment patterns (notes/ALIGN.md)
- Process isolation and core pinning
- Metrics without Prometheus (structured logging)

Key files referenced:
- src/disco/README.md - Tile architecture
- book/guide/internals/net_tile.md - Network tile design

## Barter-rs (Rust Trading Framework)

https://github.com/barter-rs/barter-rs

Learnings applied to RSX:
- Crate-per-concern structure
- Flat module hierarchies (avoid deep nesting)
- Testing patterns (tests/ directory with _test.rs suffix)
- Re-export patterns in lib.rs

## RustX

https://github.com/example/rustx

General Rust trading system reference.

---

Original refs/ directory removed: 2026-02-13
- 167 markdown files (150MB)
- Backed up to: /tmp/rsx-refs-backup-20260213.tar.gz (if created)
EOF
```

### 3.3 Delete refs/

```bash
# Final check
du -sh refs/
# Should show ~150MB

find refs/ -name "*.md" | wc -l
# Should show 167 files

# Delete
rm -rf refs/

# Verify deletion
ls -d refs/ 2>/dev/null
# Should show "No such file or directory"
```

---

## Phase 4: Reorganize specs/playground/ (3 minutes)

### 4.1 Move Files

```bash
# Move blog draft
mv specs/playground/BLOG.md blog/DRAFT-playground.md

# Move specs to v1/
mv specs/playground/SPEC.md specs/v1/PLAYGROUND-API.md
mv specs/playground/SCREENS.md specs/v1/PLAYGROUND-SCREENS.md
mv specs/playground/IDEAS.md specs/v1/PLAYGROUND-IDEAS.md

# Remove empty directory
rmdir specs/playground/

# Verify
ls -d specs/playground/ 2>/dev/null
# Should show "No such file or directory"
```

### 4.2 Update References

Update any documents that reference specs/playground/:

```bash
# Search for references
grep -r "specs/playground" --include="*.md" .

# Update DOCUMENTATION.md (if needed)
# Update DOCS-MANIFEST.md (if needed)
# Update specs/v1/PLAYGROUND-DASHBOARD.md (if it references these)
```

---

## Phase 5: Prefix Blog Drafts (1 minute)

### 5.1 Rename Drafts

```bash
cd blog/

# Rename drafts
mv cmp.md DRAFT-cmp.md
mv picking-a-wire-format.md DRAFT-picking-a-wire-format.md
mv dont-yolo-structs-over-the-wire.md DRAFT-dont-yolo-structs.md
mv flatbuffers-isnt-free.md DRAFT-flatbuffers-isnt-free.md
mv your-wal-is-lying-to-you.md DRAFT-your-wal-is-lying.md

cd ..
```

### 5.2 Update blog/README.md

```bash
# Edit blog/README.md to reflect DRAFT- prefixes
# This is a manual step - update the drafts section
```

---

## Phase 6: Optional - Consolidate Crate Docs (15 minutes)

Only if you want to reduce redundancy in simple crates.

### 6.1 Check Each Crate

For each simple crate (mark, recorder, cli, types):

```bash
# Example: rsx-mark
echo "=== rsx-mark README ==="
cat rsx-mark/README.md
echo
echo "=== rsx-mark ARCHITECTURE ==="
cat rsx-mark/ARCHITECTURE.md
```

### 6.2 Consolidate If Minimal

If ARCHITECTURE.md is <50 lines and doesn't add much:

```bash
# Append to README.md
echo -e "\n## Architecture\n" >> rsx-mark/README.md
cat rsx-mark/ARCHITECTURE.md >> rsx-mark/README.md

# Remove ARCHITECTURE.md
rm rsx-mark/ARCHITECTURE.md
```

### 6.3 Repeat for Other Simple Crates

Consider for:
- rsx-recorder (simple DXS consumer)
- rsx-cli (CLI tool)
- rsx-types (shared types)

Keep separate for complex crates:
- rsx-matching (complex algorithm)
- rsx-risk (margin/liquidation)
- rsx-dxs (WAL/replication)
- rsx-gateway (networking)
- rsx-book (orderbook structure)
- rsx-marketdata (broadcast logic)

---

## Verification

### Count Files

```bash
# Active markdown files (excluding ignored)
find . -name "*.md" -type f \
  ! -path "./.venv/*" \
  ! -path "./node_modules/*" \
  ! -path "./.pytest_cache/*" \
  ! -path "./test-results/*" \
  ! -path "./.claude/*" \
  | wc -l

# Should be ~110-115 (down from 157, or 224 with refs/)
```

### Check Structure

```bash
# Archive exists
ls -lh archive/sessions/

# refs/ deleted
ls -d refs/ 2>/dev/null || echo "refs/ deleted ✓"

# specs/playground/ deleted
ls -d specs/playground/ 2>/dev/null || echo "specs/playground/ deleted ✓"

# Blog drafts prefixed
ls blog/DRAFT-*.md

# New docs exist
ls DOCUMENTATION.md DOCS-MANIFEST.md DOCS-CLEANUP-PLAN.md REFERENCES.md
```

---

## Git Status

After all changes:

```bash
# See what changed
git status

# Should show:
# - .gitignore modified
# - 4-5 new files (DOCUMENTATION.md, DOCS-*.md, REFERENCES.md)
# - refs/ deleted (167 files)
# - archive/ added (9 files)
# - 7 session logs moved (deleted from root/blog)
# - 4 files moved from specs/playground/
# - 5 blog drafts renamed
# - Optional: crate ARCHITECTURE.md files removed
```

---

## Commit Changes

```bash
# Stage changes in groups

# 1. Documentation system
git add DOCUMENTATION.md DOCS-MANIFEST.md DOCS-CLEANUP-PLAN.md \
        DOCS-QUICK-REFERENCE.md DOCS-SUMMARY.md \
        DOCS-EXECUTION-CHECKLIST.md

# 2. Archive session logs
git add archive/
git rm CRITIQUE.md CRITIQUE-FINDINGS.md REFINEMENT.md \
       REFINEMENT-COMPLETE.md SHIP-STATUS.md
git rm blog/BLOG-UPDATE-2026-02-13.md blog/BLOG-UPDATE-2026-02-13-v2.md

# 3. Remove refs/
git rm -rf refs/
git add REFERENCES.md

# 4. Reorganize specs/playground/
git mv specs/playground/BLOG.md blog/DRAFT-playground.md
git mv specs/playground/SPEC.md specs/v1/PLAYGROUND-API.md
git mv specs/playground/SCREENS.md specs/v1/PLAYGROUND-SCREENS.md
git mv specs/playground/IDEAS.md specs/v1/PLAYGROUND-IDEAS.md

# 5. Rename blog drafts
git mv blog/cmp.md blog/DRAFT-cmp.md
git mv blog/picking-a-wire-format.md blog/DRAFT-picking-a-wire-format.md
git mv blog/dont-yolo-structs-over-the-wire.md blog/DRAFT-dont-yolo-structs.md
git mv blog/flatbuffers-isnt-free.md blog/DRAFT-flatbuffers-isnt-free.md
git mv blog/your-wal-is-lying-to-you.md blog/DRAFT-your-wal-is-lying.md

# 6. Update .gitignore
git add .gitignore

# 7. Optional: crate consolidations
# git rm rsx-mark/ARCHITECTURE.md (etc.)
# git add rsx-mark/README.md (etc.)

# Commit
git commit -m "[docs] Documentation index and cleanup system

- Add DOCUMENTATION.md (comprehensive index)
- Add DOCS-MANIFEST.md (ownership tracking)
- Add DOCS-CLEANUP-PLAN.md (cleanup plan)
- Add DOCS-QUICK-REFERENCE.md (fast lookup)
- Update .gitignore for test artifacts
- Archive session logs to archive/sessions/
- Remove refs/ directory (150MB, 167 files)
- Add REFERENCES.md with external project citations
- Reorganize specs/playground/ into v1/ and blog/
- Prefix blog drafts with DRAFT-
- Reduce 224 files to ~110 active documentation"
```

---

## Rollback (If Needed)

```bash
# Before commit
git reset --hard HEAD

# After commit
git revert HEAD

# Restore specific files
git restore archive/ refs/ specs/playground/ \
            CRITIQUE.md REFINEMENT.md # etc.
```

---

## Post-Cleanup

### Update Related Docs

1. Update PROGRESS.md with documentation cleanup status
2. Update TODO.md to remove documentation cleanup task
3. Verify all links in DOCUMENTATION.md still work
4. Update DOCS-MANIFEST.md with final file counts

### Announce Changes

If working with a team, announce:
- refs/ removed (150MB saved)
- New documentation index (DOCUMENTATION.md)
- Session logs archived (archive/sessions/)
- Blog drafts now prefixed with DRAFT-
- specs/playground/ reorganized into v1/

---

## Success Metrics

Verify these targets:

- ✓ File count reduced from 224 to ~110 (50% reduction)
- ✓ Repository size reduced by 150MB (refs/ removed)
- ✓ Clear documentation index (DOCUMENTATION.md)
- ✓ Ownership tracking (DOCS-MANIFEST.md)
- ✓ Fast lookup guide (DOCS-QUICK-REFERENCE.md)
- ✓ Test artifacts filtered (.gitignore)
- ✓ Session logs archived (not in root/blog)
- ✓ External refs documented (REFERENCES.md)
- ✓ Blog drafts clearly marked (DRAFT- prefix)
- ✓ Playground specs organized (in v1/)

---

## Timeline

| Phase | Time | Risk | Status |
|-------|------|------|--------|
| 1. Update .gitignore | 1 min | Low | ✅ DONE |
| 2. Archive sessions | 5 min | Low | Pending |
| 3. Remove refs/ | 2 min | Low | Pending |
| 4. Reorganize playground/ | 3 min | Low | Pending |
| 5. Prefix blog drafts | 1 min | Low | Pending |
| 6. Consolidate crates | 15 min | Medium | Optional |
| **Total** | **12-27 min** | - | - |

---

Last updated: 2026-02-13
