//! Runtime-agnostic `yield_now()`.
//!
//! Async hot-path loops that drain a non-blocking source
//! (e.g. `CmpReceiver::try_recv`) and then have nothing
//! to wait on (no async readiness primitive) need a way
//! to let cooperatively-scheduled siblings run without
//! imposing a fixed-delay sleep.
//!
//! The naive pattern:
//!
//! ```ignore
//! loop {
//!     while let Some(x) = receiver.try_recv() { ... }
//!     // PROBLEM: without this, the loop pegs CPU and
//!     // never yields to other monoio/tokio tasks.
//!     monoio::time::sleep(Duration::from_micros(100)).await;
//! }
//! ```
//!
//! adds 100 µs of fixed latency *plus* the runtime's
//! timer-wheel resolution jitter. Production traces
//! observed ~655 µs end-to-end through this kind of loop
//! (see `.ship/17-REFINE-2/SPEED-GRANULAR.md`).
//!
//! Replace with:
//!
//! ```ignore
//! loop {
//!     while let Some(x) = receiver.try_recv() { ... }
//!     rsx_types::yield_now().await;
//! }
//! ```
//!
//! `yield_now` returns `Pending` once, schedules itself
//! via `cx.waker().wake_by_ref()`, then returns `Ready`
//! on the second poll. The runtime gets to run any other
//! ready task between the two polls; this task resumes
//! on the next tick of the scheduler — typically within
//! single-digit µs, no timer-wheel involvement.
//!
//! Runtime-agnostic: works with monoio, tokio, and any
//! other executor that honours the Waker contract.

use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

#[derive(Default)]
#[must_use = "yield_now does nothing unless awaited"]
pub struct YieldNow {
    yielded: bool,
}

impl Future for YieldNow {
    type Output = ();

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<()> {
        if self.yielded {
            Poll::Ready(())
        } else {
            self.yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// Cooperatively yield to the runtime scheduler. Returns
/// to the caller after at least one other ready task has
/// had a chance to run. No fixed delay; no timer.
#[inline]
pub fn yield_now() -> YieldNow {
    YieldNow::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::pin;
    use std::sync::Arc;
    use std::task::Context;
    use std::task::Poll;
    use std::task::Wake;
    use std::task::Waker;

    struct DummyWaker;
    impl Wake for DummyWaker {
        fn wake(self: Arc<Self>) {}
    }

    #[test]
    fn yields_once_then_completes() {
        let waker = Waker::from(Arc::new(DummyWaker));
        let mut cx = Context::from_waker(&waker);
        let fut = yield_now();
        let mut fut = pin!(fut);
        // First poll: registers yield, returns Pending.
        assert!(matches!(
            fut.as_mut().poll(&mut cx),
            Poll::Pending
        ));
        // Second poll: completes.
        assert!(matches!(
            fut.as_mut().poll(&mut cx),
            Poll::Ready(())
        ));
    }
}
