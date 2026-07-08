use std::cell::Cell;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

/// Per-client egress wake for the single-threaded monoio event loop.
/// The cast/broadcast loop calls [`signal`](EgressWaker::signal)
/// right after queuing a message into a client's `outbound`; that
/// client's handler task parks on [`wait`](EgressWaker::wait) and is
/// woken immediately, instead of polling `outbound` on a timer.
///
/// A `signal` delivered while no task is parked sets a sticky
/// pending bit, so the next `wait` returns without blocking — a wake
/// queued between the handler's drain and its re-park is never lost.
///
/// Single-threaded by construction: the `Cell` makes this `!Sync`,
/// so `signal` and the future's `poll` can only run on the same
/// thread and never interleave mid-call. That is what lets `poll`
/// register the waker without a re-check.
#[derive(Default)]
pub struct EgressWaker {
    pending: Cell<bool>,
    waker: RefCell<Option<Waker>>,
}

impl EgressWaker {
    /// Queue a wake for the client's handler: set the pending bit and
    /// wake the parked task, if any. Idempotent between wakes — many
    /// `signal`s before one `wait` collapse to a single wake.
    pub fn signal(&self) {
        self.pending.set(true);
        if let Some(waker) = self.waker.borrow_mut().take() {
            waker.wake();
        }
    }

    /// Future that resolves the next time `signal` is called, or
    /// immediately if a signal is already pending.
    pub fn wait(&self) -> WaitEgress<'_> {
        WaitEgress { waker: self }
    }
}

/// Future returned by [`EgressWaker::wait`].
pub struct WaitEgress<'a> {
    waker: &'a EgressWaker,
}

impl Future for WaitEgress<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let waker = self.waker;
        if waker.pending.replace(false) {
            Poll::Ready(())
        } else {
            *waker.waker.borrow_mut() = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

#[cfg(test)]
#[path = "egress_test.rs"]
mod egress_test;
