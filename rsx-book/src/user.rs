use rsx_types::Side;
use rustc_hash::FxHashMap;

pub const RECLAIM_GRACE_NS: u64 = 60_000_000_000;

#[derive(Clone, Debug, Default)]
pub struct UserState {
    pub user_id: u32,
    pub net_qty: i64,
    pub order_count: u32,
    pub zero_since_ns: u64,
}

impl UserState {
    pub fn new(user_id: u32) -> Self {
        Self {
            user_id,
            net_qty: 0,
            order_count: 0,
            zero_since_ns: 0,
        }
    }

    pub fn is_idle(&self) -> bool {
        self.net_qty == 0 && self.order_count == 0
    }

    pub fn mark_zero_if_idle(&mut self, now_ns: u64) {
        if self.is_idle() && self.zero_since_ns == 0 {
            self.zero_since_ns = now_ns;
        }
    }

    pub fn clear_zero_mark(&mut self) {
        self.zero_since_ns = 0;
    }
}

/// Per-user net position and idle-reclamation state used by matching and
/// persisted in book snapshots.
#[derive(Default)]
pub struct UserRegistry {
    pub(crate) user_states: Vec<UserState>,
    pub(crate) user_map: FxHashMap<u32, u32>,
    pub(crate) user_free_list: Vec<u32>,
    pub(crate) user_bump: u32,
    pub(crate) maintenance_cursor: u32,
}

impl UserRegistry {
    pub fn new() -> Self {
        Self {
            user_states: Vec::with_capacity(256),
            user_map: FxHashMap::default(),
            user_free_list: Vec::new(),
            user_bump: 0,
            maintenance_cursor: 0,
        }
    }

    /// Return the slab index for `user_id`, assigning (and clearing any
    /// idle zero-mark) on first sight. Reuses a free-list slot before
    /// bumping.
    pub fn get_or_assign(&mut self, user_id: u32) -> u32 {
        if let Some(&idx) = self.user_map.get(&user_id) {
            self.user_states[idx as usize].clear_zero_mark();
            return idx;
        }
        let idx = if let Some(free) = self.user_free_list.pop() {
            self.user_states[free as usize] = UserState::new(user_id);
            free
        } else {
            let idx = self.user_bump;
            self.user_bump = self
                .user_bump
                .checked_add(1)
                .expect("user registry exhausted u32 index space");
            if idx as usize >= self.user_states.len() {
                self.user_states.push(UserState::new(user_id));
            } else {
                self.user_states[idx as usize] = UserState::new(user_id);
            }
            idx
        };
        self.user_map.insert(user_id, idx);
        idx
    }

    /// Inspect one assigned slot, marking newly idle users and reclaiming
    /// users that remained idle through the grace period. This bounded
    /// maintenance step is intended for an engine idle loop, never the
    /// per-order path. It is disabled while replaying.
    pub fn maintain_one(&mut self, now_ns: u64, replay_mode: bool) -> Option<u32> {
        if replay_mode || self.user_bump == 0 {
            return None;
        }
        let i = (self.maintenance_cursor % self.user_bump) as usize;
        self.maintenance_cursor = (self.maintenance_cursor + 1) % self.user_bump;
        let state = &mut self.user_states[i];
        if state.user_id == 0 {
            return None;
        }
        if !state.is_idle() {
            state.clear_zero_mark();
            return None;
        }
        if state.zero_since_ns == 0 {
            state.mark_zero_if_idle(now_ns);
            return None;
        }
        if now_ns.saturating_sub(state.zero_since_ns) < RECLAIM_GRACE_NS {
            return None;
        }
        let uid = self.user_states[i].user_id;
        self.user_map.remove(&uid);
        self.user_states[i] = UserState::default();
        self.user_free_list.push(i as u32);
        Some(uid)
    }

    /// Whether `user_id` currently has a slot assigned.
    pub fn contains(&self, user_id: u32) -> bool {
        self.user_map.contains_key(&user_id)
    }

    /// Net position for `user_id`, or `None` if the user is unknown.
    pub fn net_qty(&self, user_id: u32) -> Option<i64> {
        self.user_map
            .get(&user_id)
            .map(|&idx| self.user_states[idx as usize].net_qty)
    }

    /// Resting-order count for `user_id`, or `None` if the user is unknown.
    pub fn order_count(&self, user_id: u32) -> Option<u32> {
        self.user_map
            .get(&user_id)
            .map(|&idx| self.user_states[idx as usize].order_count)
    }

    /// Register a new resting order for `user_id`: ensures the slot exists
    /// (clearing any idle zero-mark) and bumps its order count.
    pub fn add_order(&mut self, user_id: u32) {
        let idx = self.get_or_assign(user_id);
        self.user_states[idx as usize].order_count += 1;
    }

    /// Drop one resting order for `user_id` (cancel, or a maker order
    /// fully filled). No-op if the user is unknown; saturating so the
    /// count never underflows.
    pub fn remove_order(&mut self, user_id: u32) {
        if let Some(&idx) = self.user_map.get(&user_id) {
            let count = &mut self.user_states[idx as usize].order_count;
            *count = count.saturating_sub(1);
        }
    }

    /// Apply a fill to taker and maker positions. The taker moves in the
    /// direction of its side; the maker moves opposite by the same qty.
    pub fn apply_fill(
        &mut self,
        taker_user_id: u32,
        maker_user_id: u32,
        taker_side: Side,
        qty: i64,
    ) {
        let sign: i64 = match taker_side {
            Side::Buy => 1,
            Side::Sell => -1,
        };
        let taker_idx = self.get_or_assign(taker_user_id);
        self.user_states[taker_idx as usize].net_qty = self.user_states[taker_idx as usize]
            .net_qty
            .saturating_add(sign * qty);
        let maker_idx = self.get_or_assign(maker_user_id);
        self.user_states[maker_idx as usize].net_qty = self.user_states[maker_idx as usize]
            .net_qty
            .saturating_add(-(sign * qty));
    }
}

#[cfg(test)]
#[path = "user_test.rs"]
mod user_test;
