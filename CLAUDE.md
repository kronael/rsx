# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# RSX Exchange

Spec-first perpetuals exchange. All specs in `specs/2/`.

## Architecture (see specs/2/20-network.md, TILES.md)

- Separate processes: Gateway, Risk (per user shard),
  ME (per symbol), Marketdata, Recorder, Mark
- Scale-out axes: Risk shards by user_id (each shard owns a
  range of users, holds their positions + margin in RAM);
  ME shards by symbol (one instance per tradeable instrument).
  An order from user U on symbol S routes GW → Risk[U] → ME[S]
  → Risk[U] → GW. Adding symbols = add ME instances; adding
  users = add Risk shards. The two axes are independent.
- Between processes: CMP (C structs over UDP) + WAL
  replication (TCP)
  - Live path: CMP/UDP (order flow, fills)
  - Cold path: WAL replication over TCP (replay, replication)
- Within each process: tile architecture (pinned threads
  + SPSC rings for intra-process IPC, see TILES.md)
- Hot path I/O: monoio (io_uring) on Gateway, Risk, and Marketdata
  (all on the GW→ME→GW critical path). Mark, Recorder run on tokio
  (off critical path; blocking PG write-behind, ergonomics fine).
  Risk tile: monoio CMP/UDP hot loop + tokio task for PG write-behind;
  handoff via SPSC ring between the two (same tile pattern as Gateway).
- Later: DPDK/AF_XDP swaps I/O layer, same interfaces
- Target: <50us GW→ME→GW, <500ns ME match
- Zero heap on hot path, i64 fixed-point, no floats

## Crate Layout

Rust workspace (12 crates, see Cargo.toml):

```
rsx-types/      Price, Qty, Side, SymbolConfig, shared newtypes
rsx-dxs/        Domain-agnostic transport: CMP/UDP + WAL +
                DXS/TCP replay (no rsx-types dep)
rsx-messages/   RSX exchange wire records (Fill/BBO/Order*/...)
                on top of rsx-dxs
rsx-book/       shared orderbook (PriceLevel, OrderSlot, Slab,
                CompressionMap)
rsx-matching/   ME tile logic (one instance per symbol)
rsx-risk/       Risk tile logic (one per user shard)
rsx-gateway/    Gateway tile, WS ingress + CMP/UDP to risk
rsx-marketdata/ Marketdata tile, shadow book, L2/BBO/trades
rsx-mark/       Mark price aggregator (separate process)
rsx-recorder/   Archival DXS consumer (separate process)
rsx-cli/        WAL dump/inspect tool (clap CLI)
rsx-log/        Off-hot-path logging primitive (per-thread
                SPSC ring → drain thread → tracing events)
```

Each process is a separate binary. Crates are libraries
linked into their respective process binaries.

Non-Rust subprojects (NOT in cargo workspace):

```
rsx-playground/ Python/FastAPI dev dashboard + Playwright tests
                (uv-managed; pyproject.toml; tests/api_*.py and play_*.spec.ts)
rsx-webui/      Vite + React + Tailwind Trade UI SPA (bun)
rsx-auth/       Python auth service (uv; sqlx migrations)
```

## Implementation Philosophy

- Minimal implementation -- do the simplest thing that works
- Simple names, not abbreviated: `position` not `pos`, but not
  `user_position_state_container` either
- Do things simply, not intertwinedly -- each module does one thing
- Use traits where applicable for testability (mock boundaries)
- Flat file hierarchies: one level of modules, avoid deep nesting
- Copy standard macros from `../trader` where applicable
- Tracing + structured logs, NOT Prometheus -- dump metrics as
  structured log lines, a separate reader ships them elsewhere

## Patterns from funding-bot/trader

- Crate-per-concern, flat modules (no nested mod dirs)
- Re-export key types from lib.rs
- Tests in `tests/` dir with `_test.rs` suffix, not inline
- `_utils.rs` for stateless helpers only

## Rust Patterns

- Single import per line (`use tracing::info;` not
  `use tracing::{info, debug};`) -- cleaner git diffs
- `#[repr(C, align(64))]` on all hot-path structs (cache line)
- Fixed-point i64 for all prices/quantities -- NEVER float
- `Price(pub i64)`, `Qty(pub i64)` newtypes (`#[repr(transparent)]`)
- Slab arena allocator for fixed-size objects (orders, levels)
- Zero heap allocation on hot path (pre-allocate everything)
- Explicit enum states, not implicit flags
- FxHashMap for integer-keyed maps (not std HashMap)
- SPSC rings via rtrb for intra-process IPC
- Pin hot threads to cores via core_affinity
- Panic handler: `install_panic_handler()` from rsx_types
- Document lock acquisition order where locks exist

## Trust boundaries (read this before adding "security")

When the spec explicitly delegates a concern to a different
layer, do NOT add code in the layer that's being delegated
*from*. Cite the spec; trust the boundary. Concretely:

- **CMP is intentionally unauthenticated.** specs/2/4-cast.md
  §10.4 states "Trusted internal network. No authentication,
  no encryption." Auth lives at the gateway (JWT, TLS) for
  external clients and at the L3 network (firewall, VPC,
  namespace) for internal RSX peers. Do not add per-frame
  source-IP filters, MACs, or signing to CmpReceiver. If
  cross-DC peer auth is ever genuinely needed, do it as a
  sealed-frame extension under a future `WalHeader.version`,
  not by retrofitting the current zero-copy path.
- **The matching engine doesn't validate user input.** The
  gateway and risk tile validate before the order ever
  reaches ME. ME assumes its inputs are well-formed. Don't
  add re-validation on the hot path.
- **Audit-style "X is unauthenticated" findings are not
  automatically actionable.** Read the spec first. If the
  spec already documented X as out-of-scope-by-design (with
  a named layer that owns it), the finding is closed by
  citing the spec — not by writing code in the wrong layer.

The general rule: every concern has one owner. Adding a
second owner adds complexity, contradicts the spec, and
gives false confidence. Don't do it.

## Publishing

- **Do NOT publish anything externally.** No crates.io, no
  npm, no PyPI, no blog posts, no tweets, no public talks, no
  pushing to public registries. Even if a WEDGE.md, BLOG.md,
  or "what's next" section mentions publishing, treat that as
  internal-only narrative.
- **GitHub IS okay.** Pushing to the project's GitHub
  repository is allowed when explicitly requested. (Global
  rule still applies: never `git push` without an explicit
  ask.)
- If a doc, plan, or commit message proposes publishing
  externally, edit it to say "leave for the founder" or
  remove the line. Don't act on it.

## Documentation
- NEVER use "rollout" as a heading or section name
- `notes/` inside a crate or component is the canonical place for
  **why** documentation: design rationale, tradeoff research,
  derivations, prior art, measurements that justify a choice.
  Not "how it is" (that's ARCHITECTURE.md) — "why is it like that".
  Examples: `rsx-book/notes/slab.md`, `rsx-risk/notes/spsc.md`.
  `rsx-dxs/compare/` follows the same principle but is named after
  its theme (protocol comparisons); new research notes go in `notes/`.

## Naming

- `seq` not `seq_no`, `ts_ns` not `timestamp_nanoseconds`
- `px` for price, `qty` for quantity in wire/WAL contexts
- `bid_px`, `ask_px`, `bid_qty`, `ask_qty` for BBO fields
- `symbol_id: u32`, `user_id: u32` -- always u32, never string
- `_utils.rs` suffix for utility modules
- `_pad` prefix for padding fields in repr(C) structs

## Build & Dev

- `cargo check` first, always (fastest feedback, no codegen)
- Single test: `cargo test -p rsx-book -- test_name`
- Single test file: `cargo test -p rsx-dxs --test wal_test`
- Debug builds default (~3x faster compile than release)
- 80 char line width, max 120
- `make test`: Rust unit tests (`--lib` only) <5s, every commit
- `make e2e`: Rust + API + Playwright (~3min), every PR
- `make integration`: testcontainers, 1-5min (uses `--ignored`)
- `make wal`: WAL correctness <10s (cargo test -p rsx-dxs)
- `make smoke`: deployed system <1min
- `make perf`: Criterion benchmarks, nightly
- `make lint`: clippy with `-D warnings`
- Config via env vars only (no TOML args)
- Entrypoint always called `main`

## Playground (Primary Dev Entrypoint)

```bash
./rsx-playground/playground start     # FastAPI dashboard on :49171
./rsx-playground/playground start-all # build + launch all RSX processes
./rsx-playground/playground stop-all  # stop processes
./rsx-playground/playground reset     # stop + clean state
```

Web dashboard lets you submit orders, inspect WAL, control
processes, inject faults. See `rsx-playground/README.md`.

## Release Gates (acceptance pipeline)

The repo enforces a 4-gate pipeline; never run `gate-4` directly.

```bash
make gate              # all 4 gates in order
make ci                # check-progress + gates 1-3 + infra-smoke (no fan-out)
make ci-full           # ci + shards-gated (fan-out after 3 green infra-smokes)
make status-doctor     # required before any PROGRESS.md update
make release-gate      # blocks unless Playwright == 421/421 + all green
```

Gate ordering: `gate-1-startup` (server imports) → `gate-2-partials`
(HTMX 200s) → `gate-3-api` (Python API tests) → `gate-4-playwright`
(full Playwright suite, JSON+JUnit artifacts).

## Testing

- Tests in dedicated files, separate from code:
  `src/margin.rs` -> `tests/margin_test.rs` (not inline #[cfg(test)])
- E2E tests: real component + mocked deps, `tests/` dir
- Integration: testcontainers-rs (Postgres), `tests/` dir
- `--test-threads=1` if global state via DashMap/RwLock
- Centralize test setup in `tests/common/mod.rs`
- Testcontainers: dynamic port via `.get_host_port_ipv4()`
- Criterion for benchmarks with regression detection (>10% = fail)
- Property tests: proptest for order sequence invariants (future)

## Fixed-Point Arithmetic

All values are i64 in smallest units. Conversion at API boundary only.

```
price_raw = (human_price / tick_size) as i64
qty_raw   = (human_qty / lot_size) as i64
```

Overflow: check at order entry, not on hot path. Use checked_mul
for notional = price * qty at risk boundary.

## WAL / DXS

- Fixed-record format: 16B header + `#[repr(C, align(64))]` payload
- WAL disk format = wire format = DXS stream format (no transformation)
- WalWriter flush every 10ms, rotate at 64MB, retain 4h
  (hot tier only; ARCHIVE handles long-term durability)
- Backpressure: buffer full or flush lag > 10ms -> stall producer
- Tip persistence: every 10ms, idempotent replay from tip+1

## Networking Stack

- **Gateway, Risk, Market Data:** monoio with io_uring. All three
  are on the critical path (<50us end-to-end GW→ME→GW). Every
  epoll syscall adds latency. io_uring batches submissions
  in shared kernel/userspace rings -- fewer syscalls, lower
  tail latency. Each tile is a dedicated pinned thread with
  one SPSC downqueue (orders in) and one SPSC upqueue
  (fills out). The I/O multiplexing inside the tile is the
  only part that touches the network stack.
- **Risk tile specifically:** monoio CMP/UDP loop handles the
  hot path (order recv → margin check → forward to ME). A
  separate tokio task owns PG write-behind; SPSC ring hands
  off position updates from monoio → tokio without blocking
  the hot loop. Same handoff pattern as Gateway's WS→CMP split.
- **Later:** userspace networking (DPDK, AF_XDP) swaps the
  I/O layer inside the same tile. No changes to SPSC rings
  or ME.
- **DXS:** CMP/UDP for hot path, WAL replication over TCP
  for cold path. Same wire format as disk. See CMP.md.
- Reference impl: sibling `trader/monoio-client/` (set
  `RSX_TRADER_REF_DIR` to the absolute path locally)
  - `ws_monoio.rs`: WebSocket client/server on monoio
  - `web_client.rs`: HTTP client with monoio
  - Proven in production (funding-bot, trader)

## SPSC Ring Patterns

- rtrb for same-process IPC (~50-170ns latency)
- `push_spin()`: bare busy-spin, no `spin_loop()`, dedicated core
- Per-consumer rings (slow mktdata doesn't stall risk)
- Ring full = producer stalls (matching engine waits)

## Component Spec Cross-References

| Component | Spec | Test Spec |
|-----------|------|-----------|
| Architecture | specs/2/45-tiles.md | - |
| Shared orderbook | specs/2/21-orderbook.md | specs/2/34-testing-book.md |
| Matching engine | specs/2/21-orderbook.md, specs/2/6-consistency.md | specs/2/41-testing-matching.md |
| DXS (WAL + replay) | specs/2/10-replication.md, specs/2/48-wal.md, specs/2/4-cast.md | specs/2/36-testing-replication.md |
| Risk engine | specs/2/28-risk.md | specs/2/42-testing-risk.md |
| Liquidator | specs/2/13-liquidator.md | specs/2/38-testing-liquidator.md |
| Mark price | specs/2/15-mark.md | specs/2/39-testing-mark.md |
| Gateway | specs/2/20-network.md, specs/2/49-webproto.md, specs/2/29-rpc.md, specs/2/18-messages.md | specs/2/37-testing-gateway.md |
| Market data | specs/2/16-marketdata.md | specs/2/40-testing-marketdata.md |
| SPSC rings | notes/SMRB.md | specs/2/43-testing-smrb.md |
| Validation edge cases | specs/2/47-validation-edge-cases.md | (cross-references all) |

## Root Navigation

| File | Purpose |
|------|---------|
| README.md | Quick start, architecture diagram, deployment |
| PROGRESS.md | Per-crate status, regenerated from artifacts |
| GUARANTEES.md | Consistency, durability, ordering guarantees |
| CRASH-SCENARIOS.md | Failure mode catalog |
| RECOVERY-RUNBOOK.md | Ops recovery procedures |
| TESTING.md | Test taxonomy and how to run each tier |
| MONITORING.md | Structured-log metrics shipping |
| FEATURES.md | Feature inventory |
| BLOG.md | Publishable narrative (not internal docs) |
| TODO.md | Open work |

`.diary/` is the long-lived shipping log (date-named YYYYMMDD.md).

**RSX exception: `.ship/` is checked in.** The global `/ship`
skill defaults to gitignored + prune-on-close, but RSX keeps
sprint dirs in git as a build log — the audit reports,
benchmark sprints, and meta-reviews are useful "how this got
built" artifacts for reviewers. Do NOT add `.ship/` to
`.gitignore`. Do NOT `git rm -rf .ship/NN-NAME/` on close-out.
Distillation to durable homes (diary / specs / CHANGELOG)
still happens, but the source dir stays.

## Correctness Invariants (system-wide)

1. Fills precede ORDER_DONE (per order)
2. Exactly-one completion per order (ORDER_DONE xor ORDER_FAILED)
3. FIFO within price level (time priority)
4. Position = sum of fills (risk engine)
5. Tips monotonic, never decrease
6. Best bid < best ask (no crossed book)
7. SPSC preserves event FIFO order
8. Slab no-leak: allocated = free + active
9. Funding zero-sum across all users per symbol per interval
10. Advisory lock exclusive: at most one main per shard
