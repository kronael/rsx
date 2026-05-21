# Changelog

## [v0.2.0] — 2026-05-21

Workspace expands from 11 to 12 Rust crates; transport
(`rsx-dxs`) is now domain-agnostic; security and correctness
gaps surfaced by an a16z-style review are closed; 28 refine
commits sweep wisdom-rule violations and reconcile 12 specs
against the actual code.

### Architecture

- **`rsx-messages` extracted from `rsx-dxs`.** Domain wire
  records (Fill, BBO, Order*, Mark, Liquidation,
  ConfigApplied, CancelRequest) moved to a new crate.
  `rsx-dxs` is now a pure transport library with **zero
  `rsx-types` production dependency** — provable via
  `cargo tree -p rsx-dxs --edges normal`.
- **WAL header gains a `version: u8`** at byte 8.
  `V0 = 0` (legacy zero-reserved, accepted on read for
  back-compat). `V1 = 1` (current, written by all new
  senders). Receivers reject unknown versions at every
  ingress path (CmpReceiver::try_recv + recv_control,
  WalReader::next, DxsConsumer, DxsReplayService).
  Adding a new record type does NOT bump the version
  (additive); only header-layout / CRC-algorithm changes do,
  and require a coordinated stop-redeploy.
- **`send_ring` rewritten to preallocated `Box<[T]>` slabs.**
  3 parallel slabs (`ring_seqs: Box<[u64; 4096]>`,
  `ring_lens: Box<[u16; 4096]>`, `ring_frames:
  Box<[u8; 4096*128]>`) indexed by `seq & MASK`. Zero heap
  allocations on the CMP send path. Replaces the prior
  `BTreeMap<u64, Vec<u8>>` that allocated per send.
- **O(1) cancel index in matching.**
  `FxHashMap<OrderKey, slab_handle>` maintained from
  `book.events()`. Replaces the previous O(n) slab scan
  (n up to 65,536; the stale `cap=1024` comment was wrong).
- **Two-tier NAK retransmit shipped.** Hot tier (in-memory
  preallocated ring) → cold tier (WAL random-access via
  `read_record_at_seq`). Retransmit horizon = WAL retention,
  not buffer size.
- **`RecorderConfig` moved out of `rsx-dxs`.** Lives in
  `rsx-recorder/src/config.rs` (`pub(crate)`); transport
  no longer carries application-level config.

### Security & correctness

- **Silent fill loss closed.** Six `let _ = wal_writer.
  append(...)` sites in `rsx-matching/src/main.rs` replaced
  with `.expect("INVARIANT N: ...")` carrying specific
  invariant names from `specs/2/6-consistency.md`.
  Matching is the authoritative WAL writer; silent drop
  violated Invariant #1.
- **JWT hardening.** `JWT_SECRET_MIN_LEN = 32` enforced at
  gateway startup. `Validation::validate_nbf = true`. New
  `JtiTracker` (bounded FIFO replay set) shipped, but
  currently **dormant** — not yet wired through
  `ws_handshake`. Decision pending: per-process tracker vs
  shared Redis. TODO at `rsx-gateway/src/ws.rs:108`.
- **Gateway per-IP rate-limiter bounded.** `IP_LIMITER_MAX
  = 10_000` with FIFO eviction via a parallel
  `VecDeque<IpAddr>`. Closes a slow-burn memory-DoS via IP
  rotation.
- **DXS replay TCP server verifies version + CRC before
  unsafe cast.** Closes a parity gap with CMP/UDP ingress.
- **CMP source-IP filter — rejected on review.** A `src.ip
  == sender_addr.ip` filter was added then reverted. CMP is
  intentionally unauthenticated per spec §10.4; trust is
  delegated to the gateway (JWT) for external clients and
  to the L3 network (firewall, VPC, namespace) for internal
  RSX peers. New "Trust boundaries" section in `CLAUDE.md`
  prevents this misclassification next time.

### Wisdom rules (now enforced uniformly)

- **`let _ = call_returning_result()` → 0 violations.**
  28 sites across 8 crates fixed to propagate via `?`,
  log via `if let Err`, or fail loud with a named
  invariant.
- **Bare `.unwrap()` in production → 0.** 5 sites in
  non-test code replaced with `.expect("INVARIANT: ...")`.
- **Every `.expect()` annotated.** 12 sites gained
  `// SAFETY: fail-fast at startup` or named-invariant
  messages.
- **16 wire records × 2 compile-time asserts** (`size_of`
  + `align_of`). Wire-format size of every domain and
  protocol record is part of the build.
- **No `panic!` / `todo!` / `unimplemented!`** in
  production code. Confirmed by audit.

### Performance

- Match single fill: **54 ns** (Criterion, `rsx-book`)
- CMP encode / decode: **43 ns / 9 ns**
- WAL append (in-memory): **31 ns**
- WAL flush + fsync 64 KB: **24 µs**
- `make latency-publish` harness shipped — writes measured
  GW→ME→GW p50/p99 to `bench-baseline.json`. The <50µs
  end-to-end claim remains a **design budget** until the
  founder runs the harness against a live cluster.

### Specs

12 spec files reconciled against code:
- `4-cmp.md` (transport)
- `10-dxs.md` (TCP replay)
- `11-gateway.md` + `49-webproto.md`
- `15-mark.md`
- `16-marketdata.md`
- `17-matching.md`
- `18-messages.md` (full record inventory)
- `21-orderbook.md`
- `28-risk.md`
- `45-tiles.md` (9 line-number drift fixes)
- `47-validation-edge-cases.md`
- `48-wal.md`
- `6-consistency.md` (10 invariants cross-referenced;
  7 code-comment additions naming the invariant each
  enforces)

### Documentation

- New `CLAUDE.md` section: **"Trust boundaries"** — codifies
  when not to add code in a layer the spec delegates from.
- 19 docs refreshed pre-release (root + per-crate
  READMEs + per-crate ARCHITECTURE).
- New artifacts: `.ship/13-A16Z-FIXES/{PLAN,REPORT,WEDGE}.md`,
  `.ship/14-REFINE-PASSES/{CHECKLIST,REPORT}.md`.
- `blog/cmp.md` updated: two-tier NAK + version-byte
  schema-evolution section.

### Build & test

- Workspace: **12 crates, 878 Rust tests pass, 0 fail.**
- Clippy lib + bin: 13 warnings → 6 (deeper too-many-args
  refactors flagged for a future pass).
- Playwright canonical: **421 of 424** passing (3
  conditional skips).
- `make latency-publish` added; runs the F1 probe under
  `N=2000` (default; override via `N=`) and writes p50/p99
  to `bench-baseline.json`.

### Open work (carry to v0.3.0)

- **JtiTracker wiring through ws_handshake** — design call.
- **Replica → main promotion refactor** (`rsx-risk/src/main.
  rs:~1086`) — `std::env::set_var` + recursive `run_main`
  is UB-adjacent on glibc; deferred (`13-A16Z-FIXES T3.2`).
- **Hot-path `.to_vec()` in `rsx-risk/src/shard.rs:764-767,
  805-809`** — borrow-checker workaround vs `ExposureIndex`.
- **6 deeper clippy warnings** — too-many-args refactors
  (matching, maker, risk).
- **Measured E2E latency** in `bench-baseline.json` —
  founder runs `make latency-publish`.
- **BLOG.md narrative reframe** (`13-A16Z-FIXES T5.2`) —
  editorial; depends on wedge choice (`WEDGE.md` Option
  A/B/C).
- **2 hot-path `eprintln!` in `rsx-book`** — no `tracing`
  dep on the crate; cross-cutting decision.

## [v0.1.0] — earlier

Initial spec-first scaffold. Per-symbol matching engine, CMP
prototype, WAL-based recovery, JWT-authed WebSocket gateway,
risk engine with margin + funding + liquidation, marketdata
shadow book, mark price aggregator (Binance + Coinbase),
recorder + CLI + market-maker bot. ~21k LOC Rust across 11
crates, 871 tests passing.
