# Build System Limits: When Parallel Workers Fail

We thought scaling was simple: more workers = faster builds. Then we hit
90GB disk usage during parallel compilation and the CI runner ran out of
space.

This is the story of discovering resource limits the hard way, and how
to adapt build strategies in real time.

## The Plan: Parallel Everything

We had 90 test files to audit. Sequential auditing would take hours.
Solution: spin up parallel workers.

Initial approach:

```bash
# Divide test files across 4 workers
cargo test --test wal_test &
cargo test --test cmp_test &
cargo test --test tls_test &
cargo test --test gateway_test &
wait
```

On paper: 4x speedup. Each worker compiles and runs tests independently.
Total time: 30 minutes instead of 2 hours.

## What Actually Happened

Worker 1: Started, compiled rsx-dxs (2.1GB target dir)
Worker 2: Started, compiled rsx-dxs again (2.1GB target dir)
Worker 3: Started, compiled rsx-gateway (1.8GB target dir)
Worker 4: Started, compiled rsx-risk (2.3GB target dir)

Total disk usage: 8.3GB... then 15GB... then 40GB...

At 90GB: `No space left on device`

The CI runner has 100GB total. We consumed 90% during compilation.
Builds failed. Workers crashed.

## Why Parallel Cargo Builds Explode

Cargo doesn't share `target/` directories across parallel invocations.
Each `cargo test --test X` gets its own isolated build:

```
target/
  debug/
    deps/           # Shared dependencies
    incremental/    # Incremental state (worker 1)
    incremental-2/  # Worker 2 snapshot
    incremental-3/  # Worker 3 snapshot
    incremental-4/  # Worker 4 snapshot
```

Four workers = four copies of intermediate artifacts. For a large
workspace (9 crates, heavy deps like tokio/rustls), each worker needs
2-3GB.

The space isn't wasted (incremental builds need it), but it accumulates
faster than cleanup can run.

## The Disk Pressure Curve

Timeline of a parallel build:

```
0-2 min:   4 workers start, each allocates 1GB (deps)
2-5 min:   Incremental artifacts grow, 8GB total
5-10 min:  Codegen phase, 20GB total
10-15 min: Link phase, 40GB total
15-20 min: Parallel artifact copies, 60GB total
20 min:    Peak usage 90GB, build fails
```

The peak happens just before cleanup. If you have 95GB free, you might
barely make it. If you have 85GB free, you fail.

## Attempted Fix 1: cargo clean Between Tests

```bash
cargo test --test wal_test
cargo clean  # Delete 2.1GB
cargo test --test cmp_test
cargo clean
```

This worked (stayed under 10GB), but now tests run sequentially. We went
from "parallel and fast" to "serial and slow." Back to 2-hour builds.

## Attempted Fix 2: Shared Target Directory

Set `CARGO_TARGET_DIR` to force sharing:

```bash
export CARGO_TARGET_DIR=/tmp/shared-target
cargo test --test wal_test &
cargo test --test cmp_test &
```

This reduced duplication, but introduced a different problem: Cargo's
lock contention. Multiple `cargo` processes fighting over the same
target directory causes:

- Lock waits (build stalls)
- Incremental state corruption (random compile errors)
- Race conditions in artifact generation

Builds succeeded sometimes, failed randomly other times. Flakiness worse
than disk pressure.

## The Solution: Hybrid Strategy

We adapted mid-audit:

1. **Direct fixes for simple bugs**: Changed code directly, skip build
2. **Background agents for isolated tests**: Long-running test suites
3. **Sequential for complex integration**: Tests touching shared state

This got us:

- Fast feedback on simple bugs (no compilation needed)
- Parallel execution where safe (isolated components)
- Predictable resource usage (no disk explosion)

We finished the audit in 6 hours instead of 2 (parallel build) or 20
(sequential testing).

## Lessons for Build System Design

**1. Parallel builds have quadratic resource growth**

N workers don't use N × resources. They use N × base + N² × incremental
artifacts. At N=4, incremental artifacts dominate.

**2. CI runners are resource-constrained**

Your laptop: 2TB SSD, 32GB RAM, 8 cores.
CI runner: 100GB SSD, 8GB RAM, 2 cores.

Strategies that work locally fail in CI. Always test in CI-like
environments.

**3. cargo clean is a relief valve, not a solution**

If your build process requires `cargo clean` to avoid failure, you have
a resource leak. Fix the leak (reduce artifact size, share target dirs
safely) rather than cleaning repeatedly.

**4. Know your bottleneck**

Is it:
- CPU (compilation time)?
- Disk space (artifacts)?
- Memory (linker)?
- I/O (networked file systems)?

Different bottlenecks need different solutions. We assumed CPU, but disk
was the limit.

## Actionable Strategies

**For local development:**

```bash
# Use sccache to share compilation across workspaces
export RUSTC_WRAPPER=sccache
```

`sccache` caches compiled artifacts across projects. First build: slow.
Subsequent builds: reuse cache. Disk usage: shared across all projects.

**For CI:**

```yaml
# Cache target/ between runs
- uses: actions/cache@v3
  with:
    path: target
    key: ${{ runner.os }}-cargo-${{ hashFiles('Cargo.lock') }}
```

First CI run: build from scratch. Subsequent runs: restore cache, only
compile changes. Massive speedup for incremental PRs.

**For parallel test execution:**

```bash
# Run tests WITHOUT recompilation
cargo build --tests  # Compile once
cargo test --test wal_test --no-fail-fast &
cargo test --test cmp_test --no-fail-fast &
cargo test --test gateway_test --no-fail-fast &
wait
```

One compilation phase (shared target dir), parallel test execution (no
re-compilation). Best of both worlds.

## The Hidden Cost: Cleanup Time

Even when builds succeed, cleanup takes time:

```bash
cargo clean  # Deletes 2.1GB
# Takes 15-30 seconds on HDD, 3-5 seconds on SSD
```

If you're doing `cargo clean` after every test, and you have 50 tests,
that's 150-1500 seconds (2-25 minutes) just deleting files.

On networked file systems (some CI runners use NFS), it's even worse:
each deletion is a network round trip.

## What We Learned

**Before the audit:**
- Assumption: "Parallel builds are always faster"
- Plan: Spin up 4 workers, watch them race
- Reality: Disk exploded, builds failed

**After the audit:**
- Fact: "Parallel builds have resource limits"
- Strategy: Hybrid approach based on bottleneck
- Reality: Audit completed in reasonable time

The key insight: parallelism isn't free. It trades CPU time for disk
space (and sometimes reliability). Know your constraints before scaling.

## When to Use Parallel Builds

**Use parallel when:**
- Workspace has independent crates
- Disk space > 10GB per worker
- Tests are isolated (no shared state)
- Build time dominates test time

**Use sequential when:**
- Disk space is limited
- Tests touch shared resources (DB, files)
- Build time is small compared to test time
- CI runner is constrained

**Use hybrid when:**
- Some tests are fast (unit), some slow (integration)
- Some components are isolated, some shared
- You need fast feedback on simple changes

We ended up using hybrid. It wasn't the fastest possible approach, but
it was the fastest *reliable* approach given our constraints.

## Summary

Parallel builds sound great until you hit resource limits. We tried to
run 4 parallel cargo builds in a 100GB CI environment and consumed 90GB
before failing.

The fix wasn't more parallelism or bigger machines. It was understanding
the bottleneck (disk space, not CPU) and adapting the strategy (hybrid
instead of full parallel).

Build systems have limits. Respect them or they'll respect you (by
failing at the worst moment).

---

**The Rule:** Before scaling builds, measure your bottleneck. Is it CPU,
disk, memory, or I/O? Optimize for the actual constraint, not the
assumed constraint.

---

Related:
- [Test Suite Archaeology](06-test-suite-archaeology.md) on the audit
  that discovered these limits
- [TempDir Over ./tmp](08-tempdir-over-tmp.md) on resource management
  patterns
