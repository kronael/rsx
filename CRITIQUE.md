# Critique (Factual, Tested)

This critique only lists real, verified problems. I ran `cargo test` and
`make test`, and cross-checked the claims in `README.md`, `CLAUDE.md`, and
`PROGRESS.md` against the actual repo state.

## Test Results

- `cargo test` passes, but it emits warnings in `rsx-dxs` about unused fields
  (`WalReader.stream_id`, `WalReader.wal_dir`). This contradicts the
  "refined (zero warnings)" claim in `PROGRESS.md`.
- `make test` fails because there is no `Makefile` and no `test` target.

## README.md: Incorrect Or Stale Claims

- **"No implementation code yet."** False. There are real crates with
  code and tests: `rsx-book`, `rsx-types`, `rsx-dxs`, `rsx-recorder`,
  and `rsx-matching`.
- **Crate layout path.** README lists a `crates/` directory, but the crates
  live at the repo root (`rsx-book/`, `rsx-dxs/`, etc). The documented layout
  does not match the actual layout.
- **Missing crates in layout.** README lists `rsx-risk`, `rsx-mark`,
  `rsx-gateway`, and `rsx-marketdata`, but those directories do not exist.
- **Build/test commands.** README lists `make test`, `make e2e`, etc, but
  there is no `Makefile`, so those commands are not runnable.
- **WAL recovery and SPSC performance claims.** README asserts properties like
  "0ms fill loss" and "50-170ns latency" as design principles, but there is
  no end-to-end system or benchmark in the repo to validate those claims yet.
  That makes the statements aspirational, not verified.

## CLAUDE.md: Incorrect Or Stale Claims

- **"No implementation code yet."** Same issue as README: there is already
  code and tests in multiple crates.
- **Crate layout path mismatch.** It documents `crates/` but the repo uses
  top-level crate directories instead.

## PROGRESS.md: Incorrect Or Stale Claims

- **Commit count.** `PROGRESS.md` says 35 commits; `git rev-list --count HEAD`
  reports 38.
- **Test count.** `PROGRESS.md` claims 75 tests passing. Current `cargo test`
  runs 114 tests across `rsx-book`, `rsx-dxs`, and `rsx-types`.
- **Zero warnings.** Current build emits warnings (see Test Results).
- **"Three crates shipped."** The workspace includes `rsx-dxs` and
  `rsx-recorder` with active code/tests, so the summary is incomplete.

## Bottom Line

The main problems are documentation drift and verifiable claim mismatches:
README/CLAUDE/PROGRESS describe a different state than what is currently in
the repo. The fixes are straightforward: update the docs to the actual crate
layout, existing implementations, current test counts, and real build entry
points.
