use rsx_types::Side;
use rustc_hash::FxHashMap;

pub const RECLAIM_GRACE_NS: u64 = 60_000_000_000;

#[derive(Clone, Debug, Default)]
pub struct UserState {
    pub user_id: u32,
    pub net_qty: i64,
    pub order_count: u16,
    pub _pad: [u8; 2],
    pub zero_since_ns: u64,
}

impl UserState {
    pub fn new(user_id: u32) -> Self {
        Self {
            user_id,
            net_qty: 0,
            order_count: 0,
            _pad: [0; 2],
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

pub fn get_or_assign_user(
    user_states: &mut Vec<UserState>,
    user_map: &mut FxHashMap<u32, u16>,
    user_free_list: &mut Vec<u16>,
    user_bump: &mut u16,
    user_id: u32,
) -> u16 {
    if let Some(&idx) = user_map.get(&user_id) {
        user_states[idx as usize].clear_zero_mark();
        return idx;
    }
    let idx = if let Some(free) = user_free_list.pop() {
        user_states[free as usize] =
            UserState::new(user_id);
        free
    } else {
        let idx = *user_bump;
        *user_bump += 1;
        if idx as usize >= user_states.len() {
            user_states.push(UserState::new(user_id));
        } else {
            user_states[idx as usize] =
                UserState::new(user_id);
        }
        idx
    };
    user_map.insert(user_id, idx);
    idx
}

pub fn try_reclaim(
    user_states: &mut [UserState],
    user_map: &mut FxHashMap<u32, u16>,
    user_free_list: &mut Vec<u16>,
    now_ns: u64,
    replay_mode: bool,
) -> Option<u32> {
    if replay_mode {
        return None;
    }
    for (i, s) in user_states.iter().enumerate() {
        if s.user_id == 0 {
            continue;
        }
        if !s.is_idle() {
            continue;
        }
        let z = s.zero_since_ns;
        if z == 0 {
            continue;
        }
        if now_ns.saturating_sub(z) >= RECLAIM_GRACE_NS {
            let uid = s.user_id;
            user_map.remove(&uid);
            user_states[i] = UserState::default();
            user_free_list.push(i as u16);
            return Some(uid);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub fn update_positions_on_fill(
    user_states: &mut Vec<UserState>,
    user_map: &mut FxHashMap<u32, u16>,
    user_free_list: &mut Vec<u16>,
    user_bump: &mut u16,
    taker_user_id: u32,
    maker_user_id: u32,
    taker_side: Side,
    qty: i64,
) {
    let sign: i64 = match taker_side {
        Side::Buy => 1,
        Side::Sell => -1,
    };
    let taker_idx = get_or_assign_user(
        user_states,
        user_map,
        user_free_list,
        user_bump,
        taker_user_id,
    );
    user_states[taker_idx as usize].net_qty +=
        sign * qty;
    let maker_idx = get_or_assign_user(
        user_states,
        user_map,
        user_free_list,
        user_bump,
        maker_user_id,
    );
    user_states[maker_idx as usize].net_qty -=
        sign * qty;
}
