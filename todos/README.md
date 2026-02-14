# TODO Tracking

Bug hunt 2026-02-14: 59 bugs + 33 spec test gaps + 7 future items.

## Files

| File | Contents | Items |
|------|----------|-------|
| FIX.md | Full bug report with code snippets and fixes | 59 |
| TODO-CRITICAL.md | Must fix before any deployment | 10 |
| TODO-HIGH.md | Fix before production | 16 |
| TODO-MEDIUM.md | Quality improvements | 24 |
| TODO-LOW.md | Optional / defensive | 5 |
| TODO-SPEC-TESTS.md | Tests specified but not implemented | 33 |
| TODO-FUTURE.md | Deferred analysis and spec gaps | 7 |
| TODO-DONE.md | Already fixed (this session) | 11 |

## Bug Summary

| Crate | Bugs | Critical | High | Medium | Low |
|-------|------|----------|------|--------|-----|
| rsx-playground (Python) | 30 | 8 | 8 | 11 | 3 |
| rsx-gateway | 4 | 1 | 1 | 2 | 0 |
| rsx-dxs | 5 | 0 | 2 | 3 | 0 |
| rsx-book | 4 | 0 | 3 | 1 | 0 |
| rsx-risk | 4 | 0 | 2 | 1 | 1 |
| rsx-mark | 3 | 1 | 0 | 2 | 0 |
| rsx-marketdata | 1 | 0 | 0 | 1 | 0 |
| rsx-cli | 1 | 0 | 0 | 1 | 0 |
| start script | 3 | 0 | 0 | 2 | 1 |
| **TOTAL** | **59** | **10** | **16** | **24** | **5** |

## Spec Test Gaps

| Spec File | Missing Tests |
|-----------|---------------|
| TESTING-GATEWAY.md | 11 (heartbeat, config, E2E) |
| TESTING-RISK.md | 11 (integration, replication, full) |
| TESTING-LIQUIDATOR.md | 11 (multi-position, E2E) |
| **TOTAL** | **33** |

## Sources

TODOs consolidated from: PROGRESS.md, GUARANTEES.md, CRITIQUE.md,
LEFTOSPEC.md, specs/v1/TESTING-*.md, specs/v1/LIQUIDATOR.md,
and full codebase bug hunt (9 agent rounds).
