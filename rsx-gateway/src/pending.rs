use std::collections::VecDeque;

pub struct PendingOrder {
    pub order_id: [u8; 16],
    pub user_id: u32,
    pub symbol_id: u32,
    pub client_order_id: [u8; 20],
    pub timestamp_ns: u64,
}

pub struct PendingOrders {
    queue: VecDeque<PendingOrder>,
    capacity: usize,
}

impl PendingOrders {
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(
                capacity.min(10_000),
            ),
            capacity,
        }
    }

    pub fn push(&mut self, order: PendingOrder) -> bool {
        if self.queue.len() >= self.capacity {
            return false;
        }
        self.queue.push_back(order);
        true
    }

    pub fn remove(
        &mut self,
        order_id: &[u8; 16],
    ) -> Option<PendingOrder> {
        if let Some(back) = self.queue.back() {
            if &back.order_id == order_id {
                return self.queue.pop_back();
            }
        }
        for i in (0..self.queue.len()).rev() {
            if &self.queue[i].order_id == order_id {
                return self.queue.remove(i);
            }
        }
        None
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.queue.len() >= self.capacity
    }

    /// Find a pending order by order_id.
    pub fn find_by_order_id(
        &self,
        order_id: &[u8; 16],
    ) -> Option<&PendingOrder> {
        self.queue
            .iter()
            .find(|o| &o.order_id == order_id)
    }

    /// Remove orders older than `cutoff_ns` timestamp.
    /// Returns removed orders.
    pub fn remove_stale(
        &mut self,
        cutoff_ns: u64,
    ) -> Vec<PendingOrder> {
        let mut stale = Vec::new();
        let mut i = 0;
        while i < self.queue.len() {
            if self.queue[i].timestamp_ns < cutoff_ns {
                if let Some(order) = self.queue.remove(i) {
                    stale.push(order);
                }
            } else {
                i += 1;
            }
        }
        stale
    }
}
