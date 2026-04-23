# Spec Cleanup Consolidated Findings

Check-pass completed 2026-04-23 across 48 specs in `specs/2/`.
Raw output: `findings-bucket-{1,2,3,4}.md`.

## Per-spec status recommendations

| Spec | Current | Recommended | Reason |
|------|---------|-------------|--------|
| 1-architecture | shipped | shipped | minor crate-map drift |
| 2-archive | shipped | **draft** | archive replay server + consumer fallback unimplemented |
| 3-cli | shipped | **partial** | proposed improvements shipped but spec not updated; 2 field-name drifts |
| 4-cmp | shipped | shipped | struct-name drift (`WalReplicationServer` → `DxsReplayService`) |
| 5-codepaths | shipped | shipped | 1 test file ref doesn't exist |
| 6-consistency | shipped | shipped | dead link to `PERSISTENCE.md` |
| 7-dashboard | shipped | **draft** | zero implementation (support dashboard not built) |
| 8-database | shipped | **reference** | half impl notes, half design-session artifacts |
| 9-deploy | shipped | **partial** | 8 [STUB] sections, `run.py` → `start` drift |
| 10-dxs | shipped | **partial** | record-type list incomplete; struct details stale |
| 11-gateway | shipped | shipped | REST `/v1/*` shipped but spec says "Post-MVP" |
| 12-health-dashboard | shipped | **draft** | zero implementation |
| 13-liquidator | shipped | **partial** | `max_slip_bps` not wired; §5 vs §10.3 contradiction |
| 14-management-dashboard | shipped | **partial** | API paths drift, audit_log is stdout |
| 15-mark | shipped | shipped | heavy bloat but content accurate |
| 16-marketdata | shipped | shipped | lean and accurate |
| 17-matching | shipped | shipped | terse and correct |
| 18-messages | shipped | **partial** | dedup key drift (tuple vs single) |
| 19-metadata | shipped | shipped | accurate and lean |
| 20-network | shipped | **partial** | fill-notification flow drift, ME "stateless" drift, env-var drift |
| 21-orderbook | shipped | **partial** | OrderSlot struct drift: `sequence` u16 vs u32; `order_id_hi/lo` location |
| 22-perf-verification | shipped | **partial** | `test.skip()` in play_latency.spec.ts still open |
| 23-playground-dashboard | shipped | **partial** | API base path drift, `PLAYGROUND_WRITES_ENABLED` doesn't exist |
| 24-position-edge-cases | shipped | **reference** | reference-manual grade; no code gaps |
| 25-process | shipped | shipped | Risk file list drift |
| 26-rest | shipped | **partial** | only /health + /v1/symbols shipped; 5 endpoints unshipped |
| 27-risk-dashboard | shipped | **draft** | zero implementation |
| 28-risk | shipped | shipped | heavy bloat; content accurate |
| 29-rpc | shipped | shipped | heavy bloat; estimated ~350 → ~100 lines possible |
| 30-scenarios | shipped | shipped | "What Is Broken" all fixed |
| 31-sim | shipped | **fully shipped, archive candidate** | delete or move to specs/1/ |
| 33-telemetry | shipped | **draft** | rsyslog/Vector/Prometheus infrastructure unshipped |
| 34-testing-book | shipped | shipped | B15/B29 GTC-only claim false; IOC/FOK/post-only shipped |
| 35-testing-cmp | shipped | **partial** | §7 file org drift; named E2E files don't exist |
| 36-testing-dxs | shipped | shipped | large aspirational test list |
| 37-testing-gateway | shipped | **partial** | FailureReason 0-7 vs 0-12; heartbeat TODOs now DONE |
| 38-testing-liquidator | shipped | shipped | impl status table stale |
| 39-testing-mark | shipped | shipped | source connector tests unshipped |
| 40-testing-marketdata | shipped | shipped | MD11 epoll→monoio drift |
| 41-testing-matching | shipped | shipped | M23 zero-heap not auto-tested |
| 42-testing-risk | shipped | shipped | 2 false "not wired"/"not implemented" disclaimers |
| 43-testing-smrb | shipped | **reference** | tests concept that's external (rtrb); no rsx-authored files |
| 44-testing | shipped | shipped | test count 877 → 1035; 3 stale deferred items |
| 45-tiles | shipped | shipped | tile diagram vs inline WAL drift; `maybe_flush` naming |
| 46-trade-ui | shipped | **partial** | Fix 3/4/5 shipped; nginx Fix 1/2 external |
| 47-validation-edge-cases | shipped | shipped | §1.4 TIF unsafe example stale |
| 48-wal | shipped | shipped | clean |
| 49-webproto | shipped | shipped | T "Post-MVP" wrong (shipped); Reconnection guidance drift |

## Summary by recommendation

- **shipped**: 23 (keep; light trim)
- **partial**: 14 (real drift or partial implementation — needs research + action)
- **draft**: 5 (mostly/entirely unshipped — needs decision: specs/3/ move or ship now)
- **reference**: 4 (analysis docs, not design specs)
- **archive candidate**: 1 (31-sim.md, fully shipped)

## Aggregate action categories

### (A) DRIFT to resolve — fix spec OR fix code (~20 items)

Highest-value items (vital):
- **13-liquidator §2/§9**: `RSX_LIQUIDATION_MAX_SLIP_BPS` advertised, unimplemented in `LiquidationConfig`. **Fix code.**
- **13-liquidator §7**: main-loop ordering mismatch.
- **18-messages dedup**: key is `(user_id, order_id_hi, order_id_lo)` tuple not single `order_id`. **Fix spec.**
- **20-network fill notification**: fills go ME→Risk→Gateway, not ME→Gateway direct. **Fix spec.**
- **21-orderbook §3 OrderSlot**: `sequence` u16 vs u32; `order_id_hi/lo` are IN slot. **Fix spec.**
- **22-perf-verification**: `test.skip()` in `play_latency.spec.ts:245,298,335`. **Fix code (remove skips).**
- **23-playground-dashboard §4**: API base path `/api/` vs spec's `/v1/api/play/`. **Fix spec** (code is canonical).
- **37-testing-gateway Enum**: FailureReason 0-7 vs 0-12. **Fix spec.**
- **44-testing count**: 877 vs 1035 actual. **Fix spec.**
- **49-webproto T (Trade)**: shipped but spec says "Post-MVP". **Fix spec.**

Tidying (not vital):
- **1-architecture Crate Map**: lists rsx-webui/rsx-playground as Rust crates. **Fix spec.**
- **4-cmp**: `WalReplicationServer` → `DxsReplayService`. **Fix spec.**
- **5-codepaths**: `fanout_test.rs` ref doesn't exist. **Fix spec.**
- **6-consistency §4**: `PERSISTENCE.md` dead link. **Fix spec.**
- **9-deploy**: `run.py` → `start`. **Fix spec.**
- **10-dxs record types**: list 11 vs actual 14+. **Fix spec.**
- **25-process Risk files**: missing `insurance.rs`/`rings.rs`/`schema.rs`. **Fix spec.**
- **40-testing-marketdata MD11**: epoll → monoio. **Fix spec.**
- **42-testing-risk**: 2 false "not wired" claims. **Fix spec.**
- **45-tiles**: `maybe_flush` → `flush`. **Fix spec.**

### (B) UNSHIPPED — vital for publish → new ship project

- **26-rest**: 5 endpoints (account, positions, orders, fills, funding) unshipped; playground serves these for trade UI but gateway doesn't. **Decide**: are these needed for public gateway? If yes → new ship project.
- **33-telemetry**: JSON structured logs + Vector + Prometheus. **Not vital for publish.** → specs/3/
- **9-deploy [STUB] sections**: 8 sections of production ops (Security, HA, Core Pinning, Monitoring, Rolling Upgrades, Backup). **Not vital for repo-only publish.** → specs/3/
- **2-archive**: archive replay server. **Not vital.** → specs/3/

### (C) UNSHIPPED — not vital → move to specs/3/

- **7-dashboard** (support dashboard) → specs/3/
- **12-health-dashboard** → specs/3/
- **27-risk-dashboard** → specs/3/
- **2-archive** content → specs/3/
- **33-telemetry** production sections → specs/3/

### (D) BLOAT to trim (check-pass confirms; no risk)

Major trim targets (estimated line counts):
- **21-orderbook** §§1-2.7, 5, 6, 6.5, 7 — ~500 lines of code-in-spec
- **28-risk** §§2, 3, 6, 7, persistence SQL, main loop — ~400 lines
- **29-rpc** — struct defs, impl blocks, 6-step pseudocode — ~250 lines
- **10-dxs** §§3, 4, 6 — 7 `#[repr(C)]` struct defs
- **4-cmp** §§3, 8 — struct defs
- **13-liquidator** §§1, 4, 10 — struct defs, edge case walls
- **34/35/36/37/38/39/40/41/42-testing-*** — test-name code blocks (~80-300 lines each)

### (E) CONSOLIDATION

- **Dashboards**: 7-dashboard + 12-health-dashboard + 14-management + 23-playground + 27-risk → merge shared "Platform" section, per-module deltas
- **Process/tile arch**: 1-architecture + 25-process + 45-tiles → pick 45-tiles as canonical; others reference
- **Message flow**: 18-messages + 29-rpc duplicate — split by concern (schema vs lifecycle)
- **Correctness invariants**: 44-testing.md canonical; others cross-ref

### (F) DELETE or ARCHIVE

- **31-sim.md**: fully shipped deletion plan; move to specs/1/ as historical
- **43-testing-smrb.md**: tests external (rtrb) concept; demote to reference or delete

## Proposed execution order

1. **Quick wins — spec-side drift fixes** (one pass, ~30min): items in (A) marked "Fix spec" — mechanical, low-risk
2. **Vital code-side drift fixes**: 13-liquidator `max_slip_bps`, 22-perf `test.skip()` — may be own ship task or fold into 06-PUBLISH
3. **Decide vital unshipped items** (B): user input on REST endpoints scope
4. **Move not-vital unshipped to specs/3/** (C): `git mv` + renumber
5. **Bloat trim pass** (D): mechanical, large diff, low-risk
6. **Consolidation** (E): higher-risk reorganization — defer until after trim
7. **Delete/archive** (F): 31-sim.md, 43-testing-smrb.md
8. **Regenerate index.md**
9. **Status frontmatter pass**
10. **Second-audit cycle** to confirm acceptance

## Next session notes

- Recommended starting point: **(A) spec-side drift fixes** — contained, mechanical, builds momentum
- Alternative starting point: **(D) bloat trim** — bigger diff but lower-risk than consolidation
- User must decide **(B) vital unshipped scope** before moving into trim phases
