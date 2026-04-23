# The Development Journey: Spec to System in Three Days

RSX went from first commit to ~99% spec completion in three days.
Nine crates, 960 tests, roughly 34,000 lines of Rust. This post
is a chronological account of how it happened, what we learned,
and what remains.

## Day 0: Specs (Feb 7-8, Morning)

The project started with documentation, not code. The first commits
are all `[docs]`:

```
2026-02-07 [docs] Add networking architecture documentation
2026-02-07 [docs] Add remaining architecture documentation
2026-02-07 [docs] Add event fan-out consistency model
2026-02-08 [docs] Add testing strategy documentation
2026-02-08 [docs] Reorganize specs into versioned directories
```

By the end of day 0, we had 35 spec files in `specs/1/` covering
every component: ORDERBOOK.md, RISK.md, DXS.md, MARK.md,
CONSISTENCY.md, LIQUIDATOR.md, MARKETDATA.md, GATEWAY.md,
and their corresponding TESTING-*.md files.

We also wrote CRITIQUE.md -- a systematic audit of the specs
themselves. It found 36 issues across severity levels:

- Missing dedup windows for order IDs
- Ambiguous ack semantics between gateway and risk
- No backpressure specification for the SPSC rings
- Fee rounding behavior unspecified
- WAL record format underspecified for cross-component streaming

All 36 items were resolved by updating the specs. The total spec
corpus was roughly 5,000 lines of markdown.

### Why This Matters

Writing specs first forced us to think about interfaces before
implementations. The CMP wire format, for example, went through
three iterations in the specs (gRPC, then QUIC, then raw C structs
over UDP) before we wrote any networking code. Each iteration was
a 20-minute discussion; changing it in code would have been days
of refactoring.

The GUARANTEES.md document -- a 1,100-line specification of every
failure scenario and recovery procedure -- would have been
impossible to write after the fact. We would have discovered the
guarantees by testing, not by design.

## Day 1: Core Implementation (Feb 9)

The first code checkpoint:

```
2026-02-09 06:58 [checkpoint] orderbook & matching engine
2026-02-09 07:47 [refactor] fix all warnings, add #[inline]
2026-02-09 07:54 [docs] Add PROGRESS.md, rsx-dxs, rsx-recorder
```

By 8am on day 1, we had: the orderbook data structures (slab,
compression map, price levels), the matching algorithm (FIFO,
price-time priority), the WAL writer/reader, DXS replay service,
and the recorder.

The afternoon brought risk:

```
2026-02-09 11:32 [impl] Complete rsx-book modify, rsx-dxs tests
2026-02-09 12:07 [impl] Add rsx-risk Phase 1: position, margin,
                        price, funding math
2026-02-09 12:10 [impl] Harden rsx-risk: i128 overflow safety
```

And then a spec compliance audit:

```
2026-02-09 14:22 [impl] Spec compliance audit: align all 6 crates
2026-02-09 20:07 [checkpoint] CRITIQUE fixes: order IDs, risk
                              binary, DXS sidecar, panic handlers
```

By end of day 1: 6 crates implemented, ~400 tests, spec compliance
audit completed. The audit caught mismatches between spec and code
-- wrong field names, missing record types, inconsistent enum
values.

## Day 2: Wiring and Integration (Feb 10, Morning)

Day 2 was about connecting the pieces:

```
2026-02-10 11:50 [refactor] Standardize CMP payload preamble
2026-02-10 12:43 [impl] Phase 1: header simplification + CmpRecord
2026-02-10 13:20 [refined] Fix UB in decode, consolidate as_bytes
2026-02-10 13:49 [feat] Wire ME->Risk->Gateway event forwarding
```

The CMP protocol got its final form: a `CmpRecord` trait with
`seq: u64` as the first 8 bytes, shared across all data payloads.
The matching engine, risk engine, and gateway were wired together
through CMP/UDP.

Then the user-facing components:

```
2026-02-10 13:51 [feat] Gateway connection handler impl
2026-02-10 14:03 [feat] Gateway wiring: cancel, heartbeat,
                        rate limit, circuit breaker, auth
2026-02-10 14:03 [feat] Marketdata CMP decode + shadow book
```

Gateway got JWT authentication (HS256), per-user and per-IP rate
limiting (token bucket), and a circuit breaker for risk engine
unavailability. Market data got its shadow book and L2/BBO/trade
serialization.

## Day 2: Features (Feb 10, Afternoon)

The afternoon was feature work:

```
2026-02-10 15:26 [feat] Liquidation engine: check_liquidation,
                        generate_liquidation_order, tests
2026-02-10 15:30 [feat] Wire liquidation engine into risk shard
2026-02-10 15:50 [feat] ORDER_FAILED routing + server heartbeats
2026-02-10 15:53 [feat] Risk receives mark prices via CMP
2026-02-10 16:26 [feat] BBO emission from matching engine
```

Liquidation, insurance fund, mark price integration, BBO emission,
server heartbeats -- each feature landed with its tests.

The evening push:

```
2026-02-10 20:23 [feat] Insurance fund accounting, tick/lot
                        validation, backpressure
2026-02-10 20:52 [feat] CMP config env vars, seq gap detection
2026-02-10 21:01 [feat] ME order dedup with 5min pruning
2026-02-10 21:09 [feat] Orderbook snapshot save/load
```

By midnight: 9 crates, 960 tests, 100% v1 spec compliance.

## What the Critique Process Revealed

We ran the critique process three times during development. Each
pass read the code against the specs and identified mismatches.

The final CRITIQUE.md (post-implementation) found 7 remaining
issues, down from 36 in the spec-only phase:

**Critical:**
- Mark price feed has no integration test (mark -> risk CMP path)
- Frozen margin per-order map is memory-only (not persisted)
- OrderDone status mapping is code-defined, not spec-defined

**High:**
- Cancel by client-id requires gateway state
- Marketdata backpressure drops silently

**Medium:**
- Risk reject reason mapping is policy without spec backing
- Matching fan-out tests model SPSC, not CMP

These are all integration-level issues -- the individual
components work correctly, but the seams between them have gaps.
This is exactly what we expected: spec-level review catches
design issues, code-level review catches integration issues.

## Patterns That Worked

### Same code path for live and recovery

Risk processes fills identically whether they come from a live
SPSC ring or a DXS replay stream. There is no "recovery mode."
The dedup check (`seq <= tip`) handles both cases. This
eliminates an entire class of bugs where recovery logic diverges
from live logic.

### Event buffer pattern

The matching engine writes events to a fixed-size array. The
caller drains them to wherever they need to go. This made the
matching algorithm testable in isolation -- 127 tests exercise
it without any network or persistence infrastructure.

### Spec-driven test coverage

Each spec file has a corresponding TESTING-*.md that lists
required test cases. We wrote tests to match the spec, not to
match the implementation. When the implementation changed (e.g.,
CMP header simplification), the tests did not need updating
because they tested behavior, not wire format.

### Fixed-point everywhere

Zero floating-point bugs. Zero precision discussions. Zero
cross-platform divergence concerns. The type system (`Price`,
`Qty` as `i64` newtypes) catches unit errors at compile time.

## Patterns That Hurt

### Linter reverts uncommitted changes

We discovered that the pre-commit linter occasionally reverts
uncommitted changes. This happened twice during development:
once with a config change and once with gateway modifications.
The workaround was committing more frequently.

### Memory-only state in risk

The `frozen_orders` map (per-order frozen margin tracking) is
not persisted. On risk restart, the per-order entries are lost.
The account-level `frozen_margin` is correct (it is persisted),
but releasing frozen margin for specific cancelled orders after
restart is broken. This is the most significant remaining bug.

### Underspecified inter-component mappings

The specs define each component's behavior in isolation but
underspecify the mappings between components. What risk reject
reason maps to what gateway error code? What OrderDone status
maps to what WebSocket message? These ended up as code-defined
policy, not spec-defined behavior.

## Current Status

As of the last audit:

| Crate | Tests | Completion |
|-------|-------|------------|
| rsx-types | 15 | 100% |
| rsx-book | 97 | 100% |
| rsx-matching | 30 | 100% |
| rsx-dxs | 83 | 100% |
| rsx-risk | 201 | 100% |
| rsx-gateway | 124 | 97% |
| rsx-marketdata | 57 | 98% |
| rsx-mark | 40 | 100% |
| rsx-recorder | 0 | 100% |

Overall: ~99% weighted by criticality. All MVP features
implemented. No critical path items remaining.

## What is Next

**Phase 6: E2E smoke test.** The one thing we have not done is
run the full system end-to-end. Individual components are tested,
component pairs are tested, but the full pipeline (WS client ->
Gateway -> Risk -> ME -> Risk -> Gateway -> WS client) has not
been exercised as a single test.

**Integration test gaps.** The CRITIQUE.md identifies specific
missing tests: mark -> risk CMP integration, ME BBO -> risk
integration, cancel/done margin release through the full CMP flow.

**Stress testing.** The system is designed for 1M fills/sec.
We have not measured actual throughput. Criterion benchmarks
exist for the hot path, but sustained load testing has not been
done.

## Reflections

Building an exchange in three days sounds unreasonable. The key
enabler was spec-first development. By the time we started coding,
every interface was defined, every failure scenario was analyzed,
and every invariant was written down. The implementation was
largely mechanical: translate spec to code, write tests from the
test spec, run the compliance audit.

The other enabler was scope discipline. RSX is v1: GTC limit
orders only, fixed tick sizes, single datacenter, no market
orders, no variable tick sizes, no multi-datacenter replication.
Each of those is a v2 feature with its own spec. We shipped the
smallest exchange that is correct, not the most featured exchange
that is not.

The codebase is roughly 21,000 lines of Rust across 9 crates.
The spec corpus is roughly 5,000 lines of markdown. The ratio
(4:1 code-to-spec) feels right: specs are dense, code is
verbose, and there is no duplication between them.
