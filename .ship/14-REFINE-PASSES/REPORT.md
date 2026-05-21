# 14-REFINE-PASSES — report

**Outcome:** 16 commits over two batches; workspace held green
at **878 tests pass, 0 fail** throughout; wisdom-rule
violations swept; spec contradictions closed.

## Rounds landed (16 of ~46 planned)

### Bucket A — wisdom sweeps (cross-cutting)

| Round | Commit  | Result |
|-------|---------|--------|
| A1 `let _ = Result` sweep | `caa9a5b` | 28 fixes across 8 crates: `?` (1), `if let Err` warn (most), backpressure-rtrb intentional (kept w/ comment) |
| A2 `.unwrap()` sweep      | `b393b95` | 5 sites in non-test code; all already post-check, replaced with `.expect("INVARIANT: ...")` |
| A3 `.expect()` annotate   | `11bad1d` | 12 sites annotated with SAFETY/INVARIANT; flagged 5 hot-path WAL `.expect()` for review |
| A4 panic/todo audit       | (no-op)  | zero macro instances in production; nothing to fix |
| A5 TODO/FIXME audit       | `89e57ec` | one TODO linked to `TODO.md 10-DEPLOY` |
| A6 single-import-per-line | `6f8a581` | 2 trivial fixes; codebase already 99.99% conformed |
| A7 dead code              | `5539ef5` | 18 unused pub re-exports removed; 6 visibility downgrades |
| A8 comment hygiene        | `9e683e1` | 23 history/narration comments removed across 7 crates |

### Bucket B — per-crate hygiene (9 of 12 crates)

| Round | Commit  | Result |
|-------|---------|--------|
| B1 rsx-types  | `5539ef5` | `time_us` + `SlabIdx` dropped (zero callers) |
| B2 rsx-dxs    | `1dafd35` | server.rs CRC+version checks; cmp.rs recv_control CRC; protocol.rs 5 layout asserts; config.rs env-var docs |
| B3 rsx-messages | `f9a6c61` | 22 size+align asserts (11 records × 2) |
| B4 rsx-book   | (in A7 sub) | confirmed clean; hot path heap-free; bench coverage validated |
| B5 rsx-matching | `86f736b` | 5 WAL `.expect()` messages cite specific Invariants from §6-consistency |
| B6 rsx-risk   | `86f736b` | lock-order docstring; TODO marker for T3.2; flagged hot-path `.to_vec()` |
| B7 rsx-gateway | (in B2 batch) | JtiTracker confirmed dormant — TODO at ws.rs:108; rest.rs `let _ =` fix |
| B8 rsx-marketdata | (in B2 batch) | monoio idiom confirmed; bounds-safe frame parsing |
| B9 rsx-mark   | `ea305a8` | trace-log dropped ring-push (was silent) |
| B10 rsx-recorder | (no-op) | clean |
| B11 rsx-cli   | `762308e` | exit-code routing: `die()`/`misuse()` helpers; clippy fixes |
| B12 rsx-maker | `ea305a8` | clippy match-single → if-let |

### Bucket C — per-spec audit (2 of 12)

| Round | Commit  | Result |
|-------|---------|--------|
| C1 spec 4-cmp.md ↔ cmp.rs | `47baf2f` | §6.1 send_ring updated (BTreeMap→preallocated slabs); §10.7 NAK-clamped (was "unclamped") |
| C2 spec 10-dxs.md ↔ server/client | `9ab0f3b` | version-byte handling documented across all 3 ingress paths |

### Bucket D — orthogonality (2 of 8)

| Round | Commit  | Result |
|-------|---------|--------|
| D1 layout asserts (partial) | `b10dd44` | CompressionMap ≤128B asserted (others done in B2/B3) |
| D2 CRC coverage             | (in C1)   | audited; every read_unaligned site post-CRC verified |
| D7 dependency dedup         | `59b1fcb` | base64 0.21→0.22 in rsx-marketdata |

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

## What's still open

- **C3–C12** — remaining spec ↔ code audits (matching/orderbook, risk, marketdata, gateway, mark, messages, tiles, wal, validation-edge-cases, consistency). Subagent budget exhausted; pick up next session.
- **D3** uniform tracing format
- **D5** lib.rs re-export hygiene — attempted then reverted; speculative trim broke tests, not worth the churn
- **D6** test-only API leakage (`pub` → `pub(crate)`)
- **D8** orphan modules — verified: 0 in Rust crates
- **F3** bench-gate run (criterion baselines)
- **F4** Playwright e2e (requires running cluster)
- **F5/F6** REPORT.md (this file) — done

## Flags for next pass / founder

1. **5 hot-path WAL `.expect()` in matching** — design choice per `82a9206` ("matching is authoritative") but flagged for re-litigation if a graceful WAL-stall path is wanted.
2. **rsx-risk hot-path `.to_vec()`** at `shard.rs:764-767, 805-809` — borrow-checker workaround. Real per-BBO heap alloc. Needs ExposureIndex callback API.
3. **`OrderRequest::clone()` at shard.rs:1003** — POD memcpy via `derive(Clone)`. Could derive `Copy` but out of scope.
4. **JtiTracker dormant** at `rsx-gateway/src/ws.rs:108` — replay-protection mechanism shipped but not wired through ws_handshake. Decision needed: per-process tracker (current type) vs shared Redis.
5. **`encode_*_record` helpers in rsx-messages** — 8 of them have zero production callers; candidates for deletion.
6. **`OrderStatus` / `FinalStatus` enums in rsx-types** — exercised only by repr tests; should replace magic u8 `final_status` field or be deleted.
7. **specs/2/45-tiles.md ring-line numbers** drift ~2 lines vs current `rsx-risk/src/main.rs`. Cosmetic.
8. **Replica → main promotion** (`rsx-risk/src/main.rs:~1086`) — `std::env::set_var` + recursive call. T3.2 in 13-A16Z-FIXES.

— file end —
