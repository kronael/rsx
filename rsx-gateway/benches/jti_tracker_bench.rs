//! `JtiTracker::record` under steady-state churn. Once the
//! tracker is filled to capacity (16384 by spec), every new
//! jti pushes one out via FIFO eviction — that's the
//! production path. We pre-fill the tracker to cap, then
//! time one (insert, evict) pair per iter with a unique jti.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_gateway::jwt::JtiTracker;

const CAP: usize = 16_384;

fn bench_record_steady_state(c: &mut Criterion) {
    c.bench_function("jti_record_steady_state", |b| {
        let mut tracker = JtiTracker::new(CAP);
        // Pre-fill to cap so every record() evicts.
        for i in 0..CAP {
            tracker.record(Some(&format!("seed-{i:08x}")));
        }
        let mut counter: u64 = CAP as u64;
        b.iter(|| {
            counter += 1;
            let jti = format!("jti-{counter:016x}");
            let r = tracker.record(black_box(Some(&jti)));
            black_box(r);
        });
    });
}

/// Lookup-hit path: the same jti seen twice. Cheaper than
/// steady-state insert; useful for triangulating replay-detect cost.
fn bench_record_duplicate(c: &mut Criterion) {
    c.bench_function("jti_record_duplicate", |b| {
        let mut tracker = JtiTracker::new(CAP);
        let jti = "01HXYZ1234567890ABCDEF".to_string();
        tracker.record(Some(&jti));
        b.iter(|| {
            let r = tracker.record(black_box(Some(&jti)));
            black_box(r);
        });
    });
}

criterion_group!(
    benches,
    bench_record_steady_state,
    bench_record_duplicate,
);
criterion_main!(benches);
