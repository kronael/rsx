# Changelog

## [Unreleased]

## [v0.7.0] — 20260707

> RSX v0.7.0 — TLS-everywhere replication, a measured recovery envelope, and the cast pitch
>
> The transport now proves its reliability claims with numbers, the cold path is encrypted by default, and rsx-cast has a demo you can post.
>
> • Replication mandates TLS — the plaintext TCP catch-up path is gone (casting/UDP stays plaintext by design, trusted LAN).
> • Measured recovery envelope — casting delivers reliably through 30% sustained packet loss (5/5, 8-retry budget); a 52k-record outage replays over TCP in ~1s.
> • The cast demo — a two-act terminal pitch GIF (flowing narrative, 10s packet-count race, all four numbers measured same-day); book/matching/risk demos aligned on the same palette.
> • QA gate enforced — workspace clippy `-D warnings` green, default rustfmt (no config), `make fmt`/`fmt-check` wired.

### Changed (BREAKING)

- **`CastSender::send<T>` removed.** One framing entry point:
  `WalWriter::prepare` → `append_framed` + `send_framed` for paired
  persist-and-publish; WAL-free publishers use `Framed::pack` +
  `send_framed`; `send_raw` remains the manual-seq escape hatch.
  Callers own seq monotonicity — a counter must survive the caller's
  own loop/phase boundaries (see the RTT-bench fix below for the
  failure mode).
- **Replication (TCP catch-up) now mandates TLS.** rustls +
  aws-lc-rs. `ReplicationService::new` and
  `ReplicationConsumer::new` take a `TlsConfig` (was
  `Option<TlsConfig>`) and require a cert (server) / CA (client);
  the plaintext code paths are gone. `TlsConfig::from_env` returns
  `io::Result` (was `Option`) and no longer gates on
  `RSX_REPL_TLS` — that env var is removed. It reads
  `RSX_REPL_CERT_PATH` / `RSX_REPL_KEY_PATH` / `RSX_REPL_CA_PATH`
  (defaulting to `./certs/{cert,key,ca}.pem`) and errors when any
  file is missing. The casting/UDP order-flow path stays plaintext
  by design (trusted LAN, spec 4-cast §10.4).
- **Migration.** Run `scripts/gen-snakeoil-certs.sh` to write
  self-signed dev certs into `./certs/`, or point the env vars at
  real certs. Deploy env templates and the playground now provision
  these automatically.

### Added

- **Measured recovery envelope** (`loss_degradation`,
  `outage_recovery` benches): reliable delivery through 30%
  sustained loss (collapse at 40%); 52k-record gap replays over
  TCP in ~1s. `reports/20260707_cast-loss-recovery.md`.
- **Per-crate demo GIFs** on the shared "Cemani" palette; the
  rsx-cast two-act pitch (`rsx-cast/demo/`) with a 10s
  packet-count race — all four comparison numbers measured
  2026-07-07.
- **Durability model documented** (rsx-cast README, GUARANTEES.md):
  replicas + continuously-spinning sync, not per-record fsync.

### Changed

- **QA gate enforced**: workspace `clippy -D warnings` green,
  default rustfmt (no rustfmt.toml), `make fmt`/`fmt-check`,
  `make lint --all-targets`.
- Replication stream-end signal is a named `StreamEnd { Eof,
  Stopped }` (internal; was `Ok(bool)`).

### Fixed

- **`cast_rtt_bench` double-hang**: a Criterion-phase seq reset
  (caller-owned counter declared inside the `|b|` closure) and a
  loss-deadlock in the echo wait (no in-loop NAK service). All
  four comparison rows re-measured same-day: casting ~9.5µs,
  raw UDP ~9.0µs (statistical tie — the floor), TCP ~15.2µs,
  QUIC ~36.3µs.

## [v0.6.1] — 20260706

> RSX v0.6.1 — turnkey deploy + an honest dashboard
>
> RSX now installs on a single box with one command, and the dev dashboard finally does what its buttons say.
>
> • Single-machine deploy — systemd units and a dry-run-by-default installer bring the whole exchange up.
> • Recorder stops lying — a watchdog faults its health when replication stalls, and old archive segments prune.
> • Mark price works again — it unwraps Binance's combined-stream envelope, so the index actually updates.
> • Marketdata stops crying wolf — per-stream sequence tracking kills false-positive retransmit storms.
> • Gateway rides out a transient port-in-use on cast rebind instead of crashing under WS churn.
> • Dashboard overhaul — one honest process count, human units, reject reasons in words, prefix-safe links.
>
> Full notes: CHANGELOG.md

### Added

- **Single-machine production deploy** (`deploy/`): systemd units for
  every process, an env template, and `deploy.sh` — a dry-run-by-default
  installer (`--apply` to mutate). Completes the single-machine sections
  of `specs/2/9-deploy.md`. Multi-server topology + Postgres-HA stay
  founder-owned.
- **Recorder archive retention** — `RSX_RECORDER_RETAIN_DAYS` prunes
  archive segments older than the window (the hot tier already retained
  4h; the archive tier had none and grew unbounded under maker churn).
- **Recorder health watchdog** — `/health` faults when the replication
  stream stalls, instead of reporting a startup constant forever
  ("healthy while dead" is gone).
- **Marketdata stream-level seq-gap detection** — tracks sequence
  per-stream so ignored record types no longer read as gaps.
- **Playwright interaction coverage** — 4 new specs (`play_links`,
  `play_render`, `play_controls`, `play_flows`) that click controls and
  assert effects, not just that pages render.
- rsx-cast: `impl AsRawFd for CastReceiver` — a founder-authorized,
  purely additive read-only accessor exposing the receiver's UDP fd so a
  caller on an async runtime (gateway on monoio/io_uring) can await
  readiness on its own reactor instead of polling `try_recv_with`. No
  runtime dep, no behavior change, no signature change. The gateway now
  parks on POLL_ADD and wakes the instant a casting datagram lands,
  dropping the old `sleep(ZERO)` yield-spin from the response path.

### Changed

- **Playground dashboard overhaul** — one authoritative process count
  everywhere; human units and reject reasons rendered as words; honest
  health / CPU / error surfaces (no cached or startup-constant values);
  WAL dump/verify and stress reports actually render their output;
  base-href is prefix-safe and depth-aware so the dashboard works behind
  a deploy prefix; topology graph shows the fill return-route; crate
  pages link their implementing crate and carry captioned demo GIFs;
  Ayam Cemani palette unified across every page.
- `start-all` is genuinely idempotent — ports derived from the spawn
  plan, daemons detached from the watcher before cleanup, port-free
  polled (not slept), orphans reaped; one PID per binary. Also reaps a
  stale `rsx-recorder` that previously survived to cause port conflicts.
- `make`: the `tui` target is guarded off the production default build.
- Docs: latency verification is stated to live in benches + the
  regression gate, never the Playwright playtests (CLAUDE.md + gate).

### Fixed

- **Mark produced no index** — the Binance *combined* stream wraps trades
  as `{stream, data:{s,p}}` but the handler read `s`/`p` at top level and
  early-returned, writing a 0-byte WAL. Now unwraps `data`; index updates.
- **Gateway crashed under WS churn (F20)** — cast rebind now rides out a
  transient `AddrInUse` instead of faulting the gateway.
- **Marketdata false-positive resends** — per-symbol seq tracking counted
  every ignored record type as a gap (11.3 gap-warns/s → 0); fixed by
  tracking per-stream.

## [v0.6.0] — 2026-05-30

> RSX v0.6.0 — warm standby + zero-alloc hot path
>
> Risk shards now keep a hot standby that fails over fast, and the
> order hot path stopped allocating per message.
>
> • Warm-standby replica — a standby applies the live stream and takes over only once caught up (advisory-lock fenced, no split-brain).
> • Zero-alloc receive — every cast loop reads packets in place via try_recv_with, no per-message heap copy.
> • Latency trace is now a compile-out macro (latency_sample!) — zero cost unless built with the trace feature.
> • Lock-free, copy-free hot tiles — removed the last per-fill Vec; an audit confirms no mutexes/copies on the busy-spin path.
>
> Full notes: CHANGELOG.md

Workspace-wide wave; rsx-cast itself is frozen (unchanged).

### Added

- Eager warm-standby replica (rsx-risk): every process loads PG, warm-
  catches-up by applying ME's WAL replication stream, then wins
  `pg_try_advisory_lock` only once caught up; the loser stays warm and
  retries. Boot/standby/promotion are one path. Strict catch-up-only
  (no cold fallback); the async staleness window is documented in
  CRASH-SCENARIOS.md. Advisory lock stays the sole single-main fence (#10).
- `rsx-types::cpu::setup_hot_thread` — concentrated hot-thread setup
  (pin + stack-warm + `mlockall` + isolation check), wired into all 5
  binaries; `notes/hot-path.md` documents the discipline.
- `rsx-types::cache::Padded<T>` — 128B-aligned false-sharing primitive.
- `latency_sample!` macro (rsx-log) — compile-time per-stage trace behind
  the `latency-trace` feature (default off = zero hot-path cost).

### Changed

- All cast-receive hot loops migrated to zero-copy `try_recv_with` — no
  per-message `Vec` alloc on risk/ME/marketdata/gateway order paths.
- Replaced the dead buffer-then-discard replica + `Role`/tip-sync state
  machine with the eager protocol (large net LOC removal).
- Gateway egress outbound queue → `Arc<str>` (refcount fan-out, not
  per-connection `String` copies).
- `check_liquidation_for` iterates the user's positions directly — the
  per-fill `Vec<u32>` alloc is gone.

### Fixed

- rsx-log test race under multi-threaded `make test` (serialized via a guard).
- Integration tests: stale persist schema refs (`funding_payments`→`funding`,
  `seq`→`last_seq`) and the advisory-lock tests now self-provision Postgres
  via testcontainers instead of a non-existent `localhost` DB.

### Audit

- No-mutex / no-copy sweep: the busy-spin tiles are confirmed lock-free and
  (now) copy-free. 19 triaged correctness findings logged in `bugs.md`
  (review queue; not auto-fixed).

## [rsx-cast v0.5.2] — 2026-05-25

Cargo.toml version finally caught up to the CHANGELOG (was
stuck at 0.5.0 through the v0.5.1 work). Plus minor:

- Spec: `CastRecv::Reconnect` variant spec'd as a sibling of
  `Faulted` in `specs/2/4-cast.md` (ring-overflow case had
  been conflated into FAULTED prose).
- README: project intent stated openly — demonstration today,
  deployment planned for esoteric special derivatives.
- New audit skills `cto-eval` + `ceo-eval` codify the
  `.ship/27` review patterns.

## [rsx-cast v0.5.1] — 2026-05-25

Internal-API trims from the `.ship/27-REFINE-AUDIT` Round 1+2
hygiene pass. Wire format and on-disk WAL bytes unchanged.

### Breaking — public API

- `WalWriter::append` removed (`496ae48`). All callers go
  through the two-step `prepare(...) → append_framed(framed)`
  path. Single-CRC fan-out across WAL + cast destinations
  uses the same `Framed` value, so an event is CRC'd once
  regardless of how many sinks consume it.
- `ReplicationConsumer::from_single(addr, ...)` removed
  (`b519ac9`). The single-endpoint constructor was a thin
  wrapper around `vec![addr]` — six callers inlined directly.
- `CastReceiver::new(socket, stream_id, _stream_id)` lost its
  vestigial third arg (`60faeb7`); signature is now
  `new(socket, stream_id)`. 23 call sites updated.
- `CastReceiver::tick()` removed (`dc2a6a6`) — was a no-op
  for the entire v4 reliability lifetime. `CastSender::tick()`
  stays (emits idle-stream heartbeats).
- `CastReceiver::is_faulted` / `is_reconnect_pending` demoted
  to `#[cfg(test)] pub(crate)` (`54f0a56`). Production
  consumers branch on the `CastRecv` enum returned by
  `try_recv`, not on the receiver's internal flags.
- `CastConfig::nak_retry_us` field removed (`f8d1c13`). The
  field had been dead for the whole of v4 — `maybe_nak()` is
  gated by `nak_debounce_us` (the per-gap debounce window),
  not by a separate retry timer. Env var `RSX_CAST_NAK_RETRY_US`
  also removed. No migration: the value was unread.
- `WalReader::stream_id()` / `wal_dir()` accessors removed
  (`ca7adba`); `WalReader::segment_file_path()` removed
  (`c528668`); `WalWriter::stream_id` / `next_seq` field
  visibility tightened to `pub(crate)` (`9bdce56`).
- `CastReceiver::highest_seen()` accessor removed (`e4b2333`).

### Internal — path-layout helpers

Path construction in `wal.rs` factored into six pure
helpers (`8efa326`, `2a3c87b`):

- `stream_dir(wal_dir, stream_id)`
- `active_filename(stream_id)`
- `segment_filename(stream_id, first_seq, last_seq)`
- `active_file_path(wal_dir, stream_id)`
- `is_active_filename(name)`
- `parse_segment_filename(name) -> Option<(stream_id, first, last)>`

No behavior change; replaces ad-hoc `format!` calls scattered
across writer + reader + GC.

### Internal — consumer wiring

- `rsx-matching/src/main.rs::process_cancel` now uses
  `publish_events` (`d009266`), matching the order-acceptance
  path. Eliminates a paired
  `write_events_to_wal + send_event_cmp + send_event_marketdata`
  loop that CRC'd each event three times. -207 LOC.
- `rsx-mark` aggregate + sweep emits use `send_framed` instead
  of `prepare + append_framed + send_raw` (`1415b7e`); the
  paired path double-CRC'd each record.

### Replication protocol visibility

- `ReplicationRequest`, `ReplicationNotAvailable`,
  `RECORD_REPLICATION_*` constants scoped to `pub(crate)`
  (`bf099e2`). The protocol is implementation detail of
  `ReplicationService` / `ReplicationConsumer`; consumers
  don't construct these directly.

## [rsx-cast v0.5.0] — 2026-05-24

Rename `rsx-dxs` → `rsx-cast`. The unified primitive is now
`rsx-cast`, with two protocol halves named in verb form:

- **casting** = the live UDP half (was "CMP / streaming
  protocol")
- **replication** = the catch-up TCP half (was "DXS / replay
  protocol")

Wire format and behavior unchanged. Numerical record-type
constants unchanged (`0x11` NAK, `0x12` HEARTBEAT, `0x13`
`RECORD_REPLICATION_REQUEST` (was `RECORD_REPLAY_REQUEST`),
`0x15` `RECORD_REPLICATION_NOT_AVAILABLE` (was
`RECORD_REPLAY_NOT_AVAILABLE`)). Wire byte layouts of all
structs unchanged.

### Breaking — Rust symbol names

| Old                          | New                          |
|------------------------------|------------------------------|
| `rsx-dxs` (crate)            | `rsx-cast`                   |
| `rsx_dxs::cmp::*`            | `rsx_cast::cast::*`          |
| `CmpSender`                  | `CastSender`                 |
| `CmpReceiver`                | `CastReceiver`               |
| `CmpRecv` (enum)             | `CastRecv`                   |
| `CmpConfig`                  | `CastConfig`                 |
| `CmpRecord` (trait)          | `CastRecord`                 |
| `CmpHeartbeat`               | `CastHeartbeat`              |
| `DxsConsumer`                | `ReplicationConsumer`        |
| `DxsReplayService`           | `ReplicationService`         |
| `ReplayRequest`              | `ReplicationRequest`         |
| `ReplayNotAvailable`         | `ReplicationNotAvailable`    |
| `RECORD_REPLAY_REQUEST`      | `RECORD_REPLICATION_REQUEST` |
| `RECORD_REPLAY_NOT_AVAILABLE`| `RECORD_REPLICATION_NOT_AVAILABLE` |

Module renames inside the crate:

- `src/cmp.rs` → `src/cast.rs`
- `src/client.rs` → `src/replication_client.rs`
- `src/server.rs` → `src/replication_server.rs`

Unchanged: `WalWriter`, `WalReader`, `WalHeader`, the
`#[repr(C, align(64))]` layouts, `RECORD_CAUGHT_UP`,
`RECORD_NAK`, `RECORD_HEARTBEAT`, `.wal` file extension,
all on-disk and on-wire bytes.

### Breaking — env vars

| Old                            | New                             |
|--------------------------------|---------------------------------|
| `RSX_CMP_HEARTBEAT_INTERVAL_MS`| `RSX_CAST_HEARTBEAT_INTERVAL_MS`|
| `RSX_CMP_SENDER_BIND_ADDR`     | `RSX_CAST_SENDER_BIND_ADDR`     |
| `RSX_CMP_NAK_RETRY_US`         | `RSX_CAST_NAK_RETRY_US`         |
| `RSX_CMP_MAX_NAK_RETRIES`      | `RSX_CAST_MAX_NAK_RETRIES`      |
| `RSX_CMP_RETX_DEDUP_WINDOW_US` | `RSX_CAST_RETX_DEDUP_WINDOW_US` |
| `RSX_CMP_REORDER_BUF_LIMIT`    | `RSX_CAST_REORDER_BUF_LIMIT`    |
| `RSX_CMP_UDP_ADDR`             | `RSX_CAST_UDP_ADDR`             |
| `RSX_*_CMP_ADDR` / `_ADDRS`    | `RSX_*_CAST_ADDR` / `_ADDRS`    |
| `RSX_ME_REPLAY_DXS_ADDR`       | `RSX_ME_REPLICATION_ADDR`       |
| `RSX_ME_DXS_ADDR`              | `RSX_ME_REPLICATION_BIND_ADDR`  |

Default values unchanged.

### Spec + bench renames

- `specs/2/4-cmp.md` → `specs/2/4-cast.md`
- `specs/2/10-dxs.md` → `specs/2/10-replication.md`
- `specs/2/35-testing-cmp.md` → `specs/2/35-testing-cast.md`
- `specs/2/36-testing-dxs.md` → `specs/2/36-testing-replication.md`
- `benches/cmp_*_bench.rs` → `benches/cast_*_bench.rs`
- `examples/cmp_smoke.rs` → `examples/cast_smoke.rs`

### Migration

Cargo.toml:

```toml
# old
rsx-dxs = { path = "../rsx-dxs" }
# new
rsx-cast = { path = "../rsx-cast" }
```

Rust:

```rust
// old
use rsx_dxs::cmp::CmpSender;
use rsx_dxs::DxsConsumer;
// new
use rsx_cast::cast::CastSender;
use rsx_cast::ReplicationConsumer;
```

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
