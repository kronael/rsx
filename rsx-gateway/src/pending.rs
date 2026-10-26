use rustc_hash::FxHashMap;
use std::collections::VecDeque;

/// Where an order stands with risk. `Submitted` has had no response yet;
/// `Resting` has been inserted into the book and stays pending until a
/// terminal record arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingStage {
    Submitted,
    Resting,
}

pub struct PendingOrder {
    pub order_id: [u8; 16],
    pub user_id: u32,
    pub symbol_id: u32,
    pub client_order_id: [u8; 20],
    pub timestamp_ns: u64,
    pub stage: PendingStage,
}

/// Orders awaiting a terminal record, one queue per user. `capacity`
/// bounds the `Submitted` orders across all users; `Resting` orders stay
/// tracked so they can be cancelled, and do not count against it. A lookup
/// names the user, so one user's cancel never sees another user's order.
///
/// A user's queue keeps its allocation when it empties, so the next
/// order from that user does not allocate; `remove_stale` drops the
/// empty queues.
pub struct PendingOrders {
    by_user: FxHashMap<u32, VecDeque<PendingOrder>>,
    /// Every tracked order, `Submitted` and `Resting`.
    count: usize,
    /// The `Submitted` orders: what `capacity` bounds.
    in_flight: usize,
    capacity: usize,
}

impl PendingOrders {
    pub fn new(capacity: usize) -> Self {
        Self {
            by_user: FxHashMap::default(),
            count: 0,
            in_flight: 0,
            capacity,
        }
    }

    /// Track `order`. A `Submitted` order is refused (false) while
    /// `capacity` `Submitted` orders are tracked; a `Resting` order is
    /// always accepted.
    pub fn push(&mut self, order: PendingOrder) -> bool {
        let submitted = order.stage == PendingStage::Submitted;
        if submitted && self.is_full() {
            return false;
        }
        self.by_user
            .entry(order.user_id)
            .or_default()
            .push_back(order);
        self.count += 1;
        if submitted {
            self.in_flight += 1;
        }
        true
    }

    /// Remove a pending order by order_id. `user_id` names the queue
    /// searched first; when the order is not there (a record naming
    /// the wrong user), every other queue is searched.
    pub fn remove(&mut self, user_id: u32, order_id: &[u8; 16]) -> Option<PendingOrder> {
        let found = self
            .by_user
            .get_mut(&user_id)
            .and_then(|queue| remove_from(queue, order_id));
        let found = found.or_else(|| {
            self.by_user
                .values_mut()
                .find_map(|queue| remove_from(queue, order_id))
        });
        if let Some(order) = &found {
            self.count -= 1;
            if order.stage == PendingStage::Submitted {
                self.in_flight -= 1;
            }
        }
        found
    }

    /// Mark a pending order as resting so `remove_stale` leaves it and its
    /// slot under `capacity` is freed. Searches `user_id`'s queue first,
    /// then every other queue, as `remove` does. Returns false when the
    /// order is not pending.
    pub fn mark_resting(&mut self, user_id: u32, order_id: &[u8; 16]) -> bool {
        let order = match self
            .by_user
            .get_mut(&user_id)
            .and_then(|queue| find_mut(queue, order_id))
        {
            Some(order) => order,
            None => {
                match self
                    .by_user
                    .values_mut()
                    .find_map(|queue| find_mut(queue, order_id))
                {
                    Some(order) => order,
                    None => return false,
                }
            }
        };
        if order.stage == PendingStage::Submitted {
            order.stage = PendingStage::Resting;
            self.in_flight -= 1;
        }
        true
    }

    /// Every tracked order, `Submitted` and `Resting`.
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The `Submitted` orders: what `capacity` bounds.
    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// The `Resting` orders, tracked for cancel and outside `capacity`.
    pub fn resting(&self) -> usize {
        self.count - self.in_flight
    }

    /// True when `capacity` `Submitted` orders are tracked.
    pub fn is_full(&self) -> bool {
        self.in_flight >= self.capacity
    }

    /// Find a pending order of `user_id` by order_id.
    pub fn find_by_order_id(&self, user_id: u32, order_id: &[u8; 16]) -> Option<&PendingOrder> {
        self.by_user
            .get(&user_id)?
            .iter()
            .find(|order| &order.order_id == order_id)
    }

    /// Find a pending order of `user_id` by client_order_id.
    pub fn find_by_client_order_id(&self, user_id: u32, cid: &[u8; 20]) -> Option<&PendingOrder> {
        self.by_user
            .get(&user_id)?
            .iter()
            .find(|order| &order.client_order_id == cid)
    }

    /// Remove `Submitted` orders older than `cutoff_ns` timestamp. Resting
    /// orders are never removed. Returns removed orders.
    pub fn remove_stale(&mut self, cutoff_ns: u64) -> Vec<PendingOrder> {
        let mut stale = Vec::new();
        for queue in self.by_user.values_mut() {
            let mut i = 0;
            while i < queue.len() {
                if queue[i].stage == PendingStage::Submitted && queue[i].timestamp_ns < cutoff_ns {
                    if let Some(order) = queue.remove(i) {
                        stale.push(order);
                    }
                } else {
                    i += 1;
                }
            }
        }
        self.count -= stale.len();
        self.in_flight -= stale.len();
        self.by_user.retain(|_, queue| !queue.is_empty());
        stale
    }
}

/// Newest-first search: completions arrive for recent orders.
fn remove_from(queue: &mut VecDeque<PendingOrder>, order_id: &[u8; 16]) -> Option<PendingOrder> {
    let index = queue
        .iter()
        .rposition(|order| &order.order_id == order_id)?;
    queue.remove(index)
}

fn find_mut<'a>(
    queue: &'a mut VecDeque<PendingOrder>,
    order_id: &[u8; 16],
) -> Option<&'a mut PendingOrder> {
    queue
        .iter_mut()
        .rev()
        .find(|order| &order.order_id == order_id)
}
