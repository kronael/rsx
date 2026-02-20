# RSX Engineering Blog

Technical posts from building a perpetual futures exchange in Rust.

## Overview

RSX is a spec-first exchange: matching engine, risk engine, gateway,
market data, and WAL-based recovery. 813 Rust tests + 156 Playwright
tests, ~35k LOC, <50us end-to-end latency target.

These posts document the engineering decisions, bugs, and lessons
learned.

## Architecture & Design

**[Design Philosophy: Why We Built RSX From Scratch](01-design-philosophy.md)**

Spec-first development, zero heap allocation on hot paths, slab
allocators, fixed-point arithmetic, and why we wrote 35 specs before
any code.

**[Matching Engine: FIFO, IOC, FOK, and Post-Only](02-matching-engine.md)**

How the core matching engine works. Price-time priority, order types,
reduce-only, post-only, and the compression map for sparse price levels.

**[Risk Engine: Margin, Liquidations, and Funding](03-risk-engine.md)**

Position tracking, cross-margin calculation, liquidation triggers,
funding payments, and why we store everything as i64 fixed-point.

**[WAL and Recovery: Crash-Safe State Machines](04-wal-and-recovery.md)**

Write-ahead logging, replay semantics, tip tracking, exactly-once
processing, and how we recover from crashes without losing orders.

**[Development Journey: Three Days to Working Exchange](05-development-journey.md)**

Timeline of building RSX. What we built when, what worked, what didn't,
and why specs came first.

## Wire Protocols

**[Don't YOLO Structs Over The Wire](dont-yolo-structs-over-the-wire.md)**

Why `#[repr(C)]` isn't enough. Padding, alignment, endianness, and the
bugs that happen when you cast `&T as *const u8`.

**[FlatBuffers Isn't Free](flatbuffers-isnt-free.md)**

FlatBuffers adds 150ns per message. For <50us end-to-end latency, that's
3% overhead. Why we use raw C structs instead.

**[Picking a Wire Format: FlatBuffers vs Cap'n Proto vs Raw Structs](picking-a-wire-format.md)**

Comparison of serialization formats for ultra-low-latency systems.
Benchmark results and why we chose raw structs.

**[CMP: A UDP Protocol for Financial Data](cmp.md)**

Our UDP-based replication protocol. Sequence numbers, NACKs, flow
control, and how we achieve reliable delivery over unreliable transport.

**[Your WAL Is Lying To You](your-wal-is-lying-to-you.md)**

Write amplification, fsync lies, disk reordering, and why "durable"
doesn't mean what you think. How to actually test persistence.

## Test Quality & Debugging

**[Test Suite Archaeology: Finding 90 Bugs in Production-Ready Code](06-test-suite-archaeology.md)**

Comprehensive audit of 960 tests using parallel agents. Found 90 bugs:
race conditions, resource leaks, incorrect assertions, timing
dependencies. All fixed.

**[Port Binding Races: A Subtle TOCTOU Bug](07-port-binding-toctou.md)**

The bind() → drop() → rebind() pattern creates Time-Of-Check-Time-Of-Use
races. Why fixed ports fail in parallel tests and how ephemeral ports
fix it.

**[Resource Cleanup: TempDir vs Hardcoded Paths](08-tempdir-over-tmp.md)**

Why `./tmp` in tests accumulates garbage and causes parallel execution
failures. How `TempDir` eliminates an entire category of bugs with zero
manual cleanup.

**[The Hidden Cost of time.sleep() in Tests](09-poll-dont-sleep.md)**

Sleep-based tests are slow on fast machines and flaky on slow machines.
Why polling makes tests 4x faster AND more reliable.

**[Build System Limits: When Parallel Workers Fail](10-build-system-limits.md)**

We tried parallel cargo builds and hit 90GB disk usage in a 100GB CI
environment. Why parallel builds have quadratic resource growth and how
to adapt strategies.

**[Parallel Agent Audits: Finding 90 Bugs in 3 Hours](11-parallel-agent-audits.md)**

Used four AI agents in parallel to audit 960 tests. Found 90 bugs across
race conditions, resource leaks, timing issues, and incorrect assertions.
The methodology, what agents found vs missed, and 5-8x speedup vs manual
review.

## Core Innovations

**[We Deleted the Serialization Layer](12-deleted-serialization.md)**

WAL = wire = memory format. No transformation, no encoder, no decoder.
Just memcpy and CRC32. How removing serialization saved 150ns per
message and why disk format = network format is the right choice.

**[How We Fit Bitcoin in 15MB](13-15mb-orderbook.md)**

Distance-based compression zones turn 20M price levels into 617K slots.
1:1 resolution near mid-price, 1:1000 far away. Bisection lookup in
2-5ns. How we made the orderbook fit in L3 cache.

**[Testing Like the System Wants to Lie](14-testing-hostility.md)**

Found 90 bugs by assuming every component is hostile. Position = sum of
fills. Backpressure never drops. Exactly-one completion. Why hostile
testing finds bugs happy-path tests miss.

**[Backpressure or Death: No Silent Drops](15-backpressure-or-death.md)**

When the buffer fills, the system stalls. Never drop data silently.
WouldBlock > silent loss. Small buffers fail fast. Why visible failures
are better than invisible data loss.

**[DXS: Every Producer Is the Broker](16-dxs-no-broker.md)**

No Kafka. No NATS. Producers serve their own WAL over TCP. Consumers
connect directly, replay from sequence number. 10μs latency vs 10ms.
50 lines of code vs Kafka cluster.

**[Fills: 0ms Loss. Orders: Who Cares.](17-asymmetric-durability.md)**

Not all data is equal. Fills are sacred (0ms loss, fsync before send).
Orders are ephemeral (lost on crash, user retries). Positions are
derived (replay fills to rebuild). Asymmetric durability is correct.

**[The Matching Engine That Runs at 100ns](18-100ns-matching.md)**

Single-threaded, pinned core, bare busy-spin. Pre-allocated everything.
Cache-line aligned structs. Fixed-point math. No heap on hot path.
180ns insert, 120ns per fill, 90ns cancel.

## Dev Tooling

**[The Dev Dashboard That Replaced Six Terminals](19-playground-dashboard.md)**

HTMX + FastAPI dev dashboard. Two Python files, zero JavaScript,
10 screens, 156 Playwright tests. Observe, act, verify -- all from
one browser tab.

**[Trade UI Notes: RSX WebUI](25-trade-ui-notes.md)**

React 19 + Vite 6 trading interface. RSX color palette, Bybit-style
grid layout, 60fps rendering targets, ring-buffer trade tape, and
flat component structure.

## Topics Covered

- Ultra-low-latency design (<50us end-to-end, 100ns matching)
- Spec-first development (35 docs before code)
- Zero-heap hot paths (slab allocators, fixed-point math)
- WAL-based recovery (crash-safe state machines)
- UDP replication (CMP protocol)
- Brokerless streaming (DXS: producers serve their own WAL)
- Asymmetric durability (fills sacred, orders ephemeral)
- Compression maps (20M levels → 617K slots)
- Test reliability (TempDir, polling, ephemeral ports)
- Hostile testing (assume components lie, find 90 bugs)
- Resource constraints (disk space, parallel builds)
- Wire protocols (raw structs, no serialization layer)
- Backpressure strategies (stall > drop)
- AI-assisted code review (parallel agent audits)
- Dev dashboards (HTMX, zero build step, Playwright testing)

## Reading Order

**New to the project:** Start with [Design Philosophy](01-design-philosophy.md),
then [Development Journey](05-development-journey.md).

**Core innovations first:** Read [We Deleted the Serialization Layer](12-deleted-serialization.md),
[The Matching Engine That Runs at 100ns](18-100ns-matching.md),
[DXS: Every Producer Is the Broker](16-dxs-no-broker.md).

**Architecture deep-dive:** [Matching Engine](02-matching-engine.md),
[Risk Engine](03-risk-engine.md), [WAL and Recovery](04-wal-and-recovery.md),
[How We Fit Bitcoin in 15MB](13-15mb-orderbook.md).

**Design philosophy:** [Asymmetric Durability](17-asymmetric-durability.md),
[Backpressure or Death](15-backpressure-or-death.md),
[Testing Like the System Wants to Lie](14-testing-hostility.md).

**Debugging similar issues:** Test quality posts (06-11) for practical
lessons on race conditions, resource leaks, build system limits, and
agent-assisted audits.

**Choosing wire formats:** [Picking a Wire Format](picking-a-wire-format.md),
[Don't YOLO Structs](dont-yolo-structs-over-the-wire.md),
[FlatBuffers](flatbuffers-isnt-free.md),
[We Deleted the Serialization Layer](12-deleted-serialization.md).

## Spec Quality Checklist

A spec is executable when it has all of the following. Missing any
one of these caused past plan rejections:

1. **Deploy target** — runtime environment, delivery format (binary,
   container, systemd unit)
2. **Scope boundary** — which crates/components are in scope; which
   are already complete
3. **Success criteria** — specific test suite + coverage threshold,
   not just "tests pass"
4. **Interface spec** — entry points, API surface, protocols, I/O
   surfaces to implement
5. **Edge case format** — where edge cases are documented, what
   completeness means
6. **Current state baseline** — link to PROGRESS.md percentages or
   explicit list of missing features

## Contributing

Found a bug in a post? Open an issue. Have a question? Start a
discussion. Want to write about your own exchange experience? PRs
welcome.

## License

All blog posts are licensed under CC BY 4.0. Code snippets are MIT
licensed (same as RSX codebase).
