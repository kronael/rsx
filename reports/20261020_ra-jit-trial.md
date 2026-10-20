# ra-jit trial on the RSX workspace — 2026-10-20

[ra-jit](https://github.com/indexable-inc/ra-jit) is a daemon that keeps a Rust
workspace compiled in memory and rebuilds only the changed functions. This
report records how it was tried on RSX, what it measured, and why it is not
in use.

**Verdict:** not usable on this host today. The tool works, but the daemon
needs more RAM than the dev box has (3.9 GiB total, 2 cores). It also needs
Rust 1.99.0 stable, and RSX pins nightly for the cranelift debug backend.

## Setup used

ra-jit `nightly` release, commit 4513540, Linux x86-64 binaries
(`ra-jit`, `cargo-ra-jit`, `ra-jit-rustc`) copied into `~/.local/bin` after a
`SHA256SUMS` check. Rust 1.99.0 with `rust-src`:

```sh
rustup toolchain install 1.99.0 -c rust-src --profile minimal
```

ra-jit needs exactly 1.99.0. RSX pins `nightly-2026-01-22` in
`rust-toolchain.toml`, and `.cargo/config.toml` sets
`codegen-backend = "cranelift"`. Stable cargo rejects that key:

```
error: config profile `dev` is not valid
  feature `codegen-backend` is required
```

So the trial ran in a detached worktree **outside** the repo, with a
`rustup override set 1.99.0` and the cranelift lines removed from its
`.cargo/config.toml`. A worktree inside the repo does not work: cargo also
reads the main tree's `.cargo/config.toml` from the parent directories.
RSX has no `#![feature]` gates, so it builds on 1.99.0 with LLVM.

## Numbers

Same worktree, debug profile, mold linker, 2 cores. Each edit was timed after
a warm build. "leaf" toggles one comparison in
`rsx-risk/src/margin.rs::needs_liquidation`. "wide" toggles the panic text in
`rsx-book/src/book.rs::emit` (rsx-book is a dependency of several crates).

| Build | leaf edit (3 runs) | wide edit (3 runs) |
|---|---|---|
| `cargo build --workspace`, 1.99.0 LLVM | 10.1 s, 6.7 s, 4.7 s | 9.7 s, 9.4 s, 8.2 s |
| `cargo ra-jit build --workspace` | no result — worker killed | no result |

Cold builds: plain cargo 4 m 05 s (all dependencies); `cargo ra-jit build`
55 s directly after it (dependencies were already in `target/`, so this is
not a fair cold comparison).

The second `cargo ra-jit build` failed:

```
error: the daemon's worker ended (signal: 9 (SIGKILL): change: 0 files)
```

`dmesg` shows the kernel OOM killer stopped the worker at 2.4 GiB resident
(`Killed process ... (ra-jit) anon-rss:2474036kB`). The worker loads the whole
workspace through rust-analyzer and keeps it in memory.

## Caveats

- Three runs per edit, one host. Too few runs for p50/p99; these are build
  times, not exchange latency.
- The plain-cargo row is LLVM on stable. The real dev loop uses nightly
  cranelift, which this trial did not time.
- ra-jit has no borrow checker; `cargo check` still has to run.

## What would make it usable

1. A host with enough RAM for the worker (more than ~3 GiB free beside the
   normal build).
2. A stable-toolchain build profile for RSX: drop cranelift or keep it as an
   opt-in nightly profile. That is a project decision, not a trial edit.

Rerun the trial with the same steps after either change.
