# Changelog

## [rsx-dxs v0.4.0] — 2026-05-24

Replay-endpoint federation (breaking) bundled with the
v4-reliability follow-ups (config trim, spec gap, matching
recovery POC). Wire format unchanged.

### Breaking change

- `DxsConsumer::new(stream_id, producer_addr: String, ...)` →
  `DxsConsumer::new(stream_id, endpoints: Vec<String>, ...)`.
  Endpoints are tried in order; on `ReplayNotAvailable` (see
  below) or connect failure the consumer advances to the next.
- Migration: pass `vec![addr]`, or use the convenience
  constructor `DxsConsumer::from_single(stream_id, addr, ...)`
  which is wire-equivalent to the old API.

### Protocol additions

- **`RECORD_REPLAY_NOT_AVAILABLE = 0x15`** — 64-byte transport
  record. Server emits it when the requested `from_seq` is
  below the oldest record it can serve on disk; consumer
  treats the response as "wrong endpoint" and moves on. Carries
  `requested_from_seq` / `my_oldest_seq` / `my_highest_seq` so
  the consumer can log the gap.
- Server pre-checks `from_seq >= my_oldest_seq` in
  `DxsReplayService::handle_client` before opening the
  `WalReader`. Closes the federation correctness bug where
  an under-range request silently transitioned to live-tail at
  the wrong sequence.

### Federation pattern

```
DxsConsumer ──► tries endpoint list in order:
                 1. live producer (recent ~48h hot WAL)
                 2. recent archive (e.g. 30 days)
                 3. cold archive (indefinite)
                Whichever holds `from_seq` answers.
```

Each archive is a `DxsReplayService` populated by being a
`DxsConsumer` of the upstream — `rsx-recorder` already
implements this pattern; tiered archives compose from it.

### v4 follow-ups (folded in)

- **`CmpConfig::reorder_buf_limit` removed.** The receiver's
  reorder buffer became a fixed 2048-slot compile-time ring
  in v0.3.0 (commit `c89d164`); the config field stayed for
  `from_env` back-compat but wasn't read anywhere. Dropped
  the field, the `RSX_CMP_REORDER_BUF_LIMIT` env-var read,
  and the corresponding row in the parameter table. No
  migration needed — the field was already inert.
- **`reset_after_replay` monotonicity invariant documented.**
  The code already guarded against lowering `highest_seen`
  (the `if self.highest_seen < new_tip + 1` block); the
  invariant is now spelled out in the rustdoc and in both
  spec copies (`specs/2/4-cmp.md` + `rsx-dxs/specs/4-cmp.md`,
  "Reset semantics" subsection). Lowering `highest_seen`
  would re-arm the gap detector against seqs the consumer
  already applied via replay and could cause silent
  re-delivery — a FIFO violation. `expected_seq` still
  always jumps to `new_tip + 1`; that's the live resume
  point.
- **`rsx-matching` recovers from `CmpRecv::Faulted` via DXS
  replay (POC).** The matching tile no longer panics on
  FAULTED. It opens a `DxsConsumer` against the risk
  producer (env: `RSX_ME_REPLAY_DXS_ADDR`), drains Phase 1
  until `RECORD_CAUGHT_UP`, applies each `OrderRequest` /
  `CancelRequest` through the same book + dedup + WAL path
  the live tail uses, then calls `reset_after_replay`.
  Helpers live in `rsx_matching::replay` and are exercised
  end-to-end by `rsx-matching/tests/replay_after_fault_test.rs`
  against a real `DxsReplayService` over loopback TCP.

  Other consumers (`rsx-risk`, `rsx-marketdata`,
  `rsx-gateway`) still **panic** on `Faulted`, but with a
  unified message pointing at `rsx-matching` as the
  reference implementation:

  ```
  FAULTED: DXS replay path not yet wired here;
  see rsx-matching for the POC reference impl
  ```

  Per-consumer wiring is tracked as future work.

### Spec

- `specs/2/10-dxs.md` and `rsx-dxs/specs/10-dxs.md` document
  the multi-endpoint contract and `ReplayNotAvailable`.
- `specs/2/4-cmp.md` adds the "Reset semantics" subsection
  under "Three-tier delivery contract".
- `rsx-dxs/specs/4-cmp.md` standalone copy brought up to v4:
  reorder ring, FAULTED, three-tier delivery, reset
  semantics, refreshed config table.

## [rsx-dxs v0.3.0] — 2026-05-24

CMP reliability v4 — three real bugs in `rsx-dxs/src/cmp.rs`
fixed in one consolidated change. Wire format unchanged
(no `WalHeader.version` bump). See
`.ship/26-CMP-RELIABILITY-V4/SPEC.md`.

### Bugs fixed

- **Silent reorder-buffer overflow (FIFO violation).** The
  receiver's bounded `BTreeMap` reorder buffer silently
  advanced `expected_seq` past the gap on overflow (512
  default), violating the per-stream FIFO invariant. Replaced
  with a fixed 2048-slot ring; slot conflict (gap >
  capacity) now triggers sticky `FAULTED` and surfaces
  `CmpRecv::Faulted` to the consumer. No silent skip path.
- **NAK storm under sustained loss.** Every out-of-order
  packet arrival fired a fresh NAK over the whole gap; the
  heartbeat handler re-fired unconditionally every 10 ms.
  O(N^2) retransmit traffic. Replaced with `maybe_nak()`
  rate-limited per `nak_retry_us` (default 100 us) emitting
  only the oldest contiguous missing run. After
  `max_nak_retries = 8` retries without progress, the
  receiver escalates to `FAULTED`.
- **No sender-side retransmit dedup.** Duplicate NAKs
  caused the same seq to be retransmitted N times. Added
  per-slot `ring_last_retx_ns` parallel to `ring_seqs`;
  `handle_nak` skips slots retransmitted within
  `retx_dedup_window_us` (default 1 ms).

### Receiver API change

`CmpReceiver::try_recv` returns the new `CmpRecv` enum:

```rust
pub enum CmpRecv {
    Empty,
    Data(WalHeader, Vec<u8>),
    Faulted { last_delivered_seq, gap_start, gap_end_inclusive },
}
```

`Faulted` is sticky until `reset_after_replay(new_tip)` is
called. Consumers handle `Faulted` by replaying through
`DxsConsumer` (TCP/WAL), then resuming in-band delivery.

### Tests

Eight new tests in `rsx-dxs/tests/cmp_v4_test.rs` cover
the contract (single-packet recovery, multi-gap NAK order,
slot conflict -> FAULTED, sticky FAULTED, reset-after-
replay, sender dedup window, heartbeat-driven gap
detection, progress resets retry counter).

### Config additions

| Env var                          | Default | Meaning                                             |
|----------------------------------|---------|-----------------------------------------------------|
| `RSX_CMP_NAK_RETRY_US`           | 100     | receiver NAK debounce interval (oldest gap)         |
| `RSX_CMP_MAX_NAK_RETRIES`        | 8       | retries on oldest gap before FAULTED                |
| `RSX_CMP_RETX_DEDUP_WINDOW_US`   | 1000    | sender per-seq retransmit dedup window              |

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
- Protocol-record encode / decode (StatusMessage / Nak /
  Heartbeat): **43 ns / 9 ns**; `FillRecord` encode: **23 ns**
- `WalWriter::append` (Vec extend, no disk I/O): **31 ns**
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
