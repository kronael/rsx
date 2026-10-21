//! The `(user_id, cid)` to order id map behind idempotent client retries.
//!
//! The gateway mints an order id for each new order. A client that sends the
//! same cid again inside `WINDOW_NS` gets that order id back and nothing is
//! forwarded. Entries leave on `expire`, which the housekeeping sweep calls,
//! so the map holds at most the orders admitted in the last hour.
//!
//! Single-threaded: the reactor thread is the only user.

use rustc_hash::FxHashMap;
use std::collections::VecDeque;

/// How long a cid stays bound to its order id: the ME's dedup window.
pub const WINDOW_NS: u64 = 3_600_000_000_000;

type Key = (u32, [u8; 20]);

#[derive(Default)]
pub struct ClientOrderIds {
    order_ids: FxHashMap<Key, [u8; 16]>,
    /// Keys with their insertion time, oldest first; `expire` pops the front.
    inserted: VecDeque<(u64, Key)>,
}

impl ClientOrderIds {
    /// The order id minted for `user_id`'s `cid` inside the window.
    pub fn get(&self, user_id: u32, cid: &[u8; 20]) -> Option<[u8; 16]> {
        self.order_ids.get(&(user_id, *cid)).copied()
    }

    /// Bind `cid` to `order_id`, minted at `now_ns`.
    pub fn insert(&mut self, user_id: u32, cid: [u8; 20], order_id: [u8; 16], now_ns: u64) {
        let key = (user_id, cid);
        if self.order_ids.insert(key, order_id).is_none() {
            self.inserted.push_back((now_ns, key));
        }
    }

    /// Drop every binding made more than `WINDOW_NS` before `now_ns`.
    pub fn expire(&mut self, now_ns: u64) {
        let cutoff = now_ns.saturating_sub(WINDOW_NS);
        while let Some(&(at_ns, key)) = self.inserted.front() {
            if at_ns >= cutoff {
                break;
            }
            self.inserted.pop_front();
            self.order_ids.remove(&key);
        }
    }

    pub fn len(&self) -> usize {
        self.order_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order_ids.is_empty()
    }
}

#[cfg(test)]
#[path = "client_ids_test.rs"]
mod client_ids_test;
