//! rsx-log: off-hot-path logging primitive. See README.md.

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

/// Fixed-shape log record. Kept small so each push is a
/// few hundred bits, not heap-pointer machinery.
///
/// Discriminated by `kind`. Today the only kind in use is
/// [`Kind::Latency`]; future warn/error variants would
/// reuse the same shape (`stage_or_target` becomes the
/// log target; numeric fields encode level + payload).
#[derive(Clone, Copy, Debug)]
pub struct Record {
    pub kind: Kind,
    /// For Latency: the stage name. For future kinds it
    /// can be the `tracing` target string. Always a static
    /// string so the push allocates nothing.
    pub stage_or_target: &'static str,
    /// Order id high half (Latency-only payload).
    pub a: u64,
    /// Order id low half (Latency-only payload).
    pub b: u64,
    /// Latency delta in µs (Latency-only).
    pub c: u64,
    /// Anchor timestamp in ns (Latency-only).
    pub d: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Latency = 0,
}

/// Per-thread ring capacity. At ~6 emits per order ×
/// peak ~10 k orders/s × 100 ms drain interval, 8 192
/// slots is 1.3 s of headroom per thread.
const RING_CAP: usize = 8_192;

static CONSUMERS: OnceLock<Mutex<Vec<Consumer<Record>>>> =
    OnceLock::new();

static DROPPED: AtomicU64 = AtomicU64::new(0);

fn consumers() -> &'static Mutex<Vec<Consumer<Record>>> {
    CONSUMERS.get_or_init(|| Mutex::new(Vec::new()))
}

thread_local! {
    static PRODUCER: RefCell<Option<Producer<Record>>> =
        const { RefCell::new(None) };
}

fn init_thread_ring() -> Producer<Record> {
    let (prod, cons) =
        RingBuffer::<Record>::new(RING_CAP);
    // SAFETY: registry mutex is held briefly on the slow
    // path (first push per thread). Poisoning implies a
    // panic during another thread's init — fail fast.
    consumers()
        .lock()
        .expect("INVARIANT: rsx-log registry mutex poisoned")
        .push(cons);
    prod
}

/// Push a record onto this thread's ring. Hot-path-safe:
/// a single wait-free SPSC push (~20-30 ns). First call
/// per thread allocates the ring (~µs); steady state is
/// the fast path.
#[inline]
pub fn push(record: Record) {
    PRODUCER.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(init_thread_ring());
        }
        // SAFETY: just initialised above if it was None.
        let prod = slot
            .as_mut()
            .expect("INVARIANT: thread producer init failed");
        match prod.push(record) {
            Ok(()) => {}
            Err(PushError::Full(_)) => {
                DROPPED.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
}

/// Sub-module for the per-stage latency sample API.
pub mod latency {
    use super::Kind;
    use super::Record;

    /// Push a latency sample. Wraps [`super::push`] with
    /// the fields named the way callers think about them.
    #[inline]
    pub fn sample(
        stage: &'static str,
        oid_hi: u64,
        oid_lo: u64,
        t_us: u64,
        t0_ns: u64,
    ) {
        super::push(Record {
            kind: Kind::Latency,
            stage_or_target: stage,
            a: oid_hi,
            b: oid_lo,
            c: t_us,
            d: t0_ns,
        });
    }
}

/// Spawn the drain thread. Call exactly once per process,
/// near the top of `main()`, AFTER `tracing_subscriber::
/// fmt::init()` so the drainer's emissions land in the
/// process's normal log file.
pub fn start_drainer(interval_ms: u64) {
    let interval = Duration::from_millis(interval_ms);
    std::thread::Builder::new()
        .name("rsx-log-drain".into())
        .spawn(move || {
            let mut batch: Vec<Record> = Vec::with_capacity(
                RING_CAP,
            );
            loop {
                std::thread::sleep(interval);
                {
                    // SAFETY: poisoned mutex implies a
                    // panic in init_thread_ring — fail-fast.
                    let mut regs = consumers()
                        .lock()
                        .expect("INVARIANT: rsx-log registry mutex poisoned");
                    for cons in regs.iter_mut() {
                        while let Ok(r) = cons.pop() {
                            batch.push(r);
                        }
                    }
                }
                let dropped =
                    DROPPED.swap(0, Ordering::Relaxed);
                if dropped > 0 {
                    tracing::warn!(
                        target: "latency",
                        "rsx-log dropped {} records (ring full)",
                        dropped,
                    );
                }
                for r in batch.drain(..) {
                    dispatch(&r);
                }
            }
        })
        .expect("INVARIANT: failed to spawn rsx-log-drain thread");
}

fn dispatch(r: &Record) {
    match r.kind {
        Kind::Latency => {
            tracing::info!(
                target: "latency",
                stage = r.stage_or_target,
                oid = format!(
                    "{:016x}{:016x}",
                    r.a, r.b,
                ),
                t_us = r.c,
                t0_ns = r.d,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_sample_round_trips_via_ring() {
        latency::sample(
            "test_stage",
            0xaabb,
            0xccdd,
            42,
            1_700_000_000_000_000_000,
        );
        let mut regs = consumers().lock().unwrap();
        let mut seen = false;
        for cons in regs.iter_mut() {
            while let Ok(r) = cons.pop() {
                if r.kind == Kind::Latency
                    && r.stage_or_target == "test_stage"
                    && r.a == 0xaabb
                    && r.b == 0xccdd
                    && r.c == 42
                {
                    seen = true;
                }
            }
        }
        assert!(seen, "test sample not found in any ring");
    }

    #[test]
    fn drop_counter_increments_on_full_ring() {
        for i in 0..RING_CAP {
            latency::sample(
                "fill",
                i as u64,
                0,
                0,
                0,
            );
        }
        let before = DROPPED.load(Ordering::Relaxed);
        latency::sample("overflow", 0, 0, 0, 0);
        let after = DROPPED.load(Ordering::Relaxed);
        assert!(
            after > before,
            "drop counter did not increment on overflow",
        );
    }
}
