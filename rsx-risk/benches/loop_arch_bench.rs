// Bench doc-comments aren't rendered to docs; skip the markdown lint.
#![allow(clippy::doc_lazy_continuation)]
//! Loop-architecture load benchmark: where does the round-trip time go?
//! ====================================================================
//!
//! THE QUESTION
//! ------------
//! A risk/gateway tile is, stripped of domain logic, a request-processing
//! stub:
//!
//!     recv-from-client -> calc -> submit-to-downstream -> recv-downstream
//!                       -> send-up-to-client
//!
//! At ~10k connections on a handful of cores, per-request latency is an
//! AGGREGATE: the box does a fixed amount of work per request, and 10k
//! clients share N cores. Reorganizing the work across threads/reactors
//! does NOT add core capacity and does NOT reduce total work. So the
//! honest question is not "which loop architecture is fastest" but:
//!
//!     For an identical workload + identical offered load, what aggregate
//!     round-trip does each runtime architecture achieve, and WHERE does
//!     the time actually go (work, syscalls, or scheduling)?
//!
//! We measure four stub architectures against an identical calc, an
//! identical single echo service, and identical offered load, then
//! attribute the cost.
//!
//! THE LEVERS (hypotheses we test, not assume)
//! -------------------------------------------
//!   1. work per request   -- copies + a cheap transform (identical here).
//!   2. SYSCALL COUNT per request -- each recvfrom/sendto is ~1-4 us of
//!      kernel transition; at 10k x rate this dominates the budget. The
//!      genuine work-reducer is batching syscalls (recvmmsg/sendmmsg).
//!   3. not idling while work waits -- a reactor that sleeps when an echo
//!      is in flight wastes core time another request could use.
//!
//! If every variant is syscall-bound and roughly equal, the bench SAYS SO.
//! It does not crown a winner by construction.
//!
//! EQUAL CORE BUDGET (conclusion-critical -- read this)
//! ----------------------------------------------------
//! Every variant is given the SAME total core budget K (the swept axis,
//! `LAB_NS`, is now K -- the per-variant CORE BUDGET, not a reactor count).
//! Variants that need an always-hot helper thread (tile's spin receiver,
//! batched's mmsg thread) pay for it OUT OF their budget:
//!
//!     monoio-sharded  : K reactors,             0 helper   -> K cores
//!     tokio           : K worker threads,        0 helper   -> K cores
//!     busy-spin-tile  : (K-1) reactors + 1 spin helper      -> K cores
//!     batched-syscall : (K-1) reactors + 1 batch helper     -> K cores
//!
//! `service_us` is computed with K for every variant (K x window /
//! completed). The echo service runs on its OWN core OUTSIDE K in every
//! variant (it is the constant downstream, not part of the SUT budget).
//! The per-variant reactor/helper split is printed in the table so the
//! accounting is visible. This is THE fix for the original bench, which
//! gave tile/batched a free extra always-hot core and still divided by N.
//!
//! THE FOUR VARIANTS (only the stub architecture differs)
//! ------------------------------------------------------
//!   1. monoio-sharded  -- K pinned monoio current-thread reactors; client
//!      conns sharded across them by SO_REUSEPORT; the echo submit+recv is
//!      done as monoio UDP I/O on the reactor. A `sleep(ZERO)` drain-yield
//!      mirrors the real casting-recv loop in rsx-gateway/rsx-risk.
//!   2. tokio           -- one tokio multi-thread runtime, K worker threads;
//!      same logic, tokio sockets, work-stealing scheduler.
//!   3. busy-spin-tile  -- (K-1) pinned reactors own the client conns (read +
//!      write); ONE dedicated pinned thread (the K-th core) busy-spins on the
//!      echo socket and routes each echo reply back to the owning reactor over
//!      a per-reactor rtrb SPSC ring (reactor_idx + conn token + send-stamp
//!      ride in the echo payload). Submit-to-echo is a direct UDP send from
//!      the reactor; only the echo RECV is centralized + spun.
//!   4. batched-syscall -- (K-1) monoio reactors + 1 batch helper (the K-th
//!      core); the echo submit+recv is batched with sendmmsg/recvmmsg (many
//!      datagrams per syscall) to isolate the syscall-amortization lever in
//!      (2) above. Partial batches flush PROMPTLY (whatever drained this turn
//!      goes out in one sendmmsg) so the variant does not stall waiting for a
//!      full batch under low in-flight depth -- batching needs pipeline depth
//!      to amortize, and the bench shows that rather than deadlocking.
//!
//! SYSTEM UNDER TEST (per client request)
//! --------------------------------------
//!   recv length-prefixed frame from client TCP conn
//!   -> CALC: memcpy a 512B buffer + a cheap byte transform (xor-fold);
//!            identical fn across all variants; we time it directly.
//!   -> SUBMIT: UDP send the request token to the single echo service.
//!   -> RECV ECHO: read the echo back (per-variant: inline await / spun
//!                 + ring / batched).
//!   -> SEND UP: write the length-prefixed response back on the conn.
//!
//! ECHO SERVICE: ONE std-UDP thread pinned to its OWN core (outside the N
//! stub cores), recv -> send-back, in every variant. It is a constant; it
//! stands in for the whole downstream (ME etc.). It is intentionally not
//! the bottleneck (single small datagram, no work).
//!
//! CLIENTS / OFFERED LOAD: real loopback TCP connections (length-prefixed
//! binary frames). Two load models:
//!   * CLOSED-LOOP (default): a fixed pool of `CONNS` connections, each holding
//!     `PIPELINE` requests in flight, refilled only on completion. This keeps
//!     offered load bounded and stops generator meltdown, but it measures
//!     "RTT at concurrency = conns x pipeline", NOT latency under a fixed
//!     external arrival rate. Under saturation a closed loop slows its own
//!     offered rate -- this is COORDINATED OMISSION: the latency a request
//!     would have seen had it been sent on schedule is not recorded, so tail
//!     numbers near saturation UNDERSTATE the real tail. Read closed-loop p99.9
//!     as "tail at this concurrency", not "tail under fixed load".
//!   * OPEN-LOOP (LAB_OPEN_RATE>0): each conn fires at a fixed per-conn rate
//!     regardless of whether prior requests completed; RTT is measured against
//!     the SCHEDULED send time. This is the proper near-saturation test (it
//!     does not hide coordinated omission) but can back up the conn's socket
//!     buffer if the server cannot keep up -- watch req/s vs offered rate.
//! Each request is stamped with a monotonic send-time at the client; the
//! load side records round-trip = recv_time - send_time. We try to raise
//! RLIMIT_NOFILE in-process and open as many conns as the box allows; the
//! ACHIEVED connection count is reported (loopback fd/ephemeral-port limits
//! on a 6-core dev box may cap below 10k -- we scale to the max and say so).
//!
//! HOW TO RUN
//! ----------
//!     cargo build -p rsx-risk --bench loop_arch_bench
//!     cargo bench  -p rsx-risk --bench loop_arch_bench
//!
//! Environment knobs (all optional; defaults chosen for a 6-core box):
//!     LAB_CONNS=2000      target client connections (capped by fd/ports)
//!     LAB_PIPELINE=1      in-flight requests per connection (CLOSED-LOOP).
//!                         Batching needs depth>1 to amortize; raise to see it.
//!     LAB_NS="2,4"        comma-separated CORE BUDGETS K to sweep (total cores
//!                         per variant incl. any helper; see EQUAL CORE BUDGET)
//!     LAB_SAMPLES=200000  completed round-trips to record per (variant,K)
//!     LAB_WARMUP_MS=2000  warm-up before recording (>=2s: 10k conns settle slow)
//!     LAB_DURATION_MS=0   if >0, cap each measurement window (else sample-bound)
//!     LAB_VARIANTS="all"  subset, e.g. "monoio,tokio,tile,batched"
//!     LAB_OPEN_RATE=0     if >0, OPEN-LOOP mode: each conn fires at this fixed
//!                         per-conn rate (req/s) regardless of completions, so
//!                         RTT is measured under fixed offered load (the proper
//!                         saturation test). 0 = closed-loop (default).
//!
//! HOW TO READ THE RESULTS
//! -----------------------
//! Two tables print to stdout:
//!
//!   (A) round-trip table -- per (variant, N): p50/p99/p999/max in us, plus
//!       achieved throughput (req/s). Lower latency / higher throughput is
//!       better, BUT compare WITHIN a fixed N and fixed offered load only.
//!       Across N the offered load is the same, so more cores generally
//!       means lower aggregate latency until the echo/loopback saturates.
//!
//!   (B) ATTRIBUTION table -- per (variant, N): measured calc-ns (the pure
//!       work), echo-syscalls-per-request (recvfrom+sendto, or the batched
//!       equivalent amortized over the datagrams a single syscall moved),
//!       and the server-side service estimate. Read it like this:
//!         * calc-ns small + syscalls/req ~2 + p50 tracks syscalls -> the
//!           system is SYSCALL-BOUND; batching is the only real lever.
//!         * batched variant shows syscalls/req << 2 and lower p50 -> the
//!           syscall lever is real and quantified.
//!         * variants near-equal at fixed N with syscalls/req ~2 -> report
//!           "syscall-bound, ~equal"; the tile does NOT win by reorg alone.
//!
//! CAVEATS (read before quoting any number)
//! ----------------------------------------
//!   * SYNTHETIC. The calc is a memcpy+xor, not real margin math; the echo
//!     is a no-op datagram, not a matching engine. Absolute numbers are not
//!     production latencies -- the SHAPE and the ATTRIBUTION are the point.
//!   * LOOPBACK. All sockets are 127.0.0.1. Loopback has no NIC, no PCIe, no
//!     wire; it exercises the kernel socket path only. Real NICs add latency
//!     and change the syscall/copy economics (and is exactly where DPDK/
//!     AF_XDP would later change the picture).
//!   * SINGLE BOX, shared cores. The load generator, echo service, and stub
//!     all run on the same machine and contend for the same 6 cores. We pin
//!     to isolate, but cache/memory bandwidth and the scheduler are shared.
//!   * PINNING IS TO LOGICAL CPUs. `core_affinity::get_core_ids()` returns
//!     logical CPUs (SMT siblings, not physical cores). On an SMT/NUMA box two
//!     "cores" in the budget K may be hyperthread siblings sharing one physical
//!     core, or land on different NUMA nodes -- both change the result. The
//!     bench does not pin IRQs or steer softirqs. Treat the core budget as
//!     "logical CPUs", and on a real measurement isolate physical cores
//!     (isolcpus / numactl) before quoting numbers.
//!   * SYSCALLS/REQ IS ECHO-SIDE-ONLY + APPROXIMATE. The counter tallies only
//!     the stub's echo send/recv CALLS (incl. failures). It does NOT count TCP
//!     read/write, accept, epoll/io_uring submit+complete, timers, wakeups,
//!     ring handoffs, or allocator syscalls. It is a LEVER INDICATOR (does this
//!     variant amortize the echo syscall?), not a total syscall budget. For
//!     rigorous attribution run under `perf stat -e syscalls:*,context-switches`
//!     (or strace -c / eBPF) -- the trustworthy in-bench metrics are LATENCY
//!     and THROUGHPUT; syscalls/req only labels the regime.
//!   * NOT a microbenchmark harness. This is a hand-rolled load test (no
//!     Criterion) because the unit under test is a multi-thread system, not
//!     a pure function. Run it a few times; expect a few % run-to-run noise.

use std::io::Read;
use std::io::Write;
use std::net::Shutdown;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::TcpStream;
use std::net::UdpSocket;
use std::os::fd::AsRawFd;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Barrier;
use std::sync::Mutex;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use core_affinity::CoreId;
use hdrhistogram::Histogram;

// ---------------------------------------------------------------------------
// Tunables (env-overridable; see module doc).
// ---------------------------------------------------------------------------

const FRAME: usize = 512; // request/response payload size (the "~512B buffer")
const ECHO_PAYLOAD: usize = 32; // small token to the echo service (idx+token+stamp)

struct Cfg {
    conns: usize,
    pipeline: usize,
    ns: Vec<usize>, // CORE BUDGETS K to sweep (total cores per variant incl. helper)
    samples: u64,
    warmup_ms: u64,
    duration_ms: u64,
    open_rate: u64, // per-conn req/s in open-loop mode; 0 = closed-loop
    variants: Vec<Variant>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Variant {
    MonoioSharded,
    Tokio,
    BusySpinTile,
    BatchedSyscall,
}

impl Variant {
    fn label(self) -> &'static str {
        match self {
            Variant::MonoioSharded => "monoio-sharded",
            Variant::Tokio => "tokio",
            Variant::BusySpinTile => "busy-spin-tile",
            Variant::BatchedSyscall => "batched-syscall",
        }
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(default)
}

fn load_cfg() -> Cfg {
    let ns = std::env::var("LAB_NS")
        .ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect::<Vec<usize>>())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec![2, 4]);
    let variants = std::env::var("LAB_VARIANTS")
        .ok()
        .filter(|s| s.trim() != "all" && !s.trim().is_empty())
        .map(|s| {
            s.split(',')
                .filter_map(|x| match x.trim() {
                    "monoio" | "monoio-sharded" => Some(Variant::MonoioSharded),
                    "tokio" => Some(Variant::Tokio),
                    "tile" | "busy-spin-tile" => Some(Variant::BusySpinTile),
                    "batched" | "batched-syscall" => Some(Variant::BatchedSyscall),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            vec![
                Variant::MonoioSharded,
                Variant::Tokio,
                Variant::BusySpinTile,
                Variant::BatchedSyscall,
            ]
        });
    Cfg {
        conns: env_usize("LAB_CONNS", 2_000),
        pipeline: env_usize("LAB_PIPELINE", 1),
        ns,
        samples: env_u64("LAB_SAMPLES", 200_000),
        warmup_ms: env_u64("LAB_WARMUP_MS", 2_000),
        duration_ms: env_u64("LAB_DURATION_MS", 0),
        open_rate: env_u64("LAB_OPEN_RATE", 0),
        variants,
    }
}

// ---------------------------------------------------------------------------
// Global syscall + calc counters. Shared by every variant's stub threads so
// the attribution table reports the SAME accounting regardless of runtime.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Counters {
    echo_sends: AtomicU64,    // sendto / sendmmsg CALL count (syscalls)
    echo_recvs: AtomicU64,    // recvfrom / recvmmsg CALL count (syscalls)
    echo_datagrams: AtomicU64, // datagrams actually moved (for batch amortization)
    requests: AtomicU64,      // server-side completed requests
}

impl Counters {
    fn reset(&self) {
        self.echo_sends.store(0, Ordering::Relaxed);
        self.echo_recvs.store(0, Ordering::Relaxed);
        self.echo_datagrams.store(0, Ordering::Relaxed);
        self.requests.store(0, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// CALC -- identical across every variant. memcpy a 512B buffer + xor-fold
// transform. The pure cost is calibrated once uncontended (see
// calibrate_calc_ns); under load we just call `calc`.
// ---------------------------------------------------------------------------

#[inline(always)]
fn calc(src: &[u8], dst: &mut [u8]) -> u8 {
    let n = FRAME.min(src.len()).min(dst.len());
    dst[..n].copy_from_slice(&src[..n]);
    let mut acc: u8 = 0;
    for b in &dst[..n] {
        acc ^= *b;
    }
    // touch the buffer so the xor-fold can't be optimized away
    if n > 0 {
        dst[0] = dst[0].wrapping_add(acc);
    }
    acc
}

// Pure calc cost, measured ONCE uncontended at startup (a tight loop on one
// idle core). We deliberately do NOT time calc inline under load: inline timing
// captures scheduler preemption + cache contention, not the work itself, and
// would misreport a 512B memcpy as microseconds. The honest attribution is
// "what does the work cost when nothing competes" vs "what the loaded p50 is".
fn calibrate_calc_ns() -> f64 {
    let src = vec![0xA5u8; FRAME];
    let mut dst = vec![0u8; FRAME];
    // warm caches
    for _ in 0..10_000 {
        std::hint::black_box(calc(std::hint::black_box(&src), std::hint::black_box(&mut dst)));
    }
    let iters = 1_000_000u64;
    let t0 = Instant::now();
    for _ in 0..iters {
        std::hint::black_box(calc(std::hint::black_box(&src), std::hint::black_box(&mut dst)));
    }
    t0.elapsed().as_nanos() as f64 / iters as f64
}

// ---------------------------------------------------------------------------
// Wire protocol
//   client request frame  : [u32 len][len bytes payload]
//   payload first 16 bytes: [u64 client_send_ns][u32 conn_token][u32 _]
//   echo datagram (32B)   : [u32 reactor_idx][u32 conn_token][u64 send_ns]
//                            [u64 client_send_ns][u64 _]
// The client_send_ns rides all the way to the echo and back so the tile's
// echo-recv thread can hand a fully-formed response back to the reactor.
// ---------------------------------------------------------------------------

#[inline]
fn put_u32(buf: &mut [u8], at: usize, v: u32) {
    buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

#[inline]
fn put_u64(buf: &mut [u8], at: usize, v: u64) {
    buf[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

#[inline]
fn get_u32(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(buf[at..at + 4].try_into().unwrap())
}

#[inline]
fn get_u64(buf: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(buf[at..at + 8].try_into().unwrap())
}

// ---------------------------------------------------------------------------
// Pinning helpers
// ---------------------------------------------------------------------------

fn core_ids() -> Vec<CoreId> {
    core_affinity::get_core_ids().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// RLIMIT_NOFILE: raise to the hard limit so we can attempt 10k conns.
// ---------------------------------------------------------------------------

fn raise_nofile() -> u64 {
    unsafe {
        let mut rl = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rl) != 0 {
            return 0;
        }
        rl.rlim_cur = rl.rlim_max;
        let _ = libc::setrlimit(libc::RLIMIT_NOFILE, &rl);
        let mut after = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        let _ = libc::getrlimit(libc::RLIMIT_NOFILE, &mut after);
        after.rlim_cur
    }
}

// Bump SO_RCVBUF/SO_SNDBUF so the echo + batch sockets don't drop datagrams
// under burst on loopback (a dropped reply wedges the closed-loop poll). The
// kernel doubles + clamps to net.core.{r,w}mem_max; best-effort, ignore errors.
fn set_sockbufs(fd: i32, bytes: libc::c_int) {
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            &bytes as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            &bytes as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
}

// ===========================================================================
// ECHO SERVICE -- one std-UDP thread on its own core, in EVERY variant.
// recv a datagram, send it straight back to the sender. Constant downstream.
// ===========================================================================

fn spawn_echo(core: CoreId, stop: Arc<AtomicBool>) -> (SocketAddr, JoinHandle<()>) {
    let sock = UdpSocket::bind("127.0.0.1:0").expect("echo bind");
    let addr = sock.local_addr().expect("echo addr");
    // Large socket buffers + non-blocking busy-drain: the echo stands in for a
    // downstream that keeps pace, so it must NOT drop datagrams under burst
    // (the batched variant fires up to BATCH datagrams in one sendmmsg). A
    // dropped reply would wedge the closed-loop. Identical echo for all variants.
    set_sockbufs(sock.as_raw_fd(), 16 * 1024 * 1024);
    sock.set_nonblocking(true).ok();
    let handle = thread::Builder::new()
        .name("lab-echo".into())
        .spawn(move || {
            core_affinity::set_for_current(core);
            let mut buf = [0u8; ECHO_PAYLOAD];
            while !stop.load(Ordering::Relaxed) {
                match sock.recv_from(&mut buf) {
                    Ok((n, from)) => {
                        let _ = sock.send_to(&buf[..n], from);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::hint::spin_loop();
                    }
                    Err(_) => std::hint::spin_loop(),
                }
            }
        })
        .expect("spawn echo");
    (addr, handle)
}

// ===========================================================================
// LOAD GENERATOR -- closed-loop. A pool of TCP conns split across `gen_cores`
// generator threads; each thread drives its conns with `pipeline` requests in
// flight, stamps send-time, and records round-trip on completion.
// Returns (histogram, completed, achieved_conns).
// ===========================================================================

struct LoadResult {
    hist: Histogram<u64>,
    completed: u64,
    conns: usize,
    window: Duration,
}

#[allow(clippy::too_many_arguments)]
fn run_load(
    server_addr: SocketAddr,
    target_conns: usize,
    pipeline: usize,
    open_rate: u64,
    samples_target: u64,
    warmup: Duration,
    duration_cap: Duration,
    gen_cores: &[CoreId],
) -> LoadResult {
    // 1) Open connections (best effort up to target).
    let mut streams = Vec::with_capacity(target_conns);
    for _ in 0..target_conns {
        match TcpStream::connect(server_addr) {
            Ok(s) => {
                s.set_nodelay(true).ok();
                streams.push(s);
            }
            Err(_) => break, // hit fd / ephemeral-port limit; scale down + report
        }
    }
    let achieved = streams.len();
    if achieved == 0 {
        return LoadResult {
            hist: Histogram::new(3).unwrap(),
            completed: 0,
            conns: 0,
            window: Duration::from_secs(1),
        };
    }

    // 2) Partition conns across generator threads.
    let nthreads = gen_cores.len().max(1).min(achieved);
    let mut buckets: Vec<Vec<TcpStream>> = (0..nthreads).map(|_| Vec::new()).collect();
    for (i, s) in streams.into_iter().enumerate() {
        buckets[i % nthreads].push(s);
    }

    let record = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let total_completed = Arc::new(AtomicU64::new(0));
    let per_thread_target = samples_target / nthreads as u64 + 1;
    let start_barrier = Arc::new(Barrier::new(nthreads + 1));

    let mut handles = Vec::with_capacity(nthreads);
    for (tid, conns) in buckets.into_iter().enumerate() {
        let core = gen_cores[tid % gen_cores.len()];
        let record = record.clone();
        let stop = stop.clone();
        let total_completed = total_completed.clone();
        let start_barrier = start_barrier.clone();
        let handle = thread::Builder::new()
            .name(format!("lab-gen-{tid}"))
            .spawn(move || {
                core_affinity::set_for_current(core);
                drive_conns(
                    conns,
                    pipeline,
                    open_rate,
                    record,
                    stop,
                    total_completed,
                    per_thread_target,
                    start_barrier,
                )
            })
            .expect("spawn gen");
        handles.push(handle);
    }

    // 3) Warm up, then flip recording on; stop on sample target or duration cap.
    start_barrier.wait();
    thread::sleep(warmup);
    record.store(true, Ordering::Relaxed);
    let t0 = Instant::now();
    loop {
        thread::sleep(Duration::from_millis(5));
        let done = total_completed.load(Ordering::Relaxed);
        let elapsed = t0.elapsed();
        if done >= samples_target {
            break;
        }
        if !duration_cap.is_zero() && elapsed >= duration_cap {
            break;
        }
        if elapsed >= Duration::from_secs(30) {
            break; // safety valve
        }
    }
    let window = t0.elapsed();
    stop.store(true, Ordering::Relaxed);

    // 4) Merge per-thread histograms.
    let mut hist = Histogram::<u64>::new_with_bounds(1, 60_000_000, 3).unwrap();
    let mut completed = 0u64;
    for h in handles {
        if let Ok((th, c)) = h.join() {
            hist.add(&th).ok();
            completed += c;
        }
    }
    LoadResult { hist, completed, conns: achieved, window }
}

fn drive_conns(
    mut conns: Vec<TcpStream>,
    pipeline: usize,
    open_rate: u64,
    record: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    total_completed: Arc<AtomicU64>,
    _per_thread_target: u64,
    start_barrier: Arc<Barrier>,
) -> (Histogram<u64>, u64) {
    let mut hist = Histogram::<u64>::new_with_bounds(1, 60_000_000, 3).unwrap();
    let mut completed = 0u64;
    let n = conns.len();
    if n == 0 {
        start_barrier.wait();
        return (hist, 0);
    }

    // Non-blocking sockets + a hand-rolled poll over the conn set keeps one
    // generator thread driving many conns without one slow conn blocking it.
    for s in &conns {
        s.set_nonblocking(true).ok();
    }
    let mut send_buf = vec![0u8; 4 + FRAME];
    put_u32(&mut send_buf, 0, FRAME as u32);
    let mut recv_bufs: Vec<Vec<u8>> = (0..n).map(|_| vec![0u8; 4 + FRAME]).collect();
    let mut recv_have: Vec<usize> = vec![0; n];
    let mut in_flight: Vec<usize> = vec![0; n];

    let open_loop = open_rate > 0;
    // OPEN-LOOP: per-conn inter-arrival gap in ns; next scheduled fire per conn.
    let gap_ns: u64 = if open_loop { 1_000_000_000 / open_rate } else { 0 };
    let mut next_fire: Vec<u64> = vec![0; n];

    start_barrier.wait();
    if open_loop {
        // stagger each conn's first fire across one gap so the thread doesn't
        // emit all n at the same instant.
        let base = now_ns();
        for (i, slot) in next_fire.iter_mut().enumerate().take(n) {
            *slot = base + (gap_ns / n.max(1) as u64) * i as u64;
        }
    } else {
        // CLOSED-LOOP: prime each conn to `pipeline` in-flight requests.
        for i in 0..n {
            for _ in 0..pipeline {
                if send_request(&mut conns[i], &mut send_buf).is_ok() {
                    in_flight[i] += 1;
                }
            }
        }
    }

    let mut local_flush = 0u64;
    while !stop.load(Ordering::Relaxed) {
        for i in 0..n {
            // OPEN-LOOP: fire every gap_ns regardless of completions, stamping
            // the SCHEDULED time so RTT = recv - scheduled (no coordinated
            // omission). Catch up if we fell behind (bounded per turn).
            if open_loop {
                let now = now_ns();
                let mut fires = 0;
                while next_fire[i] <= now && fires < 64 {
                    let sched = next_fire[i];
                    put_u32(&mut send_buf, 0, FRAME as u32);
                    put_u64(&mut send_buf, 4, sched); // stamp = SCHEDULED time
                    // open-loop write_all: finish the frame (closed-loop's
                    // skip-on-WouldBlock would distort a fixed-rate model).
                    if write_full(&mut conns[i], &send_buf).is_ok() {
                        in_flight[i] += 1;
                    }
                    next_fire[i] += gap_ns;
                    fires += 1;
                }
            }
            // drain any complete responses on conn i
            loop {
                let want = 4 + FRAME;
                let have = recv_have[i];
                let buf = &mut recv_bufs[i];
                match conns[i].read(&mut buf[have..want]) {
                    Ok(0) => break, // closed
                    Ok(k) => {
                        recv_have[i] += k;
                        if recv_have[i] >= want {
                            let now = now_ns();
                            let sent = get_u64(&recv_bufs[i], 4);
                            recv_have[i] = 0;
                            in_flight[i] = in_flight[i].saturating_sub(1);
                            if record.load(Ordering::Relaxed) {
                                let rtt = now.saturating_sub(sent).max(1);
                                hist.record(rtt).ok();
                                completed += 1;
                                local_flush += 1;
                                if local_flush >= 256 {
                                    total_completed.fetch_add(local_flush, Ordering::Relaxed);
                                    local_flush = 0;
                                }
                            }
                            // CLOSED-LOOP: refill to keep pipeline depth constant.
                            // OPEN-LOOP: the schedule drives sends, not completion.
                            if !open_loop && send_request(&mut conns[i], &mut send_buf).is_ok() {
                                in_flight[i] += 1;
                            }
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
        }
        // tiny yield so the generator doesn't 100%-spin the whole box during
        // warm-up gaps; under load read() returns data and we never get here.
        std::hint::spin_loop();
    }
    if local_flush > 0 {
        total_completed.fetch_add(local_flush, Ordering::Relaxed);
    }
    for s in &conns {
        s.shutdown(Shutdown::Both).ok();
    }
    (hist, completed)
}

#[inline]
fn send_request(stream: &mut TcpStream, send_buf: &mut [u8]) -> std::io::Result<()> {
    let now = now_ns();
    put_u64(send_buf, 4, now); // client_send_ns at payload[0..8]
    // Nonblocking write. A PARTIAL write must NOT silently corrupt the in-flight
    // accounting (the old code dropped a half-emitted frame): spin-retry the
    // remaining tail until the whole frame is on the socket, bounded so one
    // wedged conn can't hang the generator. On genuine error -> propagate as a
    // real failure (the caller does NOT increment in_flight). If the very first
    // byte WouldBlocks (socket buf full, nothing sent) -> skip this refill.
    let mut off = 0usize;
    let total = send_buf.len();
    let mut spins = 0u32;
    while off < total {
        match stream.write(&send_buf[off..]) {
            Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "closed")),
            Ok(k) => off += k,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if off == 0 {
                    // nothing emitted yet: clean skip, frame never started.
                    return Err(std::io::Error::new(std::io::ErrorKind::WouldBlock, "wb"));
                }
                // mid-frame: must finish it or the stream desyncs. Bounded spin.
                spins += 1;
                if spins > 1_000_000 {
                    return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "stuck"));
                }
                std::hint::spin_loop();
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

// Open-loop full-frame write: unlike send_request (which may cleanly skip a
// refill on a full socket buffer), the fixed-rate model must emit every
// scheduled frame, so spin-finish the whole buffer. Bounded so a wedged conn
// can't hang the generator; a hard failure drops the frame (counted only as a
// non-increment of in_flight).
#[inline]
fn write_full(stream: &mut TcpStream, buf: &[u8]) -> std::io::Result<()> {
    let mut off = 0usize;
    let mut spins = 0u32;
    while off < buf.len() {
        match stream.write(&buf[off..]) {
            Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "closed")),
            Ok(k) => off += k,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                spins += 1;
                if spins > 1_000_000 {
                    return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "stuck"));
                }
                std::hint::spin_loop();
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[inline(always)]
fn now_ns() -> u64 {
    // CLOCK_MONOTONIC via Instant is fine but we want a shared epoch across
    // threads; Instant is process-global monotonic, so anchor to a base.
    static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let base = BASE.get_or_init(Instant::now);
    base.elapsed().as_nanos() as u64
}

// ===========================================================================
// VARIANT 1 + 4: monoio-sharded (and batched-syscall, same shape).
// N pinned current-thread monoio reactors; SO_REUSEPORT shards client conns;
// echo I/O on the reactor. A sleep(ZERO) drain-yield mirrors the casting loop.
// ===========================================================================

mod monoio_stub {
    use super::*;
    use monoio::io::AsyncReadRentExt;
    use monoio::io::AsyncWriteRentExt;
    use monoio::net::ListenerOpts;
    use monoio::net::TcpListener;
    use monoio::net::TcpStream as MonoStream;
    use monoio::net::udp::UdpSocket as MonoUdp;
    use std::rc::Rc;

    pub fn spawn_reactors(
        bind: SocketAddr,
        echo: SocketAddr,
        n: usize,
        cores: &[CoreId],
        counters: Arc<Counters>,
        stop: Arc<AtomicBool>,
    ) -> Vec<JoinHandle<()>> {
        let mut handles = Vec::with_capacity(n);
        for (i, &core) in cores.iter().enumerate().take(n) {
            let counters = counters.clone();
            let stop = stop.clone();
            let handle = thread::Builder::new()
                .name(format!("lab-monoio-{i}"))
                .spawn(move || {
                    core_affinity::set_for_current(core);
                    let mut rt = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
                        .enable_timer()
                        .build()
                        .expect("monoio rt");
                    rt.block_on(reactor_main(bind, echo, counters, stop));
                })
                .expect("spawn monoio reactor");
            handles.push(handle);
        }
        handles
    }

    async fn reactor_main(
        bind: SocketAddr,
        echo: SocketAddr,
        counters: Arc<Counters>,
        stop: Arc<AtomicBool>,
    ) {
        let opts = ListenerOpts::new().reuse_port(true).reuse_addr(true);
        let listener = TcpListener::bind_with_config(bind, &opts).expect("monoio listen");
        // echo UDP socket owned by this reactor
        let udp = MonoUdp::bind("127.0.0.1:0").expect("monoio udp bind");
        udp.connect(echo).await.expect("monoio udp connect");
        let udp = Rc::new(udp);

        loop {
            // accept-loop runs concurrently with serving; bounded by sleep(ZERO).
            monoio::select! {
                res = listener.accept() => {
                    if let Ok((stream, _peer)) = res {
                        let counters = counters.clone();
                        let stop = stop.clone();
                        let udp = udp.clone();
                        monoio::spawn(serve_conn(stream, udp, counters, stop));
                    }
                }
                _ = monoio::time::sleep(Duration::from_millis(1)) => {}
            }
            if stop.load(Ordering::Relaxed) {
                break;
            }
            monoio::time::sleep(Duration::ZERO).await;
        }
    }

    async fn serve_conn(
        mut stream: MonoStream,
        udp: Rc<MonoUdp>,
        counters: Arc<Counters>,
        stop: Arc<AtomicBool>,
    ) {
        stream.set_nodelay(true).ok();
        let mut scratch = vec![0u8; FRAME];
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            // read length prefix
            let lenbuf = vec![0u8; 4];
            let (r, lenbuf) = stream.read_exact(lenbuf).await;
            if r.is_err() || r.unwrap() == 0 {
                break;
            }
            let len = get_u32(&lenbuf, 0) as usize;
            let payload = vec![0u8; len];
            let (r, payload) = stream.read_exact(payload).await;
            if r.is_err() {
                break;
            }
            // CALC
            calc(&payload, &mut scratch);
            let client_send_ns = get_u64(&payload, 0);

            // SUBMIT to echo + RECV echo (one sendto + one recvfrom)
            echo_single(&udp, client_send_ns, &counters).await;

            // SEND UP: length-prefixed response (echo the original payload)
            let mut resp = vec![0u8; 4 + len];
            put_u32(&mut resp, 0, len as u32);
            resp[4..4 + len].copy_from_slice(&payload);
            let (w, _resp) = stream.write_all(resp).await;
            if w.is_err() {
                break;
            }
            counters.requests.fetch_add(1, Ordering::Relaxed);
        }
    }

    async fn echo_single(udp: &MonoUdp, client_send_ns: u64, counters: &Counters) {
        let mut out = vec![0u8; ECHO_PAYLOAD];
        put_u64(&mut out, 8, client_send_ns);
        let (s, _out) = udp.send(out).await;
        counters.echo_sends.fetch_add(1, Ordering::Relaxed);
        if s.is_ok() {
            counters.echo_datagrams.fetch_add(1, Ordering::Relaxed);
        }
        let inb = vec![0u8; ECHO_PAYLOAD];
        let (r, _inb) = udp.recv(inb).await;
        counters.echo_recvs.fetch_add(1, Ordering::Relaxed);
        let _ = r;
    }
}

// ===========================================================================
// VARIANT 2: tokio multi-thread runtime, N worker threads.
// ===========================================================================

mod tokio_stub {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;
    use tokio::net::TcpStream as TokStream;
    use tokio::net::UdpSocket as TokUdp;

    pub fn spawn_runtime(
        bind: SocketAddr,
        echo: SocketAddr,
        n: usize,
        cores: Vec<CoreId>,
        counters: Arc<Counters>,
        stop: Arc<AtomicBool>,
    ) -> JoinHandle<()> {
        thread::Builder::new()
            .name("lab-tokio-host".into())
            .spawn(move || {
                let pin_cores = Arc::new(Mutex::new(cores));
                let pin_idx = Arc::new(AtomicU64::new(0));
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(n)
                    .enable_all()
                    .on_thread_start(move || {
                        let idx = pin_idx.fetch_add(1, Ordering::Relaxed) as usize;
                        let guard = pin_cores.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(c) = guard.get(idx % guard.len().max(1)) {
                            core_affinity::set_for_current(*c);
                        }
                    })
                    .build()
                    .expect("tokio rt");
                rt.block_on(serve(bind, echo, counters, stop));
            })
            .expect("spawn tokio host")
    }

    async fn serve(
        bind: SocketAddr,
        echo: SocketAddr,
        counters: Arc<Counters>,
        stop: Arc<AtomicBool>,
    ) {
        let listener = TcpListener::bind(bind).await.expect("tokio listen");
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let accept = tokio::time::timeout(Duration::from_millis(2), listener.accept()).await;
            if let Ok(Ok((stream, _))) = accept {
                let counters = counters.clone();
                let stop = stop.clone();
                tokio::spawn(serve_conn(stream, echo, counters, stop));
            }
        }
    }

    async fn serve_conn(
        mut stream: TokStream,
        echo: SocketAddr,
        counters: Arc<Counters>,
        stop: Arc<AtomicBool>,
    ) {
        stream.set_nodelay(true).ok();
        let udp = match TokUdp::bind("127.0.0.1:0").await {
            Ok(u) => u,
            Err(_) => return,
        };
        if udp.connect(echo).await.is_err() {
            return;
        }
        let mut scratch = vec![0u8; FRAME];
        let mut lenbuf = [0u8; 4];
        let mut out = [0u8; ECHO_PAYLOAD];
        let mut inb = [0u8; ECHO_PAYLOAD];
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            if stream.read_exact(&mut lenbuf).await.is_err() {
                break;
            }
            let len = get_u32(&lenbuf, 0) as usize;
            let mut payload = vec![0u8; len];
            if stream.read_exact(&mut payload).await.is_err() {
                break;
            }
            calc(&payload, &mut scratch);
            let client_send_ns = get_u64(&payload, 0);

            put_u64(&mut out, 8, client_send_ns);
            let _ = udp.send(&out).await;
            counters.echo_sends.fetch_add(1, Ordering::Relaxed);
            counters.echo_datagrams.fetch_add(1, Ordering::Relaxed);
            let _ = udp.recv(&mut inb).await;
            counters.echo_recvs.fetch_add(1, Ordering::Relaxed);

            let mut resp = vec![0u8; 4 + len];
            put_u32(&mut resp, 0, len as u32);
            resp[4..4 + len].copy_from_slice(&payload);
            if stream.write_all(&resp).await.is_err() {
                break;
            }
            counters.requests.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// ===========================================================================
// VARIANT 3: busy-spin-tile.
// N pinned monoio reactors own client conns (read+write) and DIRECT-send each
// echo submit. ONE dedicated pinned thread busy-spins recv on the shared echo
// reply socket and routes each reply to the owning reactor over a per-reactor
// rtrb SPSC ring. The reactor drains its ring and completes the response.
//
// Routing data rides in the echo datagram: [reactor_idx][conn_token][...].
// Here the conn_token is the coroutine's wakeable slot; we model the handoff
// with a per-reactor "pending response" map keyed by a monotonic request id.
// To stay faithful AND simple, the reactor blocks awaiting its ring slot via a
// notify; the spin thread pushes (req_id, client_send_ns). We keep the submit
// as a direct send from the reactor (1 sendto) and centralize ONLY the recv.
// ===========================================================================

mod tile_stub {
    use super::*;
    use monoio::io::AsyncReadRentExt;
    use monoio::io::AsyncWriteRentExt;
    use monoio::net::ListenerOpts;
    use monoio::net::TcpListener;
    use monoio::net::TcpStream as MonoStream;
    use rtrb::Consumer;
    use rtrb::Producer;
    use rtrb::RingBuffer;
    use std::cell::RefCell;
    use std::rc::Rc;

    // What the spin thread routes back to a reactor: the request id it can use
    // to wake the right pending coroutine, plus the stamp (already in payload).
    #[derive(Clone, Copy)]
    pub struct EchoReply {
        pub req_id: u64,
    }

    // Per-reactor shared state: the ring consumer + a slab of "done" flags the
    // coroutines poll. rtrb is SPSC: spin-thread = producer, reactor = consumer.
    pub struct ReactorInbox {
        pub consumer: RefCell<Consumer<EchoReply>>,
        pub done: RefCell<rustc_hash::FxHashMap<u64, bool>>,
    }

    pub fn spawn_tile(
        bind: SocketAddr,
        echo: SocketAddr,
        n: usize,
        stub_cores: &[CoreId],
        spin_core: CoreId,
        counters: Arc<Counters>,
        stop: Arc<AtomicBool>,
    ) -> Vec<JoinHandle<()>> {
        // One UDP socket per reactor for SUBMIT (direct send from reactor).
        // One SHARED reply socket the spin thread owns for RECV. Reactors send
        // their submit to echo with their own reply-addr as the from, so echo
        // replies land on... no: echo replies to the sender. To centralize the
        // recv we instead have reactors send FROM a socket whose recv side is
        // drained by the spin thread. Simplest faithful model: each reactor
        // sends on its own submit socket; the spin thread owns N recv sockets?
        // That is N spun recvs, not one. To keep ONE spun recv socket we route
        // all submits through a single shared UDP socket the spin thread reads,
        // and reactors send on it via a cloned fd. The reactor_idx in the
        // payload tells the spin thread which ring to push to.
        let reply_sock = UdpSocket::bind("127.0.0.1:0").expect("tile reply bind");
        reply_sock.connect(echo).expect("tile connect echo");
        set_sockbufs(reply_sock.as_raw_fd(), 16 * 1024 * 1024);
        let reply_fd = reply_sock.as_raw_fd();

        // Build per-reactor rings. The inbox (consumer + done-map) is owned by
        // exactly one reactor thread, so it moves by value (no Arc -- RefCell
        // isn't Sync, and we never share it across threads). The producer half
        // moves to the single spin thread.
        let mut producers: Vec<Producer<EchoReply>> = Vec::with_capacity(n);
        let mut inboxes: Vec<ReactorInbox> = Vec::with_capacity(n);
        for _ in 0..n {
            let (prod, cons) = RingBuffer::<EchoReply>::new(1 << 16);
            producers.push(prod);
            inboxes.push(ReactorInbox {
                consumer: RefCell::new(cons),
                done: RefCell::new(rustc_hash::FxHashMap::default()),
            });
        }

        let mut handles = Vec::with_capacity(n + 1);

        // Spin thread: busy-spin recv on the shared reply socket, route by idx.
        {
            let stop = stop.clone();
            let counters = counters.clone();
            // Move the std socket into the spin thread (it owns recv).
            let spin_sock = reply_sock;
            let h = thread::Builder::new()
                .name("lab-tile-spin".into())
                .spawn(move || {
                    core_affinity::set_for_current(spin_core);
                    spin_sock.set_nonblocking(true).ok();
                    let mut buf = [0u8; ECHO_PAYLOAD];
                    let mut prods = producers;
                    while !stop.load(Ordering::Relaxed) {
                        match spin_sock.recv(&mut buf) {
                            Ok(_n) => {
                                counters.echo_recvs.fetch_add(1, Ordering::Relaxed);
                                counters.echo_datagrams.fetch_add(1, Ordering::Relaxed);
                                let idx = get_u32(&buf, 0) as usize;
                                let req_id = get_u64(&buf, 8); // we stash req_id at [8..16]
                                if let Some(p) = prods.get_mut(idx) {
                                    // push_spin: bare busy retry, dedicated core
                                    while p.push(EchoReply { req_id }).is_err() {
                                        std::hint::spin_loop();
                                    }
                                }
                            }
                            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                std::hint::spin_loop();
                            }
                            Err(_) => std::hint::spin_loop(),
                        }
                    }
                })
                .expect("spawn spin");
            handles.push(h);
        }

        // Reactors. Each takes one inbox by value.
        let stub_cores: Vec<CoreId> = stub_cores[..n].to_vec();
        for (i, inbox) in inboxes.into_iter().enumerate() {
            let core = stub_cores[i];
            let counters = counters.clone();
            let stop = stop.clone();
            let h = thread::Builder::new()
                .name(format!("lab-tile-rx-{i}"))
                .spawn(move || {
                    core_affinity::set_for_current(core);
                    let mut rt = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
                        .enable_timer()
                        .build()
                        .expect("monoio rt");
                    rt.block_on(reactor_main(bind, i as u32, reply_fd, inbox, counters, stop));
                })
                .expect("spawn tile reactor");
            handles.push(h);
        }
        handles
    }

    async fn reactor_main(
        bind: SocketAddr,
        idx: u32,
        submit_fd: i32,
        inbox: ReactorInbox,
        counters: Arc<Counters>,
        stop: Arc<AtomicBool>,
    ) {
        let opts = ListenerOpts::new().reuse_port(true).reuse_addr(true);
        let listener = TcpListener::bind_with_config(bind, &opts).expect("tile listen");
        let req_seq = Rc::new(RefCell::new(0u64));
        let inbox = Rc::new(inbox);

        loop {
            monoio::select! {
                res = listener.accept() => {
                    if let Ok((stream, _peer)) = res {
                        let counters = counters.clone();
                        let stop = stop.clone();
                        let inbox = inbox.clone();
                        let seq = req_seq.clone();
                        monoio::spawn(serve_conn(stream, idx, submit_fd, seq, inbox, counters, stop));
                    }
                }
                _ = monoio::time::sleep(Duration::from_millis(1)) => {}
            }
            if stop.load(Ordering::Relaxed) {
                break;
            }
            // DRAIN the ring: pull every routed reply and mark its req done so
            // the waiting coroutines can complete. This is the tile's reactor
            // drain step (mirrors the casting recv drain).
            {
                let mut cons = inbox.consumer.borrow_mut();
                let mut done = inbox.done.borrow_mut();
                while let Ok(reply) = cons.pop() {
                    done.insert(reply.req_id, true);
                }
            }
            monoio::time::sleep(Duration::ZERO).await;
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn serve_conn(
        mut stream: MonoStream,
        idx: u32,
        submit_fd: i32,
        req_seq: Rc<RefCell<u64>>,
        inbox: Rc<ReactorInbox>,
        counters: Arc<Counters>,
        stop: Arc<AtomicBool>,
    ) {
        stream.set_nodelay(true).ok();
        let mut scratch = vec![0u8; FRAME];
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let lenbuf = vec![0u8; 4];
            let (r, lenbuf) = stream.read_exact(lenbuf).await;
            if r.is_err() || r.unwrap() == 0 {
                break;
            }
            let len = get_u32(&lenbuf, 0) as usize;
            let payload = vec![0u8; len];
            let (r, payload) = stream.read_exact(payload).await;
            if r.is_err() {
                break;
            }
            calc(&payload, &mut scratch);
            let client_send_ns = get_u64(&payload, 0);

            // assign a request id, SUBMIT direct send (1 sendto via raw fd)
            let req_id = {
                let mut s = req_seq.borrow_mut();
                *s += 1;
                // make ids unique per reactor by mixing idx in the high bits
                (*s) | ((idx as u64) << 48)
            };
            let mut out = [0u8; ECHO_PAYLOAD];
            put_u32(&mut out, 0, idx); // reactor_idx for routing
            put_u64(&mut out, 8, req_id); // req_id at [8..16]
            put_u64(&mut out, 16, client_send_ns);
            // direct sendto on the shared submit fd (kernel routes the reply to
            // the connected echo, echo replies to this socket -> spin thread).
            // A dropped submit (EAGAIN under burst) would leave the coroutine
            // awaiting a reply that never comes -> wedge the closed loop, so we
            // retry on WouldBlock. Count the SYSCALL each attempt (it happened);
            // credit a datagram only on the accepting call.
            loop {
                let r = unsafe {
                    libc::send(
                        submit_fd,
                        out.as_ptr() as *const libc::c_void,
                        out.len(),
                        libc::MSG_DONTWAIT,
                    )
                };
                counters.echo_sends.fetch_add(1, Ordering::Relaxed);
                if r >= 0 {
                    counters.echo_datagrams.fetch_add(1, Ordering::Relaxed);
                    break;
                }
                let err = std::io::Error::last_os_error();
                if err.kind() != std::io::ErrorKind::WouldBlock {
                    break; // hard error: give up this submit (reply won't come)
                }
                monoio::time::sleep(Duration::ZERO).await;
                if stop.load(Ordering::Relaxed) {
                    break;
                }
            }

            // AWAIT the routed reply: poll the per-reactor done-map, yielding.
            // The reactor's drain loop fills it from the SPSC ring.
            loop {
                {
                    let mut done = inbox.done.borrow_mut();
                    if done.remove(&req_id).is_some() {
                        break;
                    }
                }
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                monoio::time::sleep(Duration::ZERO).await;
            }

            let mut resp = vec![0u8; 4 + len];
            put_u32(&mut resp, 0, len as u32);
            resp[4..4 + len].copy_from_slice(&payload);
            let (w, _resp) = stream.write_all(resp).await;
            if w.is_err() {
                break;
            }
            counters.requests.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// ===========================================================================
// VARIANT 4: batched-syscall.
// Same shape as the busy-spin-tile (N monoio reactors own client conns), but
// the single dedicated batch thread amortizes the echo I/O across many
// requests with ONE sendmmsg and ONE recvmmsg per loop turn:
//   * Each reactor pushes its echo SUBMIT into a per-reactor SPSC submit ring
//     (no per-request sendto from the reactor).
//   * The batch thread drains all submit rings into a vector, fires them with
//     a SINGLE sendmmsg, then drains the echo socket with a SINGLE recvmmsg
//     (up to BATCH datagrams), and routes each reply to the owning reactor's
//     SPSC reply ring.
//   * Reactors drain their reply ring and complete the waiting coroutine.
// This isolates the syscall-amortization lever: echo_sends/echo_recvs count
// SYSCALLS (sendmmsg/recvmmsg calls), echo_datagrams counts datagrams moved,
// so syscalls-per-request drops well below 2 under load. Everything else
// (calc, frames, conns, offered load, echo service) is identical.
// ===========================================================================

mod batched_stub {
    use super::*;
    use monoio::io::AsyncReadRentExt;
    use monoio::io::AsyncWriteRentExt;
    use monoio::net::ListenerOpts;
    use monoio::net::TcpListener;
    use monoio::net::TcpStream as MonoStream;
    use rtrb::Consumer;
    use rtrb::Producer;
    use rtrb::RingBuffer;
    use std::cell::RefCell;
    use std::rc::Rc;

    const BATCH: usize = 64; // datagrams per sendmmsg/recvmmsg

    #[derive(Clone, Copy)]
    pub struct Submit {
        pub idx: u32,
        pub req_id: u64,
    }

    #[derive(Clone, Copy)]
    pub struct Reply {
        pub req_id: u64,
    }

    struct Inbox {
        reply_rx: RefCell<Consumer<Reply>>,
        done: RefCell<rustc_hash::FxHashMap<u64, bool>>,
        submit_tx: RefCell<Producer<Submit>>,
    }

    pub fn spawn_batched(
        bind: SocketAddr,
        echo: SocketAddr,
        n: usize,
        stub_cores: &[CoreId],
        batch_core: CoreId,
        counters: Arc<Counters>,
        stop: Arc<AtomicBool>,
    ) -> Vec<JoinHandle<()>> {
        let echo_sock = UdpSocket::bind("127.0.0.1:0").expect("batched echo bind");
        echo_sock.connect(echo).expect("batched connect echo");
        set_sockbufs(echo_sock.as_raw_fd(), 16 * 1024 * 1024);
        echo_sock.set_nonblocking(true).ok();
        let echo_fd = echo_sock.as_raw_fd();

        // per-reactor submit + reply rings (both SPSC)
        let mut submit_rx: Vec<Consumer<Submit>> = Vec::with_capacity(n);
        let mut reply_tx: Vec<Producer<Reply>> = Vec::with_capacity(n);
        let mut inboxes: Vec<Inbox> = Vec::with_capacity(n);
        for _ in 0..n {
            let (s_tx, s_rx) = RingBuffer::<Submit>::new(1 << 16);
            let (r_tx, r_rx) = RingBuffer::<Reply>::new(1 << 16);
            submit_rx.push(s_rx);
            reply_tx.push(r_tx);
            inboxes.push(Inbox {
                reply_rx: RefCell::new(r_rx),
                done: RefCell::new(rustc_hash::FxHashMap::default()),
                submit_tx: RefCell::new(s_tx),
            });
        }

        let mut handles = Vec::with_capacity(n + 1);

        // Batch thread: drain submits -> sendmmsg; recvmmsg -> route replies.
        {
            let stop = stop.clone();
            let counters = counters.clone();
            let h = thread::Builder::new()
                .name("lab-batch".into())
                .spawn(move || {
                    core_affinity::set_for_current(batch_core);
                    batch_loop(echo_sock, echo_fd, submit_rx, reply_tx, counters, stop);
                })
                .expect("spawn batch");
            handles.push(h);
        }

        // Reactors.
        let stub_cores: Vec<CoreId> = stub_cores[..n].to_vec();
        for (i, inbox) in inboxes.into_iter().enumerate() {
            let core = stub_cores[i];
            let counters = counters.clone();
            let stop = stop.clone();
            let h = thread::Builder::new()
                .name(format!("lab-batch-rx-{i}"))
                .spawn(move || {
                    core_affinity::set_for_current(core);
                    let mut rt = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
                        .enable_timer()
                        .build()
                        .expect("monoio rt");
                    rt.block_on(reactor_main(bind, i as u32, inbox, counters, stop));
                })
                .expect("spawn batch reactor");
            handles.push(h);
        }
        handles
    }

    // The batch thread keeps a fixed pool of ECHO_PAYLOAD send/recv buffers and
    // mmsghdr/iovec arrays, reused every turn (zero alloc on the hot loop).
    fn batch_loop(
        echo_sock: UdpSocket,
        echo_fd: i32,
        mut submit_rx: Vec<Consumer<Submit>>,
        mut reply_tx: Vec<Producer<Reply>>,
        counters: Arc<Counters>,
        stop: Arc<AtomicBool>,
    ) {
        let _ = &echo_sock; // keep the socket alive for the fd's lifetime
        let mut send_bufs = vec![[0u8; ECHO_PAYLOAD]; BATCH];
        let mut recv_bufs = vec![[0u8; ECHO_PAYLOAD]; BATCH];
        let n = submit_rx.len();
        let mut rr = 0usize; // round-robin start across submit rings for fairness
        // tiny backlog for submits a partial sendmmsg didn't accept (re-flushed
        // promptly next turn so no request is silently dropped).
        let mut backlog: Vec<Submit> = Vec::with_capacity(BATCH);

        while !stop.load(Ordering::Relaxed) {
            // 1) Drain up to BATCH submits across the reactor rings.
            let mut pending: Vec<Submit> = Vec::with_capacity(BATCH);
            for step in 0..n {
                let ring = (rr + step) % n;
                while pending.len() < BATCH {
                    match submit_rx[ring].pop() {
                        Ok(s) => pending.push(s),
                        Err(_) => break,
                    }
                }
                if pending.len() >= BATCH {
                    break;
                }
            }
            rr = (rr + 1) % n;

            // 2) sendmmsg: ONE syscall for whatever we drained. PROMPT PARTIAL
            //    FLUSH -- we do NOT wait for the ring to fill to BATCH. Under
            //    pipeline=1 there is at most one in-flight submit per reactor,
            //    so a "wait for full batch" loop would deadlock (the batch
            //    never fills because reactors are blocked awaiting the reply).
            //    Sending whatever is ready keeps the pipe moving; batching only
            //    actually amortizes when in-flight depth (pipeline) is high,
            //    which the throughput/syscalls-per-req numbers then show.
            //    sendmmsg can return < m on EAGAIN/partial; count the syscall
            //    ALWAYS (it happened) but only credit the datagrams it accepted.
            let m = pending.len();
            if m > 0 {
                for (k, s) in pending.iter().enumerate() {
                    put_u32(&mut send_bufs[k], 0, s.idx);
                    put_u64(&mut send_bufs[k], 8, s.req_id);
                }
                let sent = sendmmsg_batch(echo_fd, &mut send_bufs[..m]);
                counters.echo_sends.fetch_add(1, Ordering::Relaxed); // ONE syscall (even on err)
                if sent > 0 {
                    counters.echo_datagrams.fetch_add(sent as u64, Ordering::Relaxed);
                }
                // If sendmmsg accepted fewer than m, re-queue the tail so those
                // requests are not silently lost (would wedge the closed loop).
                if (sent.max(0) as usize) < m {
                    for s in pending.iter().skip(sent.max(0) as usize) {
                        let ridx = s.idx as usize;
                        // best-effort retry next turn via the owning reactor's
                        // submit ring is not reachable here; instead resend
                        // directly next turns by pushing back onto our pending
                        // path -- simplest: drop into a tiny local backlog.
                        backlog.push(*s);
                        let _ = ridx;
                    }
                }
            }

            // 2b) flush any backlog from a prior partial send (prompt, bounded).
            if !backlog.is_empty() {
                let b = backlog.len().min(BATCH);
                for (k, s) in backlog.iter().take(b).enumerate() {
                    put_u32(&mut send_bufs[k], 0, s.idx);
                    put_u64(&mut send_bufs[k], 8, s.req_id);
                }
                let sent = sendmmsg_batch(echo_fd, &mut send_bufs[..b]);
                counters.echo_sends.fetch_add(1, Ordering::Relaxed);
                let ok = sent.max(0) as usize;
                if ok > 0 {
                    counters.echo_datagrams.fetch_add(ok as u64, Ordering::Relaxed);
                }
                backlog.drain(..ok.min(backlog.len()));
            }

            // 3) recvmmsg EVERY turn, independent of whether we just sent. This
            //    is THE fix for the throughput collapse: replies arrive a turn
            //    or two AFTER the send, by which point pending may be empty; if
            //    recv only ran when there were submits, undrained replies would
            //    wedge the closed loop at low depth. Drain unconditionally.
            let got = recvmmsg_batch(echo_fd, &mut recv_bufs[..BATCH]);
            if got > 0 {
                counters.echo_recvs.fetch_add(1, Ordering::Relaxed); // ONE syscall
                counters.echo_datagrams.fetch_add(got as u64, Ordering::Relaxed);
                for buf in recv_bufs.iter().take(got) {
                    let idx = get_u32(buf, 0) as usize;
                    let req_id = get_u64(buf, 8);
                    if let Some(p) = reply_tx.get_mut(idx) {
                        while p.push(Reply { req_id }).is_err() {
                            std::hint::spin_loop();
                        }
                    }
                }
            }
            if m == 0 && got == 0 && backlog.is_empty() {
                std::hint::spin_loop();
            }
        }
    }

    // sendmmsg over a connected UDP socket: msg_name = null (connected).
    fn sendmmsg_batch(fd: i32, bufs: &mut [[u8; ECHO_PAYLOAD]]) -> i32 {
        let m = bufs.len();
        let mut iovs: Vec<libc::iovec> = Vec::with_capacity(m);
        let mut msgs: Vec<libc::mmsghdr> = Vec::with_capacity(m);
        for buf in bufs.iter_mut() {
            iovs.push(libc::iovec {
                iov_base: buf.as_mut_ptr() as *mut libc::c_void,
                iov_len: ECHO_PAYLOAD,
            });
        }
        for iov in iovs.iter_mut() {
            let mut hdr: libc::msghdr = unsafe { std::mem::zeroed() };
            hdr.msg_iov = iov as *mut libc::iovec;
            hdr.msg_iovlen = 1;
            msgs.push(libc::mmsghdr { msg_hdr: hdr, msg_len: 0 });
        }
        // SAFETY: msgs/iovs live for the call; fd is a valid connected UDP
        // socket; m == msgs.len(). recvmmsg/sendmmsg are POSIX-shaped.
        unsafe { libc::sendmmsg(fd, msgs.as_mut_ptr(), m as libc::c_uint, libc::MSG_DONTWAIT) }
    }

    // recvmmsg draining up to bufs.len() datagrams in one syscall.
    fn recvmmsg_batch(fd: i32, bufs: &mut [[u8; ECHO_PAYLOAD]]) -> usize {
        let m = bufs.len();
        let mut iovs: Vec<libc::iovec> = Vec::with_capacity(m);
        let mut msgs: Vec<libc::mmsghdr> = Vec::with_capacity(m);
        for buf in bufs.iter_mut() {
            iovs.push(libc::iovec {
                iov_base: buf.as_mut_ptr() as *mut libc::c_void,
                iov_len: ECHO_PAYLOAD,
            });
        }
        for iov in iovs.iter_mut() {
            let mut hdr: libc::msghdr = unsafe { std::mem::zeroed() };
            hdr.msg_iov = iov as *mut libc::iovec;
            hdr.msg_iovlen = 1;
            msgs.push(libc::mmsghdr { msg_hdr: hdr, msg_len: 0 });
        }
        // SAFETY: as above; MSG_DONTWAIT so the call returns immediately with
        // however many datagrams are queued (0 if none).
        let r = unsafe {
            libc::recvmmsg(
                fd,
                msgs.as_mut_ptr(),
                m as libc::c_uint,
                libc::MSG_DONTWAIT,
                std::ptr::null_mut(),
            )
        };
        if r <= 0 {
            0
        } else {
            r as usize
        }
    }

    async fn reactor_main(
        bind: SocketAddr,
        idx: u32,
        inbox: Inbox,
        counters: Arc<Counters>,
        stop: Arc<AtomicBool>,
    ) {
        let opts = ListenerOpts::new().reuse_port(true).reuse_addr(true);
        let listener = TcpListener::bind_with_config(bind, &opts).expect("batched listen");
        let inbox = Rc::new(inbox);
        let req_seq = Rc::new(RefCell::new(0u64));

        loop {
            monoio::select! {
                res = listener.accept() => {
                    if let Ok((stream, _peer)) = res {
                        let counters = counters.clone();
                        let stop = stop.clone();
                        let inbox = inbox.clone();
                        let seq = req_seq.clone();
                        monoio::spawn(serve_conn(stream, idx, seq, inbox, counters, stop));
                    }
                }
                _ = monoio::time::sleep(Duration::from_millis(1)) => {}
            }
            if stop.load(Ordering::Relaxed) {
                break;
            }
            // drain reply ring -> done-map
            {
                let mut rx = inbox.reply_rx.borrow_mut();
                let mut done = inbox.done.borrow_mut();
                while let Ok(reply) = rx.pop() {
                    done.insert(reply.req_id, true);
                }
            }
            monoio::time::sleep(Duration::ZERO).await;
        }
    }

    async fn serve_conn(
        mut stream: MonoStream,
        idx: u32,
        req_seq: Rc<RefCell<u64>>,
        inbox: Rc<Inbox>,
        counters: Arc<Counters>,
        stop: Arc<AtomicBool>,
    ) {
        stream.set_nodelay(true).ok();
        let mut scratch = vec![0u8; FRAME];
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let lenbuf = vec![0u8; 4];
            let (r, lenbuf) = stream.read_exact(lenbuf).await;
            if r.is_err() || r.unwrap() == 0 {
                break;
            }
            let len = get_u32(&lenbuf, 0) as usize;
            let payload = vec![0u8; len];
            let (r, payload) = stream.read_exact(payload).await;
            if r.is_err() {
                break;
            }
            calc(&payload, &mut scratch);

            let req_id = {
                let mut s = req_seq.borrow_mut();
                *s += 1;
                (*s) | ((idx as u64) << 48)
            };
            // SUBMIT into the per-reactor submit ring (batch thread sendmmsg's).
            loop {
                {
                    let mut tx = inbox.submit_tx.borrow_mut();
                    if tx.push(Submit { idx, req_id }).is_ok() {
                        break;
                    }
                }
                monoio::time::sleep(Duration::ZERO).await;
            }
            // AWAIT routed reply via the done-map (filled from reply ring).
            loop {
                {
                    let mut done = inbox.done.borrow_mut();
                    if done.remove(&req_id).is_some() {
                        break;
                    }
                }
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                monoio::time::sleep(Duration::ZERO).await;
            }

            let mut resp = vec![0u8; 4 + len];
            put_u32(&mut resp, 0, len as u32);
            resp[4..4 + len].copy_from_slice(&payload);
            let (w, _resp) = stream.write_all(resp).await;
            if w.is_err() {
                break;
            }
            counters.requests.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// ===========================================================================
// Driver: build SUT for one (variant, N), run load, print rows.
// ===========================================================================

struct Row {
    variant: Variant,
    k: usize,        // total core budget for this variant (reactors + helper)
    reactors: usize, // reactor/worker threads
    helper: usize,   // dedicated helper threads (spin/batch); 0 for monoio/tokio
    p50: u64,
    p99: u64,
    p999: u64,
    max: u64,
    throughput: f64,
    conns: usize,
    calc_ns: f64,
    syscalls_per_req: f64,
    service_us: f64, // per-core busy time per completed request (K / throughput)
}

// Split a core budget K into (reactors, helpers) per variant. monoio/tokio
// spend all K on reactors/workers; tile/batched spend K-1 on reactors and 1 on
// the always-hot helper -- so every variant occupies exactly K SUT cores.
fn budget_split(variant: Variant, k: usize) -> (usize, usize) {
    match variant {
        Variant::MonoioSharded | Variant::Tokio => (k, 0),
        Variant::BusySpinTile | Variant::BatchedSyscall => (k.saturating_sub(1).max(1), 1),
    }
}

fn run_variant(variant: Variant, k: usize, cfg: &Cfg, layout: &CoreLayout, calc_ns: f64) -> Row {
    let counters = Arc::new(Counters::default());
    counters.reset();
    let stop = Arc::new(AtomicBool::new(false));

    // echo on its own core in every variant (constant downstream, outside K)
    let echo_stop = Arc::new(AtomicBool::new(false));
    let (echo_addr, echo_handle) = spawn_echo(layout.echo, echo_stop.clone());

    // bind address for the stub (SO_REUSEPORT shards across reactors)
    let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
    // We need a FIXED port so clients can connect; bind one listener first to
    // claim a port, then let reactors reuse it. Claim via a throwaway std
    // listener with SO_REUSEPORT, read its port, drop it (port stays usable).
    let claim = bind_reuseport_listener(bind);
    let server_addr = claim.local_addr().unwrap();
    drop(claim); // reactors re-bind the same port with SO_REUSEPORT

    let (reactors, helper) = budget_split(variant, k);
    // The SUT draws ALL its threads from the same K-core slice of the pool:
    // reactors take pool[0..reactors]; the helper (if any) takes pool[reactors]
    // -- i.e. pool[reactors] == pool[k-1]. No variant gets a core outside [0,k).
    let pool = &layout.pool;
    let reactor_cores = &pool[..reactors.min(pool.len())];
    let helper_core = pool[(reactors).min(pool.len().saturating_sub(1))];

    let server_handles: Vec<JoinHandle<()>> = match variant {
        Variant::MonoioSharded => monoio_stub::spawn_reactors(
            server_addr, echo_addr, reactors, reactor_cores, counters.clone(), stop.clone(),
        ),
        Variant::Tokio => vec![tokio_stub::spawn_runtime(
            server_addr, echo_addr, reactors, reactor_cores.to_vec(), counters.clone(), stop.clone(),
        )],
        Variant::BusySpinTile => tile_stub::spawn_tile(
            server_addr, echo_addr, reactors, reactor_cores, helper_core, counters.clone(),
            stop.clone(),
        ),
        Variant::BatchedSyscall => batched_stub::spawn_batched(
            server_addr, echo_addr, reactors, reactor_cores, helper_core, counters.clone(),
            stop.clone(),
        ),
    };

    // give reactors a moment to bind their listeners
    thread::sleep(Duration::from_millis(200));

    let load = run_load(
        server_addr,
        cfg.conns,
        cfg.pipeline,
        cfg.open_rate,
        cfg.samples,
        Duration::from_millis(cfg.warmup_ms),
        Duration::from_millis(cfg.duration_ms),
        &layout.gen,
    );

    // tear down stub + echo
    stop.store(true, Ordering::Relaxed);
    thread::sleep(Duration::from_millis(50));
    echo_stop.store(true, Ordering::Relaxed);
    for h in server_handles {
        // monoio/tokio reactors break their loops on stop; join with a guard
        let _ = h.join();
    }
    let _ = echo_handle.join();

    let sends = counters.echo_sends.load(Ordering::Relaxed);
    let recvs = counters.echo_recvs.load(Ordering::Relaxed);
    let reqs = counters.requests.load(Ordering::Relaxed).max(1);
    // ECHO-SIDE-ONLY syscalls/req = (send + recv CALLS) / requests. Non-batched
    // paths do 1 send + 1 recv => ~2.0. Batched counts sendmmsg/recvmmsg CALLS
    // (not datagrams) so it drops well < 2 under depth. NOT a total syscall
    // budget (excludes TCP/accept/epoll/io_uring/timers) -- a lever indicator.
    let syscalls_per_req = (sends + recvs) as f64 / reqs as f64;

    // throughput = completed round-trips / recorded window (warm-up excluded).
    let throughput = load.completed as f64 / load.window.as_secs_f64().max(1e-9);
    // per-core service estimate uses the FULL budget K (reactors + helper), so
    // tile/batched are charged for their always-hot helper core. K cores busy
    // for the whole window completing `completed` requests => K/throughput
    // core-seconds per request. Work-rate view, NOT response time.
    let service_us = if throughput > 0.0 { k as f64 / throughput * 1e6 } else { 0.0 };

    Row {
        variant,
        k,
        reactors,
        helper,
        p50: load.hist.value_at_quantile(0.50),
        p99: load.hist.value_at_quantile(0.99),
        p999: load.hist.value_at_quantile(0.999),
        max: load.hist.max(),
        throughput,
        conns: load.conns,
        calc_ns,
        syscalls_per_req,
        service_us,
    }
}

fn bind_reuseport_listener(addr: SocketAddr) -> TcpListener {
    // std TcpListener doesn't expose SO_REUSEPORT; set it via setsockopt on a
    // raw socket, then convert. We only need the port claim; reactors set
    // reuse_port via monoio ListenerOpts when they bind the same port.
    let listener = TcpListener::bind(addr).expect("claim listener");
    unsafe {
        let fd = listener.as_raw_fd();
        let one: libc::c_int = 1;
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEPORT,
            &one as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
    listener
}

// ---------------------------------------------------------------------------
// Core layout: EQUAL CORE BUDGET. The SUT gets a pool of `max_k` LOGICAL CPUs
// (the largest budget swept). Every variant draws its threads from this same
// pool, so the total SUT cores are identical across variants:
//   core 0              : echo service (constant downstream, OUTSIDE the budget)
//   cores 1..1+max_k    : the SUT budget pool. A variant with budget K uses the
//                         first K of these:
//                           monoio/tokio  -> K reactors/workers, no helper
//                           tile/batched  -> (K-1) reactors + 1 helper (the
//                                            K-th pool core is the spin/batch
//                                            thread, NOT a free extra core)
//   remaining           : load generators
// Pinning is to LOGICAL CPUs (SMT siblings / NUMA not accounted for; see the
// module CAVEATS). On a tight box (gen pool empty) generators share echo's core.
// ---------------------------------------------------------------------------

struct CoreLayout {
    echo: CoreId,
    pool: Vec<CoreId>, // SUT budget pool: max_k logical CPUs, shared by all variants
    gen: Vec<CoreId>,
}

fn plan_cores(max_k: usize) -> CoreLayout {
    let ids = core_ids();
    let total = ids.len().max(1);
    let echo = ids[0];
    // budget pool: cores 1..1+max_k (wrap if the box is small).
    let pool: Vec<CoreId> = (0..max_k).map(|i| ids[(1 + i) % total]).collect();
    let gen_start = 1 + max_k;
    let mut gen: Vec<CoreId> = (gen_start..total).map(|i| ids[i]).collect();
    if gen.is_empty() {
        // out of dedicated cores: generators share echo's core (loopback gen is
        // light relative to the SUT) and the last pool core.
        gen.push(ids[0]);
        if total > 1 {
            gen.push(ids[total - 1]);
        }
    }
    CoreLayout { echo, pool, gen }
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

fn print_tables(rows: &[Row], achieved_conns: usize, nofile: u64, cfg: &Cfg) {
    println!();
    println!("================================================================================");
    println!(" RSX loop-architecture load benchmark");
    println!("================================================================================");
    println!(
        " target conns={}  achieved={}  pipeline={}  RLIMIT_NOFILE={}  samples/cell={}",
        cfg.conns, achieved_conns, cfg.pipeline, nofile, cfg.samples
    );
    println!(" calc=512B memcpy+xor-fold   echo=1 UDP datagram round-trip (own core, outside K)");
    let mode = if cfg.open_rate > 0 {
        format!("OPEN-LOOP @ {} req/s/conn (RTT vs scheduled send)", cfg.open_rate)
    } else {
        format!("CLOSED-LOOP @ concurrency = conns x pipeline = {}", cfg.conns * cfg.pipeline)
    };
    println!(" load model: {mode}");
    println!(" EQUAL CORE BUDGET K: every variant occupies K logical CPUs (cores/split below)");
    println!();
    println!(" (A) ROUND-TRIP LATENCY  (us)  +  throughput");
    println!(
        " {:<16} {:>3} {:>11} {:>10} {:>10} {:>10} {:>10} {:>13} {:>7}",
        "variant", "K", "cores", "p50", "p99", "p999", "max", "req/s", "conns"
    );
    println!(" {}", "-".repeat(98));
    for r in rows {
        let split = if r.helper > 0 {
            format!("{}r+{}h", r.reactors, r.helper)
        } else {
            format!("{}r", r.reactors)
        };
        println!(
            " {:<16} {:>3} {:>11} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>13.0} {:>7}",
            r.variant.label(),
            r.k,
            split,
            r.p50 as f64 / 1000.0,
            r.p99 as f64 / 1000.0,
            r.p999 as f64 / 1000.0,
            r.max as f64 / 1000.0,
            r.throughput,
            r.conns,
        );
    }
    println!();
    println!(" (B) ATTRIBUTION  (where the time goes -- per completed request)");
    println!(
        " {:<16} {:>3} {:>10} {:>12} {:>16} {:>18}",
        "variant", "K", "calc-ns", "service-us", "echo-sys/req", "verdict"
    );
    println!(" {}", "-".repeat(80));
    for r in rows {
        // service-us = per-core busy time per request across the FULL budget K
        // (helper included). calc-ns is the pure work; (service-us - calc) is
        // dominated by the echo syscalls + client frame read/write + scheduling.
        let calc_us = r.calc_ns / 1000.0;
        let calc_frac = if r.service_us > 0.0 { calc_us / r.service_us } else { 0.0 };
        let verdict = if r.syscalls_per_req < 1.5 {
            "syscall-amortized"
        } else if calc_frac > 0.25 {
            "calc-significant"
        } else {
            "syscall-bound"
        };
        println!(
            " {:<16} {:>3} {:>10.1} {:>12.2} {:>16.2} {:>18}",
            r.variant.label(),
            r.k,
            r.calc_ns,
            r.service_us,
            r.syscalls_per_req,
            verdict,
        );
    }
    println!();
    println!(" cores: r=reactor/worker threads, h=dedicated helper (spin/batch). Every");
    println!("   variant sums to K logical CPUs; echo runs on its own core OUTSIDE K.");
    println!(" calc-ns: pure 512B memcpy+xor work (calibrated uncontended).");
    println!(" service-us: K x window / completed = per-core busy time per request (K incl.");
    println!("   the helper core -- tile/batched are charged for it, the original bench was not).");
    println!(" echo-sys/req: ECHO-SIDE-ONLY send+recv CALLS per request (incl. failures).");
    println!("   NOT a total syscall budget (no TCP/accept/epoll/io_uring/timers). A LEVER");
    println!("   indicator: ~2 => one send+recv/req; <<2 (batched) => syscall amortized.");
    println!("   For rigorous attribution run under `perf stat -e syscalls:*,context-switches`.");
    println!(" p50/p99 in (A): closed-loop = RESPONSE time at concurrency = conns x pipeline");
    println!("   (coordinated-omission caveat near saturation); open-loop = RTT vs scheduled");
    println!("   send. Compare only WITHIN a fixed K and a fixed load model.");
    println!();
    println!(" CAVEATS: synthetic calc + loopback UDP echo + single shared box; pinning is to");
    println!("   LOGICAL CPUs (SMT/NUMA not accounted). Absolute us are NOT production");
    println!("   latencies; the SHAPE + attribution are the point.");
    println!("================================================================================");
}

// ---------------------------------------------------------------------------
// Entry point (harness = false)
// ---------------------------------------------------------------------------

fn main() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("lab panic: {info}");
    }));

    let cfg = load_cfg();
    let nofile = raise_nofile();
    let max_k = cfg.ns.iter().copied().max().unwrap_or(2);
    let layout = plan_cores(max_k);
    let calc_ns = calibrate_calc_ns();

    eprintln!(
        "lab: cores total={} budget-pool={:?} gen={} calc-ns={:.1} nofile={} open_rate={}",
        core_ids().len(),
        layout.pool.iter().map(|c| c.id).collect::<Vec<_>>(),
        layout.gen.len(),
        calc_ns,
        nofile,
        cfg.open_rate,
    );

    let mut rows = Vec::new();
    let mut achieved = 0usize;
    for &variant in &cfg.variants {
        for &k in &cfg.ns {
            eprintln!("lab: running variant={} K={} ...", variant.label(), k);
            let row = run_variant(variant, k, &cfg, &layout, calc_ns);
            achieved = achieved.max(row.conns);
            rows.push(row);
            // brief cool-down so ports/fds fully release between cells
            thread::sleep(Duration::from_millis(300));
        }
    }

    print_tables(&rows, achieved, nofile, &cfg);
}
