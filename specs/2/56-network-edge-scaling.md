# 56 — Network-Edge I/O Scaling (SQPOLL + userspace UDP)

Status: **spec** (not implemented). Throughput/latency scaling for the
gateway/marketdata I/O edge, past what testing needs. Off the critical
publish path — see README §Roadmap step 3.

Concept: [docs/concepts/05-network-edge-io.md](../../docs/concepts/05-network-edge-io.md).

## Why

The gateway/marketdata per-request cost is syscall-dominated (`sendto`
≈ 3.5 µs, ~99% of send). monoio already batches io_uring submissions, but
the last rungs of the ladder — kernel-polled submission (SQPOLL) and
kernel-bypass UDP — need explicit wiring. These are production-scale
levers, not correctness. Deferred out of a normal session because they
are kernel-dependent and cannot be runtime-verified without the deploy
box (a real kernel with io_uring SQPOLL + `CAP_SYS_NICE`).

## Scope

In scope: (1) SQPOLL for gateway + marketdata reactors, gated on the
dedicated-core config; (2) the userspace-UDP prerequisite (cast API
decoupling). Out of scope: multishot recv / registered buffers / GSO
(later rungs), DPDK/AF_XDP (much later), any rsx-cast behavior change
beyond the two additive APIs below.

## Part A — SQPOLL, gated on the core config

**Feasibility (verified in this repo):** monoio 0.2.4's `FusionDriver`
passes its `uring_builder` through to the io_uring path
(`builder.rs:174/184`), so `RuntimeBuilder::uring_builder(urb)` with
`urb.setup_sqpoll(idle_ms)` reaches the ring. Needs `io-uring` as a
direct dep of `rsx-gateway` + `rsx-marketdata`, version-matched to
monoio 0.2.4's (read it from `Cargo.lock`).

**Gate, not a flag.** Enable SQPOLL only when the deployment has already
committed a dedicated core — i.e. `RSX_GW_CORE_ID` / `RSX_MD_CORE_ID` is
set. In testing (no core id) it stays off. SQPOLL burns a polling kernel
thread; tying it to the cores-to-spare signal is the whole point (no new
on/off knob).

**Fallback is mandatory.** `IORING_SETUP_SQPOLL` needs `CAP_SYS_NICE` (or
root) on many kernels; without it the ring build fails. Build the runtime
**try-SQPOLL-first, else plain io_uring** so the gateway always boots:

```
fn build_runtime(sqpoll: bool) -> io::Result<Runtime> {
    let mut b = RuntimeBuilder::<FusionDriver>::new();
    b = b.enable_timer();
    if sqpoll {
        let mut urb = io_uring::IoUring::builder();
        urb.setup_sqpoll(SQPOLL_IDLE_MS);   // e.g. 1000
        b = b.uring_builder(urb);
    }
    b.build()
}
// caller: sqpoll = core_id.is_some(); build_runtime(sqpoll).or_else(|_| build_runtime(false))
```

Log which path won (`sqpoll=on` vs `fell back`). Same shape in both crates.

### Success criteria

- With the core id set on a capable box: `build_runtime(true)` succeeds and
  SQPOLL engages — verify by `strace -c -e io_uring_enter <gateway>` under
  load showing near-zero `io_uring_enter` calls, or `/proc/<pid>/status`
  showing the extra kernel poll thread.
- Without `CAP_SYS_NICE` (or no core id): the gateway still boots via the
  plain-io_uring fallback; no panic. This is the case that must be tested
  in CI, since CI can't grant the cap.
- No regression to the default (no-core-id) path: identical behavior to today.

## Part B — Userspace UDP (the prerequisite for io_uring at the caller)

io_uring on the UDP path must live in the socket-owning caller (matching's
ME hot path, gateway/marketdata edges), because rsx-cast is runtime-dep-free
by invariant. Today `CastSender`/`CastReceiver` own the `UdpSocket` and
couple framing with `recv`/`send`. **Two additive rsx-cast APIs** unblock
it (a sanctioned frozen-cast extension — founder sign-off required, per
`rsx-cast/CLAUDE.md`):

1. Expose a built `Framed`'s bytes (`Framed::as_bytes(&self) -> &[u8]`) so
   the caller io_uring-sends them.
2. A parse-already-received-bytes entry (`CastReceiver::process(&[u8])` or
   a standalone frame parser) so the caller io_uring-recvs, then hands the
   bytes to cast for framing/WAL.

Single-packet request-response gains little from batching — the lever is
SQPOLL (Part A). So Part B is worth it mainly at the fan-in/fan-out edges,
and only after Part A is proven. See BUGS.md `CAST-SOCKET-COUPLING-BLOCKS-IOURING`.

## Deploy target

Both processes ship as the existing binaries; no new artifact. SQPOLL is a
runtime path selected by env at boot. Part B is a cast API + caller rewire,
a later milestone.

## Current state baseline

Neither part implemented. Gateway/marketdata build `monoio::FusionDriver`
with `.enable_timer()` and no `uring_builder`. matching/gateway/marketdata
use `rsx-cast`'s std `UdpSocket` via `CastSender`/`CastReceiver`.
