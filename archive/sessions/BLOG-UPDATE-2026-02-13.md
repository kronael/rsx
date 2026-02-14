# Blog Update: 2026-02-13

## New Content Added

### New Post: Parallel Agent Audits (11-parallel-agent-audits.md)

Created comprehensive post documenting the agent-assisted test suite audit from this session.

**Key themes:**
- Using 4 parallel AI agents to audit 960 tests
- Category-based division (race conditions, resource leaks, timing, assertions)
- Finding 90 bugs in 3 hours (vs 3-5 days manual)
- What agents find vs what humans catch
- ROI analysis: 5-8x time savings
- Reusable patterns for other teams

**Why this post matters:**

This session demonstrated a novel approach to code quality that most teams haven't tried. The methodology is actionable, the results are measurable (90 bugs, 3 hours), and the cost analysis is concrete ($820 vs $4000-6400).

Unlike the other test quality posts (06-10) which focus on specific bug types, this post is about the *process* of finding bugs at scale using AI assistance.

## Context from Session

This conversation session involved:

1. **Comprehensive test audit** using parallel subagents
2. **Finding 90 bugs** across multiple categories
3. **Direct fixes** to compilation errors and critical bugs
4. **Build workflow challenges** (disk space pressure from parallel builds)
5. **Test reliability improvements** (unique dirs, ephemeral ports, polling)

## Blog Post Coverage

Posts 06-11 now comprehensively cover the test quality improvements:

- **06: Test Suite Archaeology** - Overview of 90-bug audit results
- **07: Port Binding TOCTOU** - Deep dive on race conditions
- **08: TempDir Over ./tmp** - Resource cleanup patterns
- **09: Poll Don't Sleep** - Timing issues in tests
- **10: Build System Limits** - Resource constraints during parallel builds
- **11: Parallel Agent Audits** - The methodology of agent-assisted review

## What Makes Post 11 Different

While posts 06-10 document *what* bugs were found and *how* to fix them, post 11 documents *how we found them at scale*.

**Unique insights in post 11:**

1. **Division strategy**: Category-based beats file-based for parallel agents
2. **Instruction design**: Specific patterns with examples beat vague tasks
3. **False positive rate**: 15% is acceptable when agents scan 960 tests
4. **Cost analysis**: $820 total vs $4000-6400 for manual audit
5. **Human-agent loop**: Agents scan, humans triage, agents fix, humans validate
6. **Reusable patterns**: Four scanner patterns other teams can copy

## Technical Debt Made Visible

The blog posts make technical debt visible and actionable:

**Before these posts:**
- "Tests are flaky" (vague, not actionable)
- "CI is unreliable" (blame the infrastructure)
- "Local works, CI fails" (give up on reproducibility)

**After these posts:**
- "We have port binding races" (specific bug type)
- "Fixed ports cause TOCTOU" (root cause identified)
- "Ephemeral ports fix it" (solution documented)

Teams reading these posts can:
1. Recognize the patterns in their own code
2. Apply the fixes immediately (code examples provided)
3. Avoid the bugs in future code (checklist in post 11)

## Lessons for Other Teams

**Actionable takeaways across all test quality posts:**

1. Audit tests like production code (06, 11)
2. Use TempDir everywhere (08)
3. Ephemeral ports eliminate races (07)
4. Poll, don't sleep (09)
5. Know your build constraints (10)
6. Use agents to scale pattern recognition (11)

## Documentation Debt Paid

This session also revealed "Documentation drift: 22 bugs" category from the audit. We didn't create a separate post for this because the problem is straightforward:

- Tests claim to test X
- Tests actually test Y
- Documentation says Z

Solution: Read the test, understand what it actually does, fix docs or code.

Not worth a full post (too simple), but worth noting in the audit summary (post 06).

## Future Blog Themes

Additional themes from RSX development that could become posts:

1. **Linter Reversion Bug** - How pre-commit hooks can lose uncommitted work
2. **Advisory Locks for HA** - PostgreSQL advisory locks for leader election
3. **Zero-Copy Message Passing** - SPSC rings and cache-line alignment
4. **Fixed-Point Arithmetic** - Why i64 beats f64 for financial calculations
5. **Spec-First Development** - Writing 35 specs before any code
6. **Dedup Without State** - Using WAL for idempotency tracking

These exist as implicit knowledge in CLAUDE.md and specs but haven't been extracted to blog posts yet.

## Summary

Added one new blog post (11-parallel-agent-audits.md) documenting the agent-assisted test audit methodology from this session.

Updated README.md to include the new post in the test quality section.

Posts 06-11 now form a comprehensive guide to test quality, from specific bug types (07-10) to audit methodology (11) to overall results (06).

Total blog posts: 17 (11 numbered + 6 topical)
Total lines: ~6000 across all posts
New content: ~430 lines in post 11

All existing posts (06-10) remain accurate and didn't need updates based on this session.
