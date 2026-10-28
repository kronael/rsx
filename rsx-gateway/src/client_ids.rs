//! The `(user_id, cid)` to order id map behind idempotent client retries.
//!
//! The gateway mints an order id for each new order. A client that sends the
//! same cid again inside `WINDOW_NS` gets that order id back and nothing is
//! forwarded. A failed order releases its cid at once, so the resend is a new
//! attempt. Entries leave on `expire`, which the housekeeping sweep calls, so
//! the map holds at most the orders admitted in the last hour.
//!
//! Single-threaded: the reactor thread is the only user.

use rustc_hash::FxHashMap;
use std::collections::VecDeque;

/// How long a cid stays bound to its order id: the ME's dedup window.
pub const WINDOW_NS: u64 = 3_600_000_000_000;

type Key = (u32, [u8; 20]);

struct Binding {
    order_id: [u8; 16],
    bound_ns: u64,
}

#[derive(Default)]
pub struct ClientOrderIds {
    bindings: FxHashMap<Key, Binding>,
    /// Keys with their binding time, oldest first; `expire` pops the front.
    /// A released key leaves its entry behind; `expire` skips it by time.
    inserted: VecDeque<(u64, Key)>,
}

impl ClientOrderIds {
    /// The order id minted for `user_id`'s `cid` inside the window.
    pub fn get(&self, user_id: u32, cid: &[u8; 20]) -> Option<[u8; 16]> {
        self.bindings
            .get(&(user_id, *cid))
            .map(|binding| binding.order_id)
    }

    /// Bind `cid` to `order_id`, minted at `now_ns`.
    pub fn insert(&mut self, user_id: u32, cid: [u8; 20], order_id: [u8; 16], now_ns: u64) {
        let key = (user_id, cid);
        let binding = Binding {
            order_id,
            bound_ns: now_ns,
        };
        self.bindings.insert(key, binding);
        self.inserted.push_back((now_ns, key));
    }

    /// Free `cid` when it is still bound to `order_id`: the order failed, so
    /// a resend is a fresh attempt.
    pub fn release(&mut self, user_id: u32, cid: &[u8; 20], order_id: &[u8; 16]) {
        let key = (user_id, *cid);
        if self
            .bindings
            .get(&key)
            .is_some_and(|binding| &binding.order_id == order_id)
        {
            self.bindings.remove(&key);
        }
    }

    /// Drop every binding made more than `WINDOW_NS` before `now_ns`.
    pub fn expire(&mut self, now_ns: u64) {
        let cutoff = now_ns.saturating_sub(WINDOW_NS);
        while let Some(&(bound_ns, key)) = self.inserted.front() {
            if bound_ns >= cutoff {
                break;
            }
            self.inserted.pop_front();
            if self
                .bindings
                .get(&key)
                .is_some_and(|binding| binding.bound_ns == bound_ns)
            {
                self.bindings.remove(&key);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

#[cfg(test)]
#[path = "client_ids_test.rs"]
mod client_ids_test;
