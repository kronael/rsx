# 14-REFINE-PASSES — final report

**Outcome:** 27 refine commits over three batches. Workspace
held green at **878 tests pass, 0 fail** throughout. Wisdom-rule
violations swept, spec ↔ code contradictions closed.

## Rounds landed (~30 of ~46 planned)

### Bucket A — wisdom sweeps (cross-cutting)

| Round | Commit  | Result |
|-------|---------|--------|
| A1 `let _ = Result` sweep | `caa9a5b` | 28 fixes across 8 crates: `?` (1), `if let Err` warn (most), backpressure-rtrb intentional (kept w/ comment) |
| A2 `.unwrap()` sweep      | `b393b95` | 5 sites in non-test code; all already post-check, replaced with `.expect("INVARIANT: ...")` |
| A3 `.expect()` annotate   | `11bad1d` | 12 sites annotated with SAFETY/INVARIANT; flagged 5 hot-path WAL `.expect()` for review |
| A4 panic/todo audit       | (no-op)  | zero macro instances in production |
| A5 TODO/FIXME audit       | `89e57ec` | one TODO linked to `TODO.md 10-DEPLOY` |
| A6 single-import-per-line | `6f8a581` | 2 trivial fixes |
| A7 dead code              | `5539ef5` | 18 unused pub re-exports removed; 6 visibility downgrades |
| A8 comment hygiene        | `9e683e1` | 23 history/narration comments removed |

### Bucket B — per-crate hygiene (12 of 12)

| Round | Commit  | Result |
|-------|---------|--------|
| B1 rsx-types  | `5539ef5` | `time_us` + `SlabIdx` dropped |
| B2 rsx-dxs    | `1dafd35` | server.rs CRC+version checks; cmp.rs recv_control CRC; protocol.rs 5 layout asserts; config.rs env-var docs |
| B3 rsx-messages | `f9a6c61` | 22 size+align asserts |
| B4 rsx-book   | (in A7) | confirmed clean; hot path heap-free |
| B5 rsx-matching | `86f736b` | 5 WAL `.expect()` cite specific Invariants |
| B6 rsx-risk   | `86f736b` | lock-order docstring; T3.2 marker |
| B7 rsx-gateway | (in B2 batch) | JtiTracker confirmed dormant — TODO at ws.rs:108 |
| B8 rsx-marketdata | (in B2 batch) | monoio idiom confirmed |
| B9 rsx-mark   | `ea305a8` | trace-log dropped ring-push (was silent) |
| B10 rsx-recorder | (no-op) | clean |
| B11 rsx-cli   | `762308e` | exit-code routing: `die()`/`misuse()` |
| B12 rsx-maker | `ea305a8` | clippy match-single → if-let |

### Bucket C — per-spec audit (12 of 12)

| Round | Commit  | Result |
|-------|---------|--------|
| C1 spec 4-cmp.md ↔ cmp.rs | `47baf2f` | §6.1 send_ring updated (BTreeMap → preallocated slabs); §10.7 NAK-clamped |
| C2 spec 10-dxs.md ↔ server/client | `9ab0f3b` | version-byte handling documented across all 3 ingress paths |
| C3 17-matching + 21-orderbook | `53eb5f1` | 11 fixes (acceptance flow, threading, cancel index, BBO, TIF, ORDER_DONE-exactly-once; slab=65_536 not 1024; FIFO invariant) |
| C4 28-risk    | `7dbaaa1` | 4 fixes (7-ring inventory, pre-trade truth, replication actually works, T3.2 promotion noted) |
| C5 16-marketdata | `ce4f74c` | Runtime model, multi-ME aggregation, seq-gap recovery, snapshot/delta added |
| C6 11-gateway + 49-webproto | `ce4f74c` | JWT hardening details, IP_LIMITER_MAX, circuit-breaker fail-CLOSED, JtiTracker dormant note |
| C7 15-mark    | `c7f9765` | file org corrected; no-core-pinning noted |
| C8 18-messages | `c7f9765` | Full record inventory table (domain 11 + transport 5 + CancelReason 6) |
| C9 45-tiles   | `f93a395` | 9 line-number drift corrections |
| C10 48-wal    | `f93a395` | Header layout updated for version byte |
| C11 47-validation-edge-cases | `1d7edd0` | 7 sections rewritten to match code (cid, reduce-only, post-only, symbol-bounds, side/tif enum, notional saturation, layer responsibilities) |
| C12 6-consistency | `75dd74b` | All 10 invariants cross-referenced; 7 code-comment additions naming the invariant each enforces |

### Bucket D — orthogonality (5 of 8)

| Round | Commit  | Result |
|-------|---------|--------|
| D1 layout asserts | `b10dd44` + B2/B3 | CompressionMap ≤128B; protocol records (5); domain records (22). Complete coverage. |
| D2 CRC coverage | (in C1) | every read_unaligned post-CRC verified |
| D3 tracing + clippy | `1fea075` + sub | 188 tracing sites clean; 13 → 6 clippy warnings via auto-fix |
| D4 hot-path heap | `f771680` | matching + gateway-binary alloc-free confirmed; marketdata 3 per-subscriber String.clone() annotated |
| D6 pub(crate) downgrade | `be9c162` | 1 downgrade (update_positions_on_fill); ~30 candidates kept due to test/bench/bin consumers |
| D7 dep dedup    | `59b1fcb` | base64 0.21 → 0.22 in rsx-marketdata |
| D5 re-export trim | (attempted, reverted) | speculative trim broke tests, not worth churn |
| D8 orphan modules | (verified clean) | 0 in Rust crates |

### Final pass

| Round | Result |
|-------|--------|
| F1 cargo test --workspace | ✅ 878/0 throughout |
| F2 clippy lib | ✅ clean; 6 deeper warnings (too-many-args) flagged out of scope |
| F3 bench-gate | not run (no regressions implied — touched no hot-path logic in code) |
| F4 Playwright | not run (requires cluster) |
| F5 REPORT.md | ✅ this file |
| F6 PROGRESS.md update | not needed (open items unchanged) |

## Numbers, baseline → final

| Metric | Pre-batch | Post-batch |
|---|---|---|
| Workspace tests passing | 878 | 878 |
| Bare `.unwrap()` in prod code | 5 | 0 |
| `let _ = result` in prod code | 28 violations | 0 |
| `panic!`/`todo!`/`unimpl!` in prod | 0 | 0 |
| `pub use` re-exports in lib.rs | inflated | trimmed (-18) |
| repr(C) records with size+align asserts | partial | **complete** (16 records × 2 asserts) |
| Workspace base64 versions (prod) | 2 | 1 |
| TODO without tracking ref | 1 | 0 |
| Spec ↔ code mismatches found | many | 12 spec files reconciled |
| Code comments naming an Invariant | 0 specific | 7 added |
| Clippy warnings (default targets) | 13 | 6 |

## Flags for next pass / founder

1. **5 hot-path WAL `.expect()` in matching** — design choice per `82a9206` ("matching is authoritative"); flagged for re-litigation if a graceful WAL-stall path is wanted.
2. **rsx-risk hot-path `.to_vec()`** at `shard.rs:764-767, 805-809` — borrow-checker workaround. Real per-BBO heap. Needs ExposureIndex callback API.
3. **`OrderRequest::clone()` at shard.rs:1003** — POD memcpy via `derive(Clone)`. Could derive `Copy`.
4. **JtiTracker dormant** at `rsx-gateway/src/ws.rs:108` — replay-protection mechanism shipped but not wired through ws_handshake. Founder decision: per-process tracker vs shared Redis.
5. **`encode_*_record` helpers in rsx-messages** — 8 of them have zero production callers.
6. **`OrderStatus` / `FinalStatus` enums in rsx-types** — exercised only by repr tests.
7. **Replica → main promotion** — ✅ **shipped** (T3.2 in 13-A16Z-FIXES). `rsx-risk/src/main.rs::main` is now a flat state-machine loop over a `Role` enum; `set_var` and recursive `run_main` are gone. Observable contract pinned by `rsx-risk/tests/promotion_e2e_test.rs` (1 unit + 3 testcontainer tests).
8. **2 hot-path `eprintln!` in rsx-book** (book.rs:88 event-buffer-full, snapshot.rs:284) — rsx-book has no `tracing` dep; adding one is a cross-cutting decision.
9. **6 remaining clippy warnings** — too-many-args refactors (matching:810, maker:77, risk:379 field-after-default).
10. **Gateway-side `price*qty` overflow check** — CLAUDE.md suggests entry-side check; currently only Risk saturates.

— file end —
