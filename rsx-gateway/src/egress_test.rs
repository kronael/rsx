use crate::egress::EgressWaker;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;

/// Counts wake calls so a test can prove a parked task was woken.
struct CountWaker(AtomicUsize);

impl Wake for CountWaker {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

/// A signal delivered before anyone waits is sticky: the next
/// `wait` resolves immediately, without a stream ever becoming
/// readable.
#[test]
fn signal_before_wait_resolves_immediately() {
    let counter = Arc::new(CountWaker(AtomicUsize::new(0)));
    let waker = Waker::from(counter.clone());
    let mut cx = Context::from_waker(&waker);

    let egress = EgressWaker::default();
    egress.signal();

    let mut fut = egress.wait();
    assert_eq!(Pin::new(&mut fut).poll(&mut cx), Poll::Ready(()));
}

/// The core egress behavior: a task parked on `wait` (no inbound
/// traffic) is woken the instant a producer calls `signal`, and its
/// next poll resolves.
#[test]
fn signal_wakes_parked_waiter() {
    let counter = Arc::new(CountWaker(AtomicUsize::new(0)));
    let waker = Waker::from(counter.clone());
    let mut cx = Context::from_waker(&waker);

    let egress = EgressWaker::default();
    let mut fut = egress.wait();

    // Parked: nothing queued yet, no wake fired.
    assert_eq!(Pin::new(&mut fut).poll(&mut cx), Poll::Pending);
    assert_eq!(counter.0.load(Ordering::Relaxed), 0);

    // Producer queues a message and signals: the parked task wakes.
    egress.signal();
    assert_eq!(counter.0.load(Ordering::Relaxed), 1);

    // The re-poll after the wake resolves.
    assert_eq!(Pin::new(&mut fut).poll(&mut cx), Poll::Ready(()));
}

/// Each `signal` arms exactly one `wait`; after it resolves the next
/// `wait` parks again rather than spinning.
#[test]
fn wait_reparks_after_resolving() {
    let counter = Arc::new(CountWaker(AtomicUsize::new(0)));
    let waker = Waker::from(counter.clone());
    let mut cx = Context::from_waker(&waker);

    let egress = EgressWaker::default();
    egress.signal();

    let mut first = egress.wait();
    assert_eq!(Pin::new(&mut first).poll(&mut cx), Poll::Ready(()));

    // Pending bit consumed by the first wait; a fresh wait parks.
    let mut second = egress.wait();
    assert_eq!(Pin::new(&mut second).poll(&mut cx), Poll::Pending);
}

/// `close` wakes a task parked on `closed`; the next poll resolves and
/// stays resolved.
#[test]
fn close_wakes_parked_closed_waiter() {
    let counter = Arc::new(CountWaker(AtomicUsize::new(0)));
    let waker = Waker::from(counter.clone());
    let mut cx = Context::from_waker(&waker);

    let egress = EgressWaker::default();
    let mut fut = egress.closed();
    assert_eq!(Pin::new(&mut fut).poll(&mut cx), Poll::Pending);

    egress.close();
    assert_eq!(counter.0.load(Ordering::Relaxed), 1);
    assert_eq!(Pin::new(&mut fut).poll(&mut cx), Poll::Ready(()));
    assert_eq!(
        Pin::new(&mut egress.closed()).poll(&mut cx),
        Poll::Ready(())
    );
}

/// A plain `signal` wakes the task but leaves `closed` pending.
#[test]
fn signal_does_not_close() {
    let counter = Arc::new(CountWaker(AtomicUsize::new(0)));
    let waker = Waker::from(counter.clone());
    let mut cx = Context::from_waker(&waker);

    let egress = EgressWaker::default();
    egress.signal();
    assert_eq!(Pin::new(&mut egress.closed()).poll(&mut cx), Poll::Pending);
}
