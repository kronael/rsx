# 51 — rsx-risk module split

## Goal

`rsx-risk` is the largest crate in the workspace (4651 src lines) and carries
the two biggest files: `main.rs` (1012) and `shard.rs` (977) — 43% of the
crate in two files. Split them along the seams that already exist, per the
repo's flat-module philosophy: one level of modules, each doing one thing.

Founder's direction (2026-08-10): no huge mono modules; introduce `app.rs`;
each loop gets its own module.

## What's actually monolithic

Measured, not assumed:

- **`main.rs:283-984` — `run_main` is a single ~700-line function.** That is
  the real monolith; the rest of the file is small (`main` 138-253,
  `persist_thread_body` 253-274, `rand_jitter`, `stop_persist_worker`,
  `log_effective_risk_config`, plus `NodeState` and `MeReceiverState`).
  `run_main` currently owns: config load, ring construction, thread spawn,
  the promotion state machine, the pinned cast receive loop, lease watching,
  and shutdown.
- **`shard.rs` — `RiskShard` has ~30 methods across seven concerns.** The
  struct is a legitimate single owner of shard state; the *impl* is what
  sprawls.

## Target layout

Flat, one level, no nested `mod` dirs — repo rule.

### Binary side

| New file | Moves from | Owns |
|---|---|---|
| `main.rs` | — | thin entry only: install panic handler, call `app::run()` |
| `app.rs` | `main.rs:138-253`, parts of `run_main` | composition root: load config, build rings, spawn threads, wire shutdown |
| `promotion.rs` | `NodeState` (81), promotion logic in `run_main` | warm-catchup → caught-up → non-blocking acquire state machine |
| `cast_loop.rs` | the receive loop inside `run_main` | the pinned, busy-spin UDP hot loop |
| `persist_loop.rs` | `persist_thread_body` (253), `stop_persist_worker` (984) | tokio sidecar: PG write-behind drain + shutdown |
| `me_receiver.rs` | `MeReceiverState` (93) | ME stream receive state |

`lease.rs` already exists and keeps the lease-renewal loop; `app.rs` spawns it
rather than inlining the watch.

### Naming, aligned with the finished crates

Studied `rsx-cast`, `rsx-book`, `rsx-matching` before cutting. Their
convention, which this split must follow:

- **Module names are single domain nouns**, usually singular: `cast`,
  `header`, `records`, `wal`, `tls` / `book`, `level`, `order`, `slab`,
  `event`, `snapshot`, `user`, `compression`, `occupancy` / `dedup`, `wire`.
- **Compound names appear only in two cases**: the `_utils` suffix for
  stateless helpers (`encode_utils`, `time_utils`, `risk_utils`), and a
  `noun_role` pair when one theme has two genuine implementations
  (`replication_client` / `replication_server`).
- **Unit tests are `<module>_test.rs` beside the source** (`cast_test.rs`,
  `wal_test.rs`, `occupancy_test.rs`, `dedup_test.rs`). rsx-risk has **zero**
  of these — every test is in `tests/`. That is a second, separate gap.
- `lib.rs` is an alphabetical `pub mod` list followed by alphabetical
  one-per-line `pub use` re-exports of the key types.

**This kills the `shard_orders.rs` / `shard_fills.rs` scheme** in the first
draft of this plan — prefixed compound names appear nowhere in the finished
crates.

### Shard side — move into the domain modules that already exist

The stronger consequence: rsx-risk **already has** `account.rs`,
`position.rs`, `margin.rs`, `funding.rs`, `liquidation.rs`, `price.rs`,
`insurance.rs`, `persist.rs`, `config.rs`. `shard.rs` grew a second copy of
each concern as `impl RiskShard` methods. So this is not "split `shard.rs`
into new files" — it is **move each method home to the module that already
owns its domain**, which is the repo's amend-the-original rule rather than a
new parallel path.

`RiskShard` stays one struct in `shard.rs`; Rust allows `impl RiskShard`
blocks across files in the same crate, so the moves change no call sites.

| Destination (existing file) | Methods moved from `shard.rs` |
|---|---|
| `position.rs` | `process_fill`, `update_exposure`, `ensure_position`, `iter_positions_for_user` |
| `margin.rs` | `process_order`, `confirm_freeze`, `release_frozen_for_order`, `replay_freeze_order`, `frozen_for_user` |
| `liquidation.rs` | `check_liquidation_for`, `liq_to_order`, `halt_liquidation`, `resume_liquidation`, `halt_liquidation_for_user` |
| `insurance.rs` | `process_socialized_loss` |
| `price.rs` | `process_bbo`, `update_mark`, `stash_bbo`, `drain_stashed_bbos`, `rebuild_fallback` |
| `funding.rs` | `maybe_settle_funding` |
| `config.rs` | `process_config_applied`, `reload_symbol_overrides` |
| `persist.rs` | `push_persist`, `is_backpressured` |
| `account.rs` | `ensure_account` |
| `shard.rs` (keeps) | struct, `new`, `set_persist_producer`, `set_state`, `user_in_shard`, `tick` dispatcher |

No new `shard_*.rs` file is created. `shard.rs` ends as the state owner plus
the dispatcher, which is what its name claims.

### Binary side — same correction

`persist_loop.rs` is dropped: `persist.rs` already exists and owns that
domain, so `persist_thread_body` and `stop_persist_worker` move there.
Likewise the lease watch goes to the existing `lease.rs`. That leaves `app.rs`
(composition root, as the founder asked for), `promotion.rs` (the
`NodeState` machine), `cast_loop.rs` (the pinned hot loop), and
`me_receiver.rs` as the only genuinely new modules.

## Constraints

- **Pure move-and-split. No behavior change** in the same pass. Anything that
  looks like a bug during the move gets recorded in `BUGS.md`, not fixed —
  a refactor that also changes behavior can't be reviewed.
- Public API in `lib.rs` re-exports stays identical, so no other crate sees
  this.
- `cargo test -p rsx-risk` must show the same counts before and after
  (baseline: **292 passed, 0 failed, 35 ignored**).
- `make lint` green; default rustfmt only.
- One commit per coherent move, not one giant commit.

## Sequencing — do NOT start before these land

Two agents are working rsx-risk concurrently:

1. The integration agent is adding replica/failover/crash tests to
   `rsx-risk/tests/` and unblocking the 35 ignored container tests. Splitting
   `src/` under it would break its compiles mid-flight.
2. The refinement audit (`.ship/50-RISK-REFINEMENT-AUDIT/REPORT.md`) is
   read-only and reports on exactly this crate's minimization and
   orthogonality. Its findings should shape the boundaries above before they
   are cut — it may identify dead code that should be deleted rather than
   moved into a new file.

Start after both land, and fold the audit's findings into the table above
first.

## Open question for the founder

The audit may recommend deleting rather than relocating some of this
(`max_leverage` is stored, settable and logged but never read by any check).
Dead code should be removed in a separate commit *before* the split, so the
split doesn't preserve it into a new home.
