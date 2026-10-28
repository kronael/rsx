use crate::risk_utils::sleep_unless_shutdown;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

#[test]
fn shutdown_during_a_long_sleep_returns_within_a_slice() {
    let shutdown = AtomicBool::new(false);
    let start = Instant::now();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(50));
            shutdown.store(true, Ordering::Relaxed);
        });
        sleep_unless_shutdown(Duration::from_secs(5), &shutdown);
    });
    assert!(start.elapsed() < Duration::from_millis(200));
}

#[test]
fn an_unset_flag_sleeps_the_whole_duration() {
    let shutdown = AtomicBool::new(false);
    let start = Instant::now();
    sleep_unless_shutdown(Duration::from_millis(250), &shutdown);
    assert!(start.elapsed() >= Duration::from_millis(250));
}
