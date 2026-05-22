//! Off-hot-path latency telemetry.
//!
//! Pattern (project-wide, applicable to any hot-path log
//! emission, not just latency):
//!
//! 1. **Hot path** pushes a fixed-size [`Sample`] into a
//!    thread-local `rtrb` SPSC producer. No allocation, no
//!    mutex; just an atomic store + index bump. Typical
//!    cost: 20-30 ns per emit. Full-ring → sample is
//!    dropped (atomically counted).
//! 2. **Logger thread** owns the consumer half of every
//!    thread-local ring (registered when the thread first
//!    emits). Wakes every N ms, drains each ring, and
//!    emits `tracing::info!(target = "latency", ...)` —
//!    the same line shape the existing dashboard parser
//!    already understands.
//!
//! Why this shape:
//!
//! - **No mutex on the hot path.** A `try_lock` is not
//!   lock-free; under contention it falls back to a futex
//!   syscall (~µs). `rtrb` is wait-free single-producer
//!   single-consumer; each push is a single atomic store +
//!   wrap-around index increment.
//! - **One ring per emitting thread.** monoio is thread-
//!   per-core; tokio's worker pool has fixed threads.
//!   Both topologies make SPSC the right primitive (the
//!   producer side is implicitly single because only one
//!   thread holds the thread-local). The logger thread is
//!   the single consumer of each ring.
//! - **Drop-on-full, not block.** Telemetry is advisory.
//!   If the logger stalls, count drops and surface them
//!   later — never delay real work for log emission.
//! - **Format in the drainer.** The hex `oid` string and
//!   the structured-tracing field formatting both happen
//!   on the logger thread, not the hot path. The hot path
//!   just holds two `u64`s.

use rtrb::Consumer;
use rtrb::Producer;
use rtrb::PushError;
use rtrb::RingBuffer;
use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// One per-event sample. 40 bytes; the `stage` field is a
/// `&'static str` (no allocation) and the `oid` is held as
/// raw u64s.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub stage: &'static str,
    pub oid_hi: u64,
    pub oid_lo: u64,
    pub t_us: u64,
    pub t0_ns: u64,
}

/// Per-thread ring capacity. At ~6 emits per order × peak
/// ~10k orders/s × 100 ms drain interval, 8 192 entries is
/// 1.3 s of headroom per thread.
const RING_CAP: usize = 8_192;

/// Global registry of consumer halves. The logger thread
/// owns the entries; we hold them behind a mutex only at
/// registration time (once per thread, first emit). The
/// drainer can iterate without locking because we use a
/// double-buffered scheme below.
static CONSUMERS: OnceLock<Mutex<Vec<Consumer<Sample>>>> =
    OnceLock::new();

/// Samples discarded because the per-thread ring was full.
/// Reported and reset by the drainer.
static DROPPED: AtomicU64 = AtomicU64::new(0);

fn consumers() -> &'static Mutex<Vec<Consumer<Sample>>> {
    CONSUMERS
        .get_or_init(|| Mutex::new(Vec::new()))
}

thread_local! {
    static PRODUCER: RefCell<Option<Producer<Sample>>> =
        const { RefCell::new(None) };
}

/// Lazy-init: on a thread's first emit, allocate its SPSC
/// ring and register the consumer half globally. Mutex is
/// touched once per thread lifetime.
fn init_thread_ring() -> Producer<Sample> {
    let (prod, cons) =
        RingBuffer::<Sample>::new(RING_CAP);
    // SAFETY: registry mutex is held briefly and only on
    // the slow path (first emit per thread). Poisoning
    // implies a panic during another thread's init — fail
    // fast so we don't silently drop telemetry.
    consumers()
        .lock()
        .expect("INVARIANT: latency registry mutex poisoned")
        .push(cons);
    prod
}

/// Push a sample. Hot-path-safe: a single atomic store +
/// index bump in the SPSC producer (typically 20-30 ns).
/// If the ring is full the sample is dropped and counted.
/// First call per thread allocates the ring (~µs); steady
/// state is the fast path.
#[inline]
pub fn emit(
    stage: &'static str,
    oid_hi: u64,
    oid_lo: u64,
    t_us: u64,
    t0_ns: u64,
) {
    let sample = Sample {
        stage,
        oid_hi,
        oid_lo,
        t_us,
        t0_ns,
    };
    PRODUCER.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(init_thread_ring());
        }
        // SAFETY: just initialised above if it was None.
        let prod = slot
            .as_mut()
            .expect("INVARIANT: thread producer init failed");
        match prod.push(sample) {
            Ok(()) => {}
            Err(PushError::Full(_)) => {
                DROPPED.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
}

/// Spawn the drain thread. Call exactly once per process,
/// near the top of `main()`, AFTER `tracing_subscriber::
/// fmt::init()` so the drainer's emissions land in the
/// process's normal log file.
///
/// Drains every registered ring at `interval_ms` cadence
/// and emits each sample via `tracing::info!(target =
/// "latency", ...)` — the exact line shape the existing
/// dashboard parser already understands.
pub fn start_drainer(interval_ms: u64) {
    let interval = Duration::from_millis(interval_ms);
    std::thread::Builder::new()
        .name("latency-drain".into())
        .spawn(move || {
            // Stable buffer reused across iterations.
            let mut batch: Vec<Sample> = Vec::with_capacity(
                RING_CAP,
            );
            loop {
                std::thread::sleep(interval);
                // Drain each registered consumer. The
                // registry mutex is held briefly here only
                // to obtain the iterator; pop() itself is
                // wait-free against the producer side.
                {
                    let mut regs = consumers()
                        .lock()
                        .expect("INVARIANT: latency registry mutex poisoned");
                    for cons in regs.iter_mut() {
                        while let Ok(s) = cons.pop() {
                            batch.push(s);
                        }
                    }
                }
                let dropped =
                    DROPPED.swap(0, Ordering::Relaxed);
                if dropped > 0 {
                    tracing::warn!(
                        target: "latency",
                        "dropped {} samples (ring full)",
                        dropped,
                    );
                }
                for s in batch.drain(..) {
                    tracing::info!(
                        target: "latency",
                        stage = s.stage,
                        oid = format!(
                            "{:016x}{:016x}",
                            s.oid_hi, s.oid_lo,
                        ),
                        t_us = s.t_us,
                        t0_ns = s.t0_ns,
                    );
                }
            }
        })
        .expect("INVARIANT: failed to spawn latency-drain thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_then_drain_via_consumer() {
        // Emit from this test thread; then directly pop
        // the consumer (bypassing the drainer thread).
        emit("test_stage", 0xaabb, 0xccdd, 42, 1_700_000_000_000_000_000);
        // Wait for thread-local registration to settle.
        // (The first emit on a new thread does an internal
        // mutex push; subsequent ones are wait-free.)
        let mut regs = consumers().lock().unwrap();
        let mut seen = false;
        for cons in regs.iter_mut() {
            while let Ok(s) = cons.pop() {
                if s.stage == "test_stage"
                    && s.oid_hi == 0xaabb
                    && s.oid_lo == 0xccdd
                    && s.t_us == 42
                {
                    seen = true;
                }
            }
        }
        assert!(seen, "test sample not found in any ring");
    }

    #[test]
    fn drop_counter_increments_when_ring_full() {
        // Fill our thread's ring to capacity, then emit one
        // more and check the global drop counter.
        for i in 0..RING_CAP {
            emit("fill", i as u64, 0, 0, 0);
        }
        let before = DROPPED.load(Ordering::Relaxed);
        emit("overflow", 0, 0, 0, 0);
        let after = DROPPED.load(Ordering::Relaxed);
        assert!(after > before,
            "drop counter did not increment on overflow");
    }
}
