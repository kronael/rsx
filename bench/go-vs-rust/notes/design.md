# Why these two cases (and not the folklore ones)

The question: for concurrent workloads, when does Go beat Rust, when does
Rust beat Go, and by how much. A "case" here is the smallest program pair
that isolates one scheduling/memory mechanism, written idiomatically on both
sides, doing byte-identical work (same splitmix64 PRNG, same seeds; the
runner refuses to report if the Go and Rust checksums differ).

## Case A — tail latency under unpredictable CPU bursts

**Problem.** Real request handlers occasionally hit an expensive pure-CPU
path (a big parse, a compression, a pathological query). Cooperative
schedulers only reschedule at yield points, and pure computation has none:
a tokio worker that enters a 100 ms compute loop is gone for 100 ms — every
task queued behind it waits ([tokio's own docs][ryhl]: >10–100 µs between
awaits is "blocking"; the coop budget added in [tokio 1.0][coop] only helps
at `.await` points, of which a compute loop has zero). Go took the other
branch: since [Go 1.14][go114], the runtime preempts any goroutine
asynchronously after ~10 ms, no cooperation needed.

**The case.** Open-loop server sim: 50k req/s released on a 1 ms tick, one
task per request; 1-in-N requests (PRNG on request id) run an
80M-iteration PRNG chain (~100 ms in Go, ~75 ms in Rust — same work,
native speed), the rest run 8k iterations (~10 µs / ~7.5 µs). Spins are
iteration-bounded, never wall-clock-bounded — a wall-clock spin would let a
preempted Go burst silently shed CPU work while descheduled — and each
spin's fold feeds the cross-language checksum, so identical work is proved,
not assumed. Latency of light requests is measured against the *intended*
arrival tick — open-loop, so queueing during block-outs is fully counted
(no coordinated omission). Two operating points: ~30 bursts/s (avg ~2–3 in
flight) and ~60 bursts/s (avg ~5–6 in flight, Go higher than Rust because
its bursts run 1.34x longer). Both stay below total capacity, so tails come
from scheduling policy, not sustained overload.

**Why two operating points.** Work stealing rescues tokio as long as at
least one worker is free: the crossover is P(concurrent bursts ≥ workers).
At ~30/s that probability is well under 2% — tokio holds its own. At ~60/s
(Poisson mean ~5–6 vs 8 workers, plus clumping as block-outs delay bursts)
naive tokio's light requests stall behind full bursts; Go's ~10 ms quantum
caps the damage even though Go is simultaneously carrying ~1.34x more CPU
load for the same offered work. The crossover is a load threshold, not a
language property.

**Fairness.** Rust also gets a tuned row: `tokio::task::block_in_place`
around the heavy spin — the documented fix. It wins outright (better than
Go) *when the code can predict which requests are heavy before running
them*. Go's advantage is not raw speed here; it is that nobody has to
predict anything. Symmetrically, Go gets a tuned row in case B.

## Case B — CPU + allocation throughput and tail

**Problem.** A GC buys allocation convenience with background CPU and — at
high allocation rates — *assists*: a goroutine allocating during a mark
phase is drafted to help scan, adding unpredictable stalls exactly on the
allocating path ([Go GC guide][gcguide]). Rust pays malloc/free inline:
more predictable, no background marker, no heap headroom multiplier.

**The case.** 8 workers (= cores), no cross-worker communication. Per
message: build a 64–320-element `Vec<u64>`/`[]uint64` from the PRNG, xor it,
sort it, binary-probe a rolling window of the last 512 messages (a live
heap the GC must re-mark; displaced messages become garbage in Go, an
inline `free` in Rust), record per-message latency. Both sides allocate per
message — this is *not* a zero-alloc-Rust strawman; Rust simply pays the
allocator inline while Go pays GOGC-amortized.

**Fairness.** Go also runs with `GOGC=1000` (the standard "trade memory for
GC CPU" tuning, [GC guide][gcguide]) so the "you didn't tune Go" objection
is answered with a number, including its memory bill.

## Folklore cases we measured and kept as probes (P1, P2)

Both are widely believed Go wins. Neither reproduced on modern tokio
(1.53); we keep them runnable (`./run.py --cases P1,P2`) as evidence
against strawmanning either side:

- **P1 — channel ping-pong at scale.** 50k actor pairs (100k tasks), 200
  roundtrips each, unbuffered chans vs `mpsc(1)`. Belief: Go's
  runtime-integrated channels (direct handoff to `runnext`) beat tokio's
  waker path. Measured: tokio is ~25% *faster* in throughput with better
  p50/p99 and ~40% less RSS — its [LIFO slot][sched10x] optimizes exactly
  this message-passing pattern. Go additionally showed rare enormous
  roundtrip outliers (hundreds of ms) under 100k runnable goroutines.
- **P2 — 50k concurrent sleepers.** Periodic absolute-deadline ticks +
  fan-in channel. Belief: tokio's globally-locked timer wheel melts under
  many timers ([#6257][d6257]). Measured: that was fixed by [sharding the
  timer driver][i6504] (tokio 1.38+); tokio now matches or beats Go here
  and uses ~4x less memory for the same task count.

The through-line: the honest Go-vs-Rust concurrency story in 2026 is not
"channels are faster" folklore — per-mechanism, modern tokio matches or
beats the Go runtime while using less memory (a tokio task is a heap-sized
future, ~hundreds of bytes; a goroutine starts at 2 KiB of stack plus
runtime bookkeeping). What Go retains is *robustness by default*:
preemption and GC mean unpredicted CPU bursts and allocation storms degrade
gracefully without any architectural foresight, and case A@6 prices exactly
that. Rust retains everything else: case B prices the GC's tail and
footprint cost that no GOGC setting removes.

## Measurement rules (both sides, always)

- Identical splitmix64 PRNG, identical constants; runner aborts on
  cross-language checksum mismatch (proof of identical work).
- In-process warmup excluded from samples; plus one discarded warm-up run;
  5 measured runs; per-metric median across runs reported, min–max
  throughput spread shown.
- Percentiles: nearest-rank over every sampled op (millions of samples),
  preallocated arrays, no allocation while measuring.
- Peak RSS from `/proc/self/status` VmHWM; CPU cores from `/proc/self/stat`
  utime+stime around the measured window only. Same code path both sides.
- Stock `go build` vs stock `cargo build --release`; no GOGC/GOMAXPROCS or
  runtime-builder tuning outside the explicitly labeled tuned rows.

[ryhl]: https://ryhl.io/blog/async-what-is-blocking/
[coop]: https://tokio.rs/blog/2020-04-preemption
[go114]: https://go.dev/doc/go1.14#runtime
[gcguide]: https://tip.golang.org/doc/gc-guide
[sched10x]: https://tokio.rs/blog/2019-10-scheduler
[d6257]: https://github.com/tokio-rs/tokio/discussions/6257
[i6504]: https://github.com/tokio-rs/tokio/issues/6504
