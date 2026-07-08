# 3.1 — Cast: failover coordinator + transport decoupling (io_uring)

Status: **draft (phase 3)**. First spec of phase 3. Extends `rsx-cast`
additively with two capabilities it does not have today: (A) a
**runtime-free failover coordinator** (warm-standby → promote), and (B) a
**transport decoupling** so callers can drive I/O with io_uring/SQPOLL. The
two share one theme: cast becomes a transport-agnostic, failover-capable
broker primitive — the direction of the cast-as-broker vision — without
ever taking an async-runtime dependency.

## Governance — this is a sanctioned frozen-cast extension

`rsx-cast` is **frozen**: new API needs the founder's explicit sign-off
before code (`rsx-cast/CLAUDE.md`). This spec records that sign-off for
phase 3 and the invariants the extension must keep:

- **No async runtime, ever, in cast** (non-revisable). The failover fence
  (Postgres) and the io_uring reactor both live in the **caller**; cast
  exposes them only through **traits / byte-level APIs**.
- **Additive only.** Same wire bytes; same `Framed` framing; no breaking
  change to `CastSender`/`CastReceiver`/`WalWriter`/`ReplicationConsumer`.
  Every std-UDP caller keeps working unchanged.
- **One CRC, one seq** — the `Framed::pack` core (`wal.rs`) stays the sole
  framing path; new APIs expose its bytes, they do not add a second framer.

This is a first pass — *simplify after it proves out* (founder note). The
spec is written so each part ships and is testable on its own.

Cross-refs: `.ship/41-MATCHING-RELEASE/DESIGN-me-failover.md` (the ME
failover mechanics this generalizes), `specs/2/56-network-edge-scaling.md`
(SQPOLL/userspace-UDP that Part B unblocks), BUGS
`CAST-SOCKET-COUPLING-BLOCKS-IOURING` (the io_uring blocker Part B closes),
`rsx-risk/src/failover.rs` + `lease.rs` (the proven pattern Part A lifts).

---

## Part A — Failover coordinator (runtime-free)

### Problem

Two services need identical warm-standby failover: `rsx-risk` (has it,
`failover.rs` + `lease.rs`) and `rsx-matching` (has none — invariant #10
unenforced for ME, so two MEs for one symbol could both write =
split-brain). Duplicating risk's ~200 lines into ME leaves two copies to
drift (the CTO review already flags this class of drift). The transport
half of the mechanism — tailing a WAL stream to a caught-up point — is
*already* cast's (`ReplicationConsumer`, `RECORD_CAUGHT_UP`). Only the
**fence** (which single writer wins) is domain-specific.

### Design — coordinator in cast, fence injected

Cast gains a **transport-only** coordinator that runs the state machine
`WarmCatchup → (fence acquired) → Live → (fence lost) → WarmCatchup`,
driving an existing `ReplicationConsumer` and calling out to a caller-
supplied fence. Cast never learns *how* the fence works.

```rust
// new in rsx-cast — a trait, so no runtime/Postgres dep enters cast.
pub trait Fence {
    /// Non-blocking attempt to become the single writer. True = held.
    fn try_acquire(&mut self) -> bool;
    /// Still held? Called each renew tick; false triggers demote.
    fn is_held(&mut self) -> bool;
    fn release(&mut self);
}

// applies one streamed record into caller state; caller owns the book/
// positions/dedup. Returns the applied seq. Keeps cast domain-agnostic.
pub trait ReplicaApply {
    fn apply(&mut self, record: &[u8]) -> u64;
}

pub enum Role { WarmCatchup, Live }

pub struct FailoverCoordinator<F: Fence, A: ReplicaApply> { /* … */ }
impl<F: Fence, A: ReplicaApply> FailoverCoordinator<F, A> {
    /// Drive one turn: stream+apply toward the tip, and when caught up
    /// try the fence. Returns the current Role. The caller loops this on
    /// its own (pinned) thread — cast spawns nothing.
    pub fn poll(&mut self, consumer: &mut ReplicationConsumer) -> Role;
}
```

**The fence ordering is the whole safety argument** (identical to risk,
`failover.rs:169-173`): every write-capable resource (WAL writer as active,
order socket bind, fan-out senders, replication *server*) is constructed by
the caller **only after** `poll` returns `Role::Live` — i.e. strictly after
`Fence::try_acquire` returned true. Postgres releases an advisory lock when
the holder's session ends (crash/exit), so the first instant a second
instance can write is strictly after the first stopped. No overlap window.

**Consumers.**
- *risk* migrates `run_warm_catchup` onto `FailoverCoordinator` (its
  `AdvisoryLease` implements `Fence`; `apply_record` implements
  `ReplicaApply`). Behavior-preserving refactor.
- *ME* gains failover for the first time by implementing the same two
  traits: `Fence` = a Postgres advisory lock **keyed `(ME_LOCK_CLASS,
  symbol_id)` via the two-int32 `pg_try_advisory_lock($1::int,$2::int)`
  form** — NOT the single-bigint `symbol_id`, which would collide with
  risk's `shard_id` in Postgres's shared advisory space (see the ME design
  doc §1). `ReplicaApply` = the extracted `apply_me_record`
  (book + dedup + order-index).

**PG becomes required for a fenced service.** ME's Postgres connection is
optional today (config-poll only); holding the lease makes it mandatory —
fail-fast at startup without `RSX_ME_DATABASE_URL` (as risk already does).

### What cast provides vs what stays in the caller

| Concern | Home |
|---|---|
| stream tail, caught-up detection, tip tracking | **cast** (`ReplicationConsumer`, extended into the coordinator) |
| state machine WarmCatchup↔Live, promote/demote ordering | **cast** (`FailoverCoordinator`, runtime-free, spawns nothing) |
| the fence (Postgres advisory lock) | **caller** (`impl Fence`) |
| applying a record to domain state | **caller** (`impl ReplicaApply`) |
| the pinned thread / loop cadence | **caller** (calls `poll` in its own loop) |

---

## Part B — Transport decoupling (io_uring / SQPOLL)

### Problem (BUGS `CAST-SOCKET-COUPLING-BLOCKS-IOURING`)

`CastSender`/`CastReceiver` (`rsx-cast/src/cast.rs`) **own the
`UdpSocket`** and couple framing with the syscall: `try_recv_with` does
`recvfrom` + parse in one; `send_framed` writes on the owned socket. To
move the hot-path `recvfrom`/`sendto` onto io_uring/SQPOLL
(`specs/2/56`), the io_uring reactor must own the fd — but it lives in the
caller (cast stays runtime-free). So the caller must own the socket, and
cast must expose framing at the **byte** boundary.

### Design — two additive byte-level APIs (exactly as BUGS pre-scoped)

1. **Expose a built frame's bytes** — `Framed::as_bytes(&self) -> &[u8]`
   (or `into_bytes`). The caller builds a `Framed` through the normal one-
   CRC/one-seq path (`WalWriter::prepare` / the send builders), then
   io_uring-`sendto`s the bytes itself. Cast still assigned the seq + CRC;
   only the syscall moved out.
2. **Parse already-received bytes** — `CastReceiver::process(&mut self,
   buf: &[u8]) -> …` (or a standalone `parse_frame(&[u8])`). The caller
   io_uring-`recvfrom`s into its own buffer, hands the bytes to cast for
   frame validation + WAL append + dispatch. Cast still owns
   framing/CRC-check/WAL; only the syscall moved out.

Both are **additive**: the existing owned-socket `send_framed` /
`try_recv_with` stay for std-UDP callers. A caller opts into io_uring by
owning the fd and using the byte APIs instead. Same wire bytes on both
paths (a std-UDP peer and an io_uring peer interoperate).

SQPOLL, registered buffers, multishot recv, GSO, kernel-bypass — all live
in the **caller's** reactor (`specs/2/56` owns that ladder). Cast is
oblivious; it only stopped owning the syscall.

---

## Part C — "Support all of these": one coordinator over any transport

The point of doing A and B together: the **failover coordinator (A) must
run over either transport backend (B)** — std-UDP busy-spin *or*
io_uring/SQPOLL — without knowing which. Achieved because:

- The coordinator drives a `ReplicationConsumer`, which is a **TCP**
  cold-path stream (catch-up/replay) — unchanged, transport-agnostic
  already; io_uring affects only the **live UDP** hot path, not catch-up.
- Post-promotion, the caller constructs its live I/O with **whichever**
  backend it chose (owned-socket std-UDP via `send_framed`, or io_uring via
  `Framed::as_bytes` + caller `sendto`). The coordinator returned
  `Role::Live`; it does not care how the caller then does UDP.

So the matrix cast supports after phase 3:

| | std-UDP (busy-spin) | io_uring / SQPOLL |
|---|---|---|
| live hot path | `send_framed`/`try_recv_with` (today) | `Framed::as_bytes` + caller reactor (Part B) |
| catch-up / replay | `ReplicationConsumer` (TCP, today) | same (TCP, unaffected) |
| failover | `FailoverCoordinator` + caller `Fence` (Part A) | `FailoverCoordinator` + caller `Fence` (Part A) |

One coordinator, one framing core, two interchangeable hot-path backends,
one cold-path — cast as a transport-agnostic, failover-capable broker.

---

## Phased implementation plan

1. **B.1 — byte APIs.** `Framed::as_bytes` + `CastReceiver::process`
   (additive, no behavior change). Unit-test round-trip byte-identity vs
   `send_framed`/`try_recv_with`. Smallest, unblocks `specs/2/56`.
2. **A.1 — coordinator + traits.** `Fence`, `ReplicaApply`,
   `FailoverCoordinator::poll` in cast. Migrate risk onto it
   (behavior-preserving; risk's tests are the regression gate).
3. **A.2 — ME failover.** ME implements `Fence` (two-int32 key) +
   `ReplicaApply` (`apply_me_record`); refactor ME `main` into
   `run_active` + coordinator loop. Tests: split-brain (2nd ME never
   writes), ME-key-vs-risk-key non-collision, kill-active→promote.
   (Full mechanics in the ME design doc.)
4. **B.2 — io_uring hot path** in the ME/gateway/marketdata callers, over
   the Part B APIs, gated per `specs/2/56` (SQPOLL, dedicated-core).
   Kernel-dependent; verified on the deploy box, not in a normal session.

Parts B.1 and A.1 are independent and can land in either order; A.2 needs
A.1; B.2 needs B.1.

## Open questions (for the "simplify later" pass)

1. **`ReplicaApply` granularity** — per-record callback vs handing the
   coordinator a batch; batch is fewer virtual calls but couples buffering.
   Start per-record (simplest), revisit if it shows on a profile.
2. **Standby's base state** — a cold standby replaying from seq 1 hits
   pre-retention gaps and must federate to ARCHIVE (`specs/2/10`
   multi-endpoint). Does the coordinator own endpoint-federation, or does
   the caller hand it a pre-seeded `ReplicationConsumer`? (Lean: caller
   supplies the consumer, keeps cast policy-free.)
3. **Fence cadence** — poll/renew intervals as coordinator params vs caller
   loop cadence. Lean: caller owns cadence (it owns the loop), coordinator
   is pure state.
4. **Where the shared `Fence` Postgres impl lives** — a thin domain helper
   both risk and ME use, or each rolls its own `impl Fence`. Not cast's
   concern either way (cast only sees the trait).
5. **io_uring buffer ownership** at `CastReceiver::process` — borrow vs
   take; must not force a copy on the std-UDP path.

## Non-goals

- No change to the wire format, the WAL format, or the one-CRC/one-seq
  framing core.
- No async runtime, Postgres, or io_uring dependency in `rsx-cast`.
- Not DPDK/AF_XDP (much-later rungs, `specs/2/56` non-goals).
- Not the fully-managed/automatic-everything failover UX — this is the
  mechanism; the operator surface (playground showcase, runbook) rides on
  top.
