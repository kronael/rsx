# CLAUDE.md

Spec-first derivatives exchange with portfolio margin. `specs/2/` holds the
intent (`specs/2/index.md`); each crate's `ARCHITECTURE.md` holds how it is
built now. A spec that proves impractical is refined, never silently diverged
from. Crate-local rules: `<crate>/CLAUDE.md`.

## Architecture

- Processes: Gateway, Risk (sharded by `user_id`), Matching (one per symbol),
  Marketdata, Mark, Recorder. An order routes GW → Risk[U] → ME[S] → Risk[U]
  → GW. The two scale-out axes are independent: more symbols = more ME
  instances, more users = more Risk shards; a symbol's book is one market and
  lives in exactly one ME.
- Between processes: casting (C structs over UDP, the live path) and WAL
  replication (TCP, replay). WAL disk format = wire format = replication format.
- Targets: <50 µs GW→ME→GW, <500 ns match.

## Workspace

Rust workspace, 14 crates (`Cargo.toml` is the list): `rsx-types`, `rsx-cast`
(frozen transport, no rsx-types dep), `rsx-messages` (every wire record),
`rsx-ws` (domain-free monoio WebSocket core of the gateway and marketdata),
`rsx-book`, `rsx-matching`, `rsx-risk`, `rsx-gateway`, `rsx-marketdata`,
`rsx-mark`, `rsx-recorder`, `rsx-cli`, `rsx-log`, `rsx-health`. Each daemon is
a thin binary over its library crate. Libraries come before daemons in any
listing.

Outside the workspace: `rsx-glass/` (Go GPU trading UI, the trade
surface; its specs are `rsx-glass/specs/`), `rsx-core/` (Go model it shares),
`rsx-maker/` (Go market maker), `rsx-playground/`
(Python/FastAPI operator dashboard + Playwright gates), `rsx-auth/` (Python
identity and roles). The sibling `../trader` repo is the reference for monoio
WS/HTTP code and the shared macros.

## Hot path

- Zero heap, i64 fixed-point, no floats; `Price(i64)`/`Qty(i64)` newtypes;
  `#[repr(C, align(64))]` on wire and hot structs; slab arenas; FxHashMap for
  integer keys. Overflow is checked at order entry (`checked_mul` for notional
  at the risk boundary), never on the hot path.
- Risk and Matching are ONE pinned busy-spin loop each over the std
  non-blocking `CastReceiver`: no async runtime, no sleep, no poll, no mutex
  on the hot path, and a bare spin: NEVER a `spin_loop()` hint, it can stall
  the core. A single loop is called a loop, not a tile.
- Gateway and Marketdata are monoio (io_uring) reactors for many client fds.
  A reactor waits on readiness; it NEVER yield-spins or polls a ring.
- tokio only off the hot path (Postgres write-behind, lease, replication
  client, Mark, Recorder), in a named side-thread fed over an rtrb SPSC ring.
  ALWAYS spawn side-threads before pinning the hot thread, so they do not
  inherit its core. A full ring stalls the producer; per-consumer rings keep a
  slow consumer from stalling a fast one.
- Every daemon pins its core from `RSX_<X>_CORE_ID`; an unpinned busy spinner
  starves a hot core. Dev boxes may share cores: functional, not fast.
- Logging and latency tracing leave the hot path over `rsx-log`'s ring;
  latency sampling is the compile-time `latency-trace` feature, NEVER a
  runtime flag. A feature that costs a core (SQPOLL) is gated on the core
  config, not on an enable flag.
- NEVER a second code path for one thing: no no-WAL send, no convenience
  wrapper, no per-crate copy of a wire struct. One optimal way; the rest is
  deleted. Wire records live in `rsx-messages` only.

## Code shape

- `main.rs` is thin: `app.rs` boots, one module per loop or side-thread
  (`cast_loop.rs`, `persist.rs`, `promotion.rs`, ...), domain logic in its
  own module. NEVER a mono module.
- Flat modules, one level; re-export from `lib.rs`; `_utils.rs` holds
  stateless helpers only; one `use` per line.
- Names are words: `position`, `order`, `level`; wire names are `seq`,
  `ts_ns`, `px`, `qty`, `bid_px`; `symbol_id`/`user_id` are `u32`; `_pad`
  prefixes padding. NEVER `o`, `O`, `l`, `I` as identifiers.
- States are `repr(u8)` enums; NEVER a magic integer or flag bits for a state.
- Comments state current behaviour or a load-bearing invariant, short, only
  where non-obvious. NEVER history, bug ids, architecture prose (that is
  `ARCHITECTURE.md`) or `#[inline]` nudges; the compiler inlines.
- An `.expect` on the hot path names the invariant it protects (a lost fill
  is unrecoverable: panic). Boot-time I/O (bind, connect) retries; fail-fast
  there is not safety. NEVER drop a `Result`.
- Config: runtime wiring (addresses, cores, shard ids) from env, parsed once
  into typed values at boot; a malformed value fails the boot. Business
  config (symbols, tick/lot, margin, fees, funding) lives in Postgres;
  `rsx-risk` owns the schema and runs every migration
  (`rsx-risk/migrations/CLAUDE.md`); every other process is a reader. No TOML,
  no seeds outside migrations.
- Prefer deleting to generalising; many `if`s mean the shape is wrong.

## Frozen: rsx-cast

Bugfixes only (restoring documented behaviour); any other change needs the
founder's explicit sign-off BEFORE code. Work in another crate that seems to
need an rsx-cast change stops and asks; the fix is almost always in the
caller. Policy and the signed-off exceptions: `rsx-cast/CLAUDE.md`.

## Trust boundaries

One owner per concern; cite the spec instead of adding a second owner.
- Casting is unauthenticated by design (`specs/2/4-cast.md` §10.4, trusted
  network). No per-frame filters, MACs or signing in `CastReceiver`.
- Gateway owns auth (JWT) and wire parsing; Risk owns margin and position
  permission; Matching enforces only structural book invariants (positive,
  tick- and lot-aligned). NEVER re-run auth or margin on the hot path.
- TLS terminates in a proxy in front of the gateway; the gateway speaks plain
  WS. A JWT is reusable until expiry.
- An audit finding "X is unauthenticated" is closed by citing the spec that
  delegates X, not by code in the delegating layer.

## Publishing and licence

- NEVER publish externally: no crates.io, npm, PyPI, blog posts, tweets. The
  prohibition equals the `git push` one. GitHub pushes only when asked. A doc
  or plan that proposes publishing says "leave for the founder".
- GPL-3.0-only for project code and docs; third-party notices preserved;
  package metadata matches root `LICENSE` and `NOTICE`. No dual-licensing or
  "other licences on request" copy anywhere.

## Docs

- Layout and the one question per file: `readme` skill → `topology.md`. Per
  crate: `README.md` (what, how to run, quirks), `ARCHITECTURE.md` (how it is
  built now), `notes/*.md` (why this choice: Problem → Fix → Cost, cited, one
  file per decision), `compare/` (vs named alternatives), `CLAUDE.md` (editor
  rules + keeper sections). Exemplars: `rsx-cast/`, `rsx-book/`.
- What is now, never history. No marketing, no fluff, no parenthesised
  reformulations, no "jargon" or "rollout" headings. Uppercase file names
  only at a directory's top level.
- Component status words are README § Component status: Finished, Release
  candidate, Working, In development. `rsx-cast` and `rsx-book` are Finished
  and presented so.
- Numbers: every latency table carries p50 and p99 from one run (a standard
  error may accompany p50, never replace p99), measured on a quiet box with
  the cluster stopped; the dated run lives in `reports/YYYYMMDD_tag.md` (what,
  tables, conclusion, caveats, bench + commit) and the docs quote it.
  Criterion benches plus the perf gate are the ONLY latency verification;
  playtests are functional and the dashboard latency card is a demo aid.
- Demos live in `<crate>/demo/` and on the playground `/demos` page; a README
  never embeds a demo or explains how to reproduce one.
- `.diary/` (the repo diary) and `.ship/` (sprint dirs) are checked in on
  purpose: NEVER gitignore or prune them.

## Build, test, gates

- Toolchain: a dated nightly pinned in `rust-toolchain.toml` (cranelift debug
  via `.cargo/config.toml`, release on LLVM); `make toolchain-bump` moves the
  pin and runs the checks. NEVER the bare `nightly` channel.
- `cargo check` first. `make build` is debug, `make release` is release.
  Default `rustfmt`, no `rustfmt.toml`; `make lint` (`clippy --all-targets
  -D warnings`) and `make fmt-check` are green or the change is not done.
- `make test` = every Rust test that needs no Docker, Postgres or running
  cluster (`--lib` plus the non-ignored `tests/` binaries); NEVER a test that
  touches a live system. `make integration` = testcontainers (`--ignored`);
  `make e2e` = Rust + API + Playwright against a real cluster; then
  `make wal`, `make smoke`, `make perf`, `make perf-gate`,
  `make perf-e2e-gate`. The Makefile is dev-only and production is never one
  `make` away. NEVER add a target nobody calls.
- Unit tests in `src/<module>_test.rs`, `#[cfg(test)] mod <module>_test;` at
  the bottom of the module. Integration tests in `tests/` drive the real
  component over real sockets. A test named `known_defect_*` pins an open
  `BUGS.md` entry and fails until fixed; `make integration` skips the prefix.
- NEVER gate, skip or delete a failing test to get green.
- Scripts are Python under uv; docs are written by hand, never generated.
- Playground is the dev entrypoint: `./rsx-playground/playground start |
  start-all | stop-all | reset`. `make gate` runs the four ordered playground
  gates (imports, HTMX partials, API, browser); run it whole, never a stage
  alone; gate 4 needs a quiet box. `make ci` and `make ci-full` wrap it.

## Correctness invariants

1. Fills precede ORDER_DONE per order. 2. Exactly one completion per order
(ORDER_DONE xor ORDER_FAILED). 3. FIFO within a price level. 4. Position =
sum of fills. 5. Tips monotonic. 6. Best bid < best ask. 7. SPSC preserves
event FIFO. 8. Slab: allocated = free + active. 9. Funding zero-sum per symbol
per interval. 10. Advisory lock: at most one main per shard.
