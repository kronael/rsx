use rustc_hash::FxHashMap;
use std::collections::VecDeque;

/// Where an order stands with risk. `Submitted` has had no response yet;
/// `Resting` has been inserted into the book and stays pending until a
/// terminal record arrives; `TimedOut` had no response in time, so its fate
/// is unknown: it holds no `Submitted` slot, stays cancellable and turns
/// `Resting` on a late insert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingStage {
    Submitted,
    Resting,
    TimedOut,
}

#[derive(Clone)]
pub struct PendingOrder {
    pub order_id: [u8; 16],
    pub user_id: u32,
    pub symbol_id: u32,
    pub client_order_id: [u8; 20],
    pub timestamp_ns: u64,
    pub stage: PendingStage,
}

/// Orders awaiting a terminal record, one queue per user. `capacity`
/// bounds the `Submitted` orders across all users; `Resting` and `TimedOut`
/// orders stay tracked so they can be cancelled, and do not count against
/// it. A lookup names the user, so one user's cancel never sees another
/// user's order.
///
/// A user's queue keeps its allocation when it empties, so the next
/// order from that user does not allocate; `drop_timed_out` drops the
/// empty queues.
pub struct PendingOrders {
    by_user: FxHashMap<u32, VecDeque<PendingOrder>>,
    /// Every tracked order, whatever its stage.
    count: usize,
    /// The `Submitted` orders: what `capacity` bounds.
    in_flight: usize,
    /// The `TimedOut` orders.
    timed_out: usize,
    capacity: usize,
}

impl PendingOrders {
    pub fn new(capacity: usize) -> Self {
        Self {
            by_user: FxHashMap::default(),
            count: 0,
            in_flight: 0,
            timed_out: 0,
            capacity,
        }
    }

    /// Track `order`. A `Submitted` order is refused (false) while
    /// `capacity` `Submitted` orders are tracked; any other stage is
    /// always accepted.
    pub fn push(&mut self, order: PendingOrder) -> bool {
        if order.stage == PendingStage::Submitted && self.is_full() {
            return false;
        }
        self.count += 1;
        self.enter(order.stage);
        self.by_user
            .entry(order.user_id)
            .or_default()
            .push_back(order);
        true
    }

    fn enter(&mut self, stage: PendingStage) {
        match stage {
            PendingStage::Submitted => self.in_flight += 1,
            PendingStage::TimedOut => self.timed_out += 1,
            PendingStage::Resting => {}
        }
    }

    fn leave(&mut self, stage: PendingStage) {
        match stage {
            PendingStage::Submitted => self.in_flight -= 1,
            PendingStage::TimedOut => self.timed_out -= 1,
            PendingStage::Resting => {}
        }
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
            self.leave(order.stage);
        }
        found
    }

    /// Mark a pending order as resting so `time_out` leaves it and its
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
        let stage = order.stage;
        order.stage = PendingStage::Resting;
        self.leave(stage);
        true
    }

    /// Every tracked order, whatever its stage.
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

    /// The `TimedOut` orders, tracked for cancel and outside `capacity`.
    pub fn timed_out(&self) -> usize {
        self.timed_out
    }

    /// The `Resting` orders, tracked for cancel and outside `capacity`.
    pub fn resting(&self) -> usize {
        self.count - self.in_flight - self.timed_out
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

    /// Turn `Submitted` orders older than `cutoff_ns` into `TimedOut`,
    /// freeing their `capacity` slots. Resting orders are never touched.
    /// Returns the orders that timed out.
    pub fn time_out(&mut self, cutoff_ns: u64) -> Vec<PendingOrder> {
        let mut stale = Vec::new();
        for order in self.by_user.values_mut().flatten() {
            if order.stage == PendingStage::Submitted && order.timestamp_ns < cutoff_ns {
                order.stage = PendingStage::TimedOut;
                stale.push(order.clone());
            }
        }
        self.in_flight -= stale.len();
        self.timed_out += stale.len();
        stale
    }

    /// Remove `TimedOut` orders older than `cutoff_ns` and the queues that
    /// empty.
    pub fn drop_timed_out(&mut self, cutoff_ns: u64) {
        let mut dropped = 0;
        for queue in self.by_user.values_mut() {
            let before = queue.len();
            queue.retain(|order| {
                order.stage != PendingStage::TimedOut || order.timestamp_ns >= cutoff_ns
            });
            dropped += before - queue.len();
        }
        self.count -= dropped;
        self.timed_out -= dropped;
        self.by_user.retain(|_, queue| !queue.is_empty());
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
