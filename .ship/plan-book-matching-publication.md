# Book + matching publication plan

## Outcome

Prepare `rsx-book` and `rsx-matching` as honest, reproducible public artifacts.
This pass may commit a GitHub-ready source/demo release, but it must not push,
tag, create a release, or publish crates. Those final actions stay with the
founder.

There are two distinct finish lines:

1. **GitHub showcase:** the two crates are correct, documented, demoed, and
   signed off in this repository. This does not require publishing their whole
   internal dependency graph.
2. **crates.io:** `rsx-book` can be a library package; `rsx-matching` requires
   six internal packages to exist in the registry first. Treat that dependency
   publication as an explicit founder decision, not incidental metadata work.

Do not touch unrelated exchange components except the minimum package manifests
needed for a founder-approved crates.io path. Do not edit `rsx-cast` source.
Preserve the current unrelated changes in `TODO.md` and `rsx-term/demo/`.

## 1. Freeze the evidence baseline

**Files:** `BUGS.md`, `PROGRESS.md`, `reports/20260709_matching-benches.md`,
`rsx-book/`, `rsx-matching/`, `.diary/20260716.md`

- Re-run the focused debug gates before changing behavior and record exact
  command/results in today's diary.
- Confirm every open book/matching correctness item against current source;
  resolve only the two publication blockers below and leave unrelated findings
  in `BUGS.md`.
- Save the starting commit and host facts used by later benchmark/demo reports.

**Acceptance:** a short dated diary entry names the starting commit, commands,
and the exact two blocking bugs; no status is upgraded.

**Green gate:**

```sh
cargo check -p rsx-book -p rsx-matching
cargo test -p rsx-book -p rsx-matching
cargo clippy -p rsx-book -p rsx-matching --all-targets -- -D warnings
```

## 2. Preserve order metadata before slab reuse

**Files:** `rsx-book/src/event.rs`, `rsx-book/src/matching.rs`, relevant
`rsx-book/src/*_test.rs`, `rsx-matching/src/wal.rs`,
`rsx-matching/src/wal_test.rs`, `BUGS.md`

- Put `tif` and `reduce_only` directly on `Event::Fill` and
  `Event::OrderDone` while the maker slot is still live.
- Build `FillRecord`/`OrderDoneRecord` only from the captured event data; never
  dereference a completed order's handle after the match cycle.
- Add the exact regression: a GTC taker fully consumes a maker, the freed LIFO
  slot is reused by the taker's residual, and the maker wire records retain the
  maker's original TIF/reduce-only values.
- Cover multi-maker sweep and no-residual paths so the fix cannot reorder
  `Fill` before `OrderDone` or change FIFO semantics.
- Resolve `ME-FILL-DONE-STALE-SLOT-TIF` inline in `BUGS.md` only after the
  regression passes.

**Acceptance:** all terminal records contain the originating order's metadata
under immediate slot reuse; no post-cycle stale-handle reads remain in
`emit_events`.

**Green gate:**

```sh
cargo test -p rsx-book -p rsx-matching stale_slot
cargo test -p rsx-book -p rsx-matching
cargo clippy -p rsx-book -p rsx-matching --all-targets -- -D warnings
```

## 3. Make user-slot lifetime bounded

**Files:** `rsx-book/src/user.rs`, `rsx-book/src/user_test.rs`,
`rsx-book/src/book.rs`, `rsx-matching/src/main.rs`, matching lifecycle/recovery
tests under `rsx-matching/tests/`, `BUGS.md`

- Add one time-gated, off-hot-path registry-maintenance operation that marks
  newly idle users and reclaims only users with zero position, zero resting
  orders, and an expired grace period. It must be disabled during replay.
- Call it from the matching idle/maintenance path, not per order. Avoid an
  unbounded scan in the hot path and handle the `u16` ceiling without overflow.
- Prove slot reuse after the grace period, no reclaim while an order/position is
  live, no reclaim during replay, snapshot round-trip of registry state, and a
  long-run churn case beyond 65,535 distinct historical users.
- Resolve `BOOK-USER-RECLAIM-UNWIRED-U16-CAP` only after the production caller
  and long-run test exist.

**Acceptance:** historical user churn cannot panic at the `u16` bump limit and
recovery produces the same user registry as uninterrupted execution.

**Green gate:**

```sh
cargo test -p rsx-book user
cargo test -p rsx-matching lifecycle
cargo test -p rsx-book -p rsx-matching
cargo clippy -p rsx-book -p rsx-matching --all-targets -- -D warnings
```

## 4. Make the demos reproduce the truth

**Files:** `rsx-book/demo/bench-live.sh`, `rsx-book/demo/Makefile`,
`rsx-book/demo/CLAUDE.md`, `rsx-book/demo/book-live-opt.gif`,
`rsx-matching/demo/bench-live.sh`, `rsx-matching/demo/Makefile`,
`rsx-matching/demo/CLAUDE.md`, `rsx-matching/demo/match-live-opt.gif`,
`rsx-book/examples/book_smoke.rs`, `scripts/demo-trade.sh`, crate READMEs,
new dated reports in `reports/`

- Remove hard-coded `~30/266 ns` claims from the matching recording. Have each
  script run the real example/bench, fail on command failure, and render the
  measured result plus its single-box/closed-loop caveat.
- Keep two demonstrations: `book_smoke` for the copyable library API, and the
  existing full trade script for matching's place/rest/cross/fill/WAL path.
- Add deterministic preflight/cleanup and explicit assertions; do not make a
  pretty GIF from a failed or stale run.
- Regenerate both optimized GIFs from committed scripts. Record script, commit,
  host, and observed values in dated reports.

**Acceptance:** a clean checkout can reproduce each GIF with one documented
command; matching's live demo proves a fill persisted to WAL, and no demo text
claims numbers that the run did not produce.

**Green gate:**

```sh
cargo run -p rsx-book --example book_smoke
make -C rsx-book/demo rec gif
scripts/demo-trade.sh
make -C rsx-matching/demo rec gif
git diff --check
```

If the current host cannot run a stable live cluster, retain the existing GIF,
mark it with its capture date/commit, and leave regeneration as a named founder
gate; never fabricate a pass.

## 5. Rebaseline claims, not the hardware

**Files:** `rsx-book/README.md`, `rsx-book/ARCHITECTURE.md`,
`rsx-book/compare/README.md`, `rsx-matching/README.md`,
`rsx-matching/ARCHITECTURE.md`, `rsx-matching/notes/depth-independent.md`,
`rsx-matching/demo/*`, `PROGRESS.md`, new dated benchmark reports

- Re-run focused Criterion benches on the quietest available host and save the
  full method/results. Preserve the robust claim—latency stays flat with
  depth—and replace exact matching headlines with the current measured range
  unless a dedicated host reproduces tighter numbers.
- Keep microbench, throughput, and cross-process latency clearly separate.
  Exact nanoseconds always cite one dated report and commit.
- Bare-metal/dedicated-host certification is deferred, not silently waived;
  describe current numbers as shared-host observations, never a hardware floor.
- Reconcile every README, architecture page, note, demo caption, and
  `PROGRESS.md` row to the same source report.

**Acceptance:** `rg '30 ns|266 ns|3\.6M|60 ns'` finds only claims supported by
their cited report and caveat; the 2026-07-09 non-reproduction is not hidden.

**Green gate:**

```sh
cargo bench -p rsx-book --bench deep_book_bench
cargo bench -p rsx-matching --bench match_depth_bench --bench process_order_bench
rg -n '30 ns|266 ns|3\.6M|60 ns' rsx-book rsx-matching PROGRESS.md
git diff --check
```

## 6. Make `rsx-book` package-clean

**Files:** `rsx-book/Cargo.toml`, `rsx-types/Cargo.toml`, `Cargo.lock`, crate
READMEs, `LICENSE` if package inspection shows it missing

- Add exact compatible registry versions alongside local paths
  (`version = "=x.y.z", path = "../…"`) and complete only metadata Cargo and
  crates.io actually require.
- Founder chooses one minimal route:
  - publish `rsx-types` first, then `rsx-book`; or
  - keep both registry-private and stop at a GitHub showcase release.
- Inspect the generated `.crate`; ensure it contains the library, example,
  README, architecture/notes needed by package users, and no repository-only
  artifacts or secrets.
- Build and test the packaged tarball using registry resolution, not workspace
  path leakage.

**Acceptance:** `rsx-book` packages from a clean tree and its extracted package
builds/tests without the workspace. No registry action has occurred.

**Green gate:**

```sh
cargo package -p rsx-types --locked
cargo package -p rsx-book --locked
cargo package -p rsx-book --list
```

`publish = false` is removed from `rsx-types` only after the founder chooses
crates.io and confirms ownership/name availability.

## 7. Decide whether `rsx-matching` belongs on crates.io

**Files:** `rsx-matching/Cargo.toml`, `rsx-types/Cargo.toml`,
`rsx-log/Cargo.toml`, `rsx-book/Cargo.toml`, `rsx-cast/Cargo.toml`,
`rsx-messages/Cargo.toml`, `rsx-health/Cargo.toml`, `Cargo.lock`

- First make matching's package identity complete: description, repository,
  readme, keywords, categories, license, and explicit `version + path` for all
  internal dependencies.
- Present the real cost before changing publication flags: crates.io matching
  requires registry releases of `rsx-types`, `rsx-log`, `rsx-book`,
  `rsx-cast`, `rsx-messages`, and `rsx-health` in dependency order.
- Resolve the `rsx-cast` ↔ `rsx-messages` development-dependency cycle without
  touching frozen cast source: either keep matching GitHub-only, or obtain
  founder sign-off for the smallest manifest/bench packaging split that makes
  independent package verification possible.
- Check crate-name availability and registry ownership only as a read-only
  preflight. Do not reserve, upload, or publish names.

**Acceptance:** either (a) matching is explicitly GitHub-only with package
metadata no longer implying crates.io readiness, or (b) every prerequisite
package passes `cargo package --locked` independently and the founder has a
written publication order. No hidden `--no-verify` escape hatch.

**Green gate:**

```sh
cargo metadata --locked --no-deps
cargo package -p rsx-matching --locked
cargo package -p rsx-matching --list
```

Expected registry order if approved: leaf packages first, then `rsx-book` and
`rsx-messages`, then `rsx-matching`; derive the final sequence from
`cargo metadata` after resolving cast's dev-only cycle.

## 8. Sign off and hand publication to the founder

**Files:** `BUGS.md`, `PROGRESS.md`, `CHANGELOG.md`, crate READMEs,
`.diary/20260716.md`, a new `.ship/state-book-matching-publication.md`

- Run the repository's normal release gates plus both package checks and demos.
- Review generated package contents, public API docs, licenses, version numbers,
  benchmark citations, and git diff. Record every command and result.
- Upgrade `rsx-book` and `rsx-matching` to **signed off** only when their open
  correctness items are resolved and all applicable gates above are green.
  This component sign-off does not make the whole exchange v1.
- Prepare exact founder commands for the chosen destination, checksums of the
  `.crate` artifacts, expected package order, and a rollback/yank note. Do not
  execute publish, push, tag, or release commands.

**Acceptance:** the handoff says exactly what is green, what is deferred
(especially dedicated-host benchmarking), which destination is approved, and
which founder-only commands remain. The working tree contains no accidental
changes to `TODO.md`, `rsx-term/demo/`, or frozen `rsx-cast` source.

**Final green gate:**

```sh
make fmt-check
make lint
make test
make gate
cargo package -p rsx-book --locked
# only when the dependency-publication path was explicitly approved:
cargo package -p rsx-matching --locked
git diff --check
git status --short
```

External publication remains founder-run even after every gate is green.
