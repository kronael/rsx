# 52 — rsx-matching module split

## Goal

`rsx-matching/src/main.rs` is 986 lines and `fn main()` alone spans
`main.rs:127-820` — a single ~694-line function, the same shape `rsx-risk`
had (see `.ship/51-RISK-MODULE-SPLIT/PLAN.md`). Split it into an `app.rs`
composition root plus one module per loop/concern, along the seams that
already exist, per the repo's flat-module philosophy.

Founder's direction (2026-08-10, from 51): no huge mono modules; introduce
`app.rs`; each loop gets its own module. Same direction applies here.

## What's actually monolithic

Measured, not assumed. Outside `main()` the file is already small named
functions: `rebuild_order_index_from_book` (60-72),
`log_effective_matching_config` (74-95), `get_env` (97-110),
`load_config_from_env` (112-125), `emit_config_applied` (822-855),
`maybe_recenter` (867-875), `process_cancel` (884-986).

Inside `main.rs:127-820`, the phases:

- **128-156** process init: panic handler, tracing, latency drainer, env
  config, core pin.
- **158-207** PG connect + applied-config version load (`block_on` on a
  current-thread runtime the loop later reuses for polling).
- **209-281** state recovery: book + `WalWriter` construction, checkpoint
  load (231-245), WAL replay + seq bump (246-271), dedup-window rebuild
  (278-281).
- **283-341** loop timers + cached clock, cast addresses, `CastReceiver` +
  two `CastSender`s, effective-config log.
- **343-376** replication (DXS) sidecar thread spawn.
- **378-388** startup CONFIG_APPLIED emit.
- **390-425** health gauges + health server.
- **427-436** shutdown flag + `CheckpointWriter` start.
- **438-819** the pinned busy-spin loop: clock refresh (439-443), shutdown
  drain (445-465), zero-copy `try_recv_with` callback (474-702) containing
  the ~210-line order path (475-682: dedup gate, dup-reject fan-out, accept
  record, match, publish, index, recenter) and the cancel dispatch
  (683-701), FAULTED gap skip (703-732), idle migration + user maintenance
  (737-745), dedup cleanup (747-750), 10-minute config poll tick (752-788),
  WAL flush (790-792), 10 s checkpoint cadence (794-809), heartbeats
  (811-818).

## Target layout

Flat, one level, no nested `mod` dirs — repo rule. New modules join the
alphabetical `pub mod` list in `lib.rs` (which stays re-export-free, its
current shape); `main.rs` consumes them as `rsx_matching::app::run()`.

### Binary side

| File | Moves from | Owns |
|---|---|---|
| `main.rs` (keeps) | — | thin entry only: `install_panic_handler`, `app::run()` |
| `app.rs` (new) | `main.rs:74-95, 128-436` | composition root: tracing/drainer init, env + DB config load (via `config`), core pin, book + WAL construction, the recovery sequence (calling `wal` primitives), cast socket wiring, replication-sidecar spawn, health server, shutdown wiring, `CheckpointWriter` start, hand-off to `cast_loop::run` |
| `cast_loop.rs` (new) | `main.rs:438-819` minus handler bodies | the pinned busy-spin loop: cached clock, shutdown drain, recv dispatch, FAULTED skip, idle migration + user maintenance, dedup cleanup, config poll tick (calling `config`), flush, checkpoint cadence, heartbeats |
| `order.rs` (new) | `main.rs:475-682, 867-875` | the RECORD_ORDER_REQUEST handler `process_order`: dedup gate, dup-reject fan-out, OrderAccepted record, `process_new_order`, `publish_events`, index update, latency stamps, `maybe_recenter` |
| `cancel.rs` (new) | `main.rs:884-986` | the RECORD_CANCEL_REQUEST handler `process_cancel` (name unchanged): index lookup, checked cancel, event emit, BBO delta, `publish_events` |

Health wiring and the replication-sidecar spawn stay inside `app.rs`
(~35 lines each of pure wiring, not domains) — deliberately not new files.

Loop state hand-off: `main()` threads ~20 mutable locals through the loop.
`cast_loop.rs` defines one plain state struct holding the wired pieces
(book, writer, receiver, senders, dedup, index, gauges, timers, PG client +
runtime + config_version); `app.rs` builds it. A dumb struct only — no
traits, no channels, no new abstraction.

### Move into the domain modules that already exist

Checked `config.rs`, `dedup.rs`, `wal.rs`, `wire.rs` first (the 51 lesson:
the monolith grew second copies of concerns that already have homes).

| Destination (existing file) | What moves | Why it is home |
|---|---|---|
| `config.rs` | `get_env` (97-110, stays private), `load_config_from_env` (112-125), `emit_config_applied` (822-855), the 10-minute poll tick body (753-788) as a named function | `config.rs` already owns the config lifecycle (PG scheduled/applied). Env loading and the applied-record announcement complete it: one owner for "where SymbolConfig comes from and how a change is announced" |
| `wal.rs` | `rebuild_order_index_from_book` (60-72) | `wal.rs` already owns the one order-index maintainer (`update_order_index`, `wal.rs:816-843`) and the replay that drives it; the snapshot-restore rebuild is the same concern. Also heals `wal.rs:159-161` ("Mirror of the type in `main.rs`") — `OrderKey` gets exactly one home |

`dedup.rs` and `wire.rs` need nothing; the loop already calls them at the
right boundary.

### Naming, aligned with the finished crates and with 51

Same convention 51 derived from rsx-cast / rsx-book / rsx-matching:
single domain nouns (`order`, `cancel`, `checkpoint`, `config`, `dedup`,
`wal`, `wire`); compounds only as `_utils` or a genuine `noun_role` pair.
`app.rs` and `cast_loop.rs` are taken verbatim from the signed 51 layout so
the two binary crates end with the same shape (`main` → `app` →
`cast_loop`); whichever split lands first sets the precedent — keep the
names identical. No prefixed compounds (`main_orders.rs`-style) anywhere.

## `wal.rs` (843 lines): mostly coherent — one misfit

**Leave the WAL record path where it is.** Fan-out (`EventSink` +
`emit_events` + `publish_events` + `write_events_to_wal`, 173-551), flush
(555-565), replay (693-767), dedup-window rebuild (786-814), and index
maintenance (820-843) are chained stages of one pipeline over the same WAL
frames, and the record-construction match deliberately lives exactly once
(`wal.rs:186-188`). Splitting producer from consumer of one format would be
worse, not better. No monolithic function exists here — the largest,
`emit_events`, is a single match over event types.

**The one genuine misfit is the checkpoint block**: `MatchingCheckpointV1`
+ `CheckpointWriter` + worker (`wal.rs:46-157`) and
`checkpoint_path`/`read_avro_checkpoint`/`persist_checkpoint`/
`load_checkpoint`/`save_checkpoint` (`wal.rs:567-689`) — ~330 lines that
never touch `WalWriter`/`WalReader`. It is Avro `snapshot.bin` persistence
on its own worker thread, related to the WAL only through the stored
`wal_seq` watermark. **Phase 2 (separate, gated — see Sequencing): extract
`checkpoint.rs`.** Import churn is confined to
`tests/replay_after_snapshot_test.rs` (six `use` lines) and the binary;
benches are unaffected (verified: `benches/harness.rs` and
`matching_bench.rs` import only `update_order_index`,
`write_events_to_wal`, `OrderKey`, `DedupTracker`). No `pub use` shim left
in `wal.rs` — a shim is a second path; update the imports.

## Unit-test convention

- `dedup.rs` already follows `<module>_test.rs`-beside-source
  (`dedup.rs:124-126` + `src/dedup_test.rs`) — the crate knows the
  convention.
- The one breaker is `wal.rs`'s inline `mod latency_tests`
  (`wal.rs:444-522`, carrying `#[allow(clippy::items_after_test_module)]`).
  The split pass moves it to `src/wal_test.rs` via
  `#[cfg(test)] #[path = "wal_test.rs"] mod wal_test;` — a child module
  keeps its access to the private `EventSink`, same trick as `dedup_test`.
  No collision with `tests/wal_test.rs` (different crate). Count stays 1.
- The split makes `process_order`/`process_cancel` unit-testable named
  functions, but adding tests is out of scope for the move pass (identical
  counts is the gate); the behavior is already covered end-to-end by
  `order_processing_test.rs`, `lifecycle_test.rs`, `invariant_test.rs`, and
  the replay suites. Candidates for later: the dup-reject path and the
  cancel drift-warn path.

## Constraints

- **Pure move-and-split. No behavior change** in the same pass. Bugs seen
  during the move go to `BUGS.md`, not fixed. One already spotted: the PG
  connection driver is an inline `tokio::spawn(async move { ... })`
  (`main.rs:172-176`), violating the repo's named-handler rule — record,
  don't fix here.
- Existing public paths (`rsx_matching::{config,dedup,wal,wire}::*`) are
  untouched in phase 1 (only additions); phase 2 renames
  `rsx_matching::wal::{CheckpointWriter, MatchingCheckpointV1,
  load_checkpoint, save_checkpoint}` to `rsx_matching::checkpoint::*` and
  updates the one test file.
- `cargo test -p rsx-matching` must show the same counts before and after
  every commit. Baseline (measured 2026-08-10 at `71873ae5`):
  **68 passed, 0 failed, 8 ignored** across 15 targets — lib unit 8
  (7 dedup + 1 wal inline), `main.rs` unit 0, twelve `tests/` suites
  60 passed / 8 ignored (all 8 ignored are `config_poll_test.rs`
  testcontainers cases), doc-tests 0.
- `make lint` green; default rustfmt only.
- One commit per coherent move, bottom-up so every step compiles green:
  (1) moves into existing modules (`config.rs`, `wal.rs` gains);
  (2) extract `order.rs` + `cancel.rs` from the recv callback;
  (3) extract `cast_loop.rs`; (4) extract `app.rs`, `main.rs` goes thin;
  (5) `wal.rs` inline tests → `src/wal_test.rs`; (6) doc sync;
  (phase 2, gated) `checkpoint.rs`.
- Doc sync is part of the split: `ARCHITECTURE.md`'s Module Layout table
  gains the new rows and its `main.rs:NNN` citations (e.g. 59, 195-200,
  403) are restated against the new homes. Crate `CLAUDE.md` keeper
  sections are claims about behavior, not layout — none regress.

## Sequencing — do NOT start before these clear

1. **Another agent is editing `rsx-matching/tests/` right now**
   (`config_poll_test.rs`). Phase 1 touches only `src/` — start it once the
   tree is quiet, coordinating so no two agents edit the crate
   concurrently. Phase 2 rewrites imports in
   `tests/replay_after_snapshot_test.rs` — hold it until the tests agent
   lands.
2. **The 51 rsx-risk split has not landed** (`rsx-risk/src` has no `app.rs`
   as of `71873ae5`). No file overlap, so the two splits are independent —
   but run them sequentially, not concurrently, per the shared-tree rule,
   and keep `app.rs`/`cast_loop.rs` naming identical across both.

## Open questions for the founder

- Phase 2 (`checkpoint.rs`): recommended, but it is the only step that
  churns public paths and a test file. Sign off separately or drop it —
  phase 1 stands alone either way.
- `order.rs`/`cancel.rs` as separate files vs. named functions inside
  `cast_loop.rs`: recommendation is separate files (the order path is the
  crate's core ~210-line concern and deserves a named home; three small
  files beat one ~500-line loop module), but folding them into
  `cast_loop.rs` is workable if fewer files is preferred.
