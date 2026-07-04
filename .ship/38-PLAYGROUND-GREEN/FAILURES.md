# Playground clean-run failures (12) — triage + fix

Clean baseline (maker fix + no manual interference): **190/386 passed, 12
failed, 184 skipped** (play-full subset). All 12 failed in BOTH runs → genuine,
predate the ribbon restyle. Cluster is up 6/6 for reproduction; the maker must
be (re)started for activity-dependent tests (`POST /api/maker/start?confirm=yes`
with `x-confirm: yes`).

Rule per failure: reproduce, decide if the **UI/code is correct** (→ fix the
stale test) or **broken** (→ fix the code). Minimal changes. Don't touch nav
structure/tab count/routes Playwright asserts on beyond the specific fix.

| # | test | error | likely |
|---|------|-------|--------|
| 81 | play_risk.spec.ts:228 user action buttons HTMX attrs | `Create User` button `hx-post` is `""`, expected `./api/users/create` | CODE — button missing hx-post |
| 103 | play_wal.spec.ts:195 wal_size_agrees_with_verify (F3) | regex match on WAL html is null (line 210) | check UI renders wal size/stream |
| 107 | play_readiness.spec.ts:79 maker running=true | `maker not running (exchange offline)` — maker died mid-run | maker stability (see note) |
| 139 | play_overview.spec.ts:101 ring backpressure card | heading `Ring Backpressure` not found | UI heading renamed/missing vs test |
| 148 | play_topology.spec.ts:58 core affinity | got `cpus.../no pinning`, test wants `/Core\|no processes/i` | wording mismatch (dev = no pinning) |
| 154 | play_topology.spec.ts:148 cast_counters (F9) | html has `ME -> Mktdata`, test wants escaped `ME -&gt; Mktdata` | escaping mismatch |
| 165 | play_health_truthful.spec.ts:81 pill not green on partial outage | 15s timeout | injects outage; check pill logic |
| 170 | play_safety.spec.ts:148 gateway restart recovers order flow | `res.ok()` false (line 212) | restart/recovery API |
| 177 | play_safety.spec.ts:352 session collision returns 409 | got 200, expected 409 | CODE — collision not 409 |
| 188 | play_safety.spec.ts:600 stale_orders_counts_string_timestamp | `no stale-orders count rendered` (null, line 618) | UI/data |
| 192 | play_safety.spec.ts:709 orders page works w/ gateway down | `res.ok()` false (line 731) | graceful degradation |
| 210 | play_latency.spec.ts:410 probe_matches_fill_to_its_own_cid (F22) | toBe(expected) false | latency probe / needs maker fills |

**Maker note:** started by global-setup (fix committed) but does NOT stay up
through the run. The md subscription circuit-breaks (marketdata `handshake
failed: connection closed during handshake`), but that alone is non-fatal (a
manually-started maker kept quoting). Investigate why the test-run maker exits
(gateway restart dropping its ws? a crash on md-breaker?) — make it resilient or
have the harness keep it alive. Several activity tests (103, 188, 210) may
depend on it.
