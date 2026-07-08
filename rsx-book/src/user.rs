use rsx_types::Side;
use rustc_hash::FxHashMap;
use std::io;
use std::io::Read;
use std::io::Write;

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

/// Per-user state owned by the matching engine: net position (for the
/// reduce-only clamp) plus the slab-index bookkeeping (`user_map` /
/// free-list / bump) and idle-GC counters. Encapsulated as a single
/// seam so it can be relocated or replaced without touching the book /
/// matching / snapshot call sites (see `ME-HOLDS-USER-STATE` in BUGS.md).
#[derive(Default)]
pub struct UserRegistry {
    pub(crate) user_states: Vec<UserState>,
    pub(crate) user_map: FxHashMap<u32, u16>,
    pub(crate) user_free_list: Vec<u16>,
    pub(crate) user_bump: u16,
}

impl UserRegistry {
    pub fn new() -> Self {
        Self {
            user_states: Vec::with_capacity(256),
            user_map: FxHashMap::default(),
            user_free_list: Vec::new(),
            user_bump: 0,
        }
    }

    /// Return the slab index for `user_id`, assigning (and clearing any
    /// idle zero-mark) on first sight. Reuses a free-list slot before
    /// bumping.
    pub fn get_or_assign(&mut self, user_id: u32) -> u16 {
        if let Some(&idx) = self.user_map.get(&user_id) {
            self.user_states[idx as usize].clear_zero_mark();
            return idx;
        }
        let idx = if let Some(free) = self.user_free_list.pop() {
            self.user_states[free as usize] = UserState::new(user_id);
            free
        } else {
            let idx = self.user_bump;
            self.user_bump += 1;
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

    /// Reclaim (at most) one idle user whose slot has been zero past the
    /// grace period, freeing its slab index for reuse. No-op in replay
    /// mode. Returns the reclaimed `user_id`.
    pub fn try_reclaim(&mut self, now_ns: u64, replay_mode: bool) -> Option<u32> {
        if replay_mode {
            return None;
        }
        let mut found: Option<usize> = None;
        for (i, s) in self.user_states.iter().enumerate() {
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
                found = Some(i);
                break;
            }
        }
        let i = found?;
        let uid = self.user_states[i].user_id;
        self.user_map.remove(&uid);
        self.user_states[i] = UserState::default();
        self.user_free_list.push(i as u16);
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
    pub fn order_count(&self, user_id: u32) -> Option<u16> {
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

    /// Serialize the registry's snapshot section (bump, per-user entries,
    /// free-list). Byte layout is part of the on-disk snapshot format.
    pub fn write_snapshot(&self, w: &mut dyn Write) -> io::Result<()> {
        w.write_all(&self.user_bump.to_le_bytes())?;
        let user_count = self.user_map.len() as u32;
        w.write_all(&user_count.to_le_bytes())?;
        for (&uid, &idx) in &self.user_map {
            w.write_all(&uid.to_le_bytes())?;
            w.write_all(&idx.to_le_bytes())?;
            let us = &self.user_states[idx as usize];
            w.write_all(&us.net_qty.to_le_bytes())?;
            w.write_all(&us.order_count.to_le_bytes())?;
        }

        let free_count = self.user_free_list.len() as u32;
        w.write_all(&free_count.to_le_bytes())?;
        for &idx in &self.user_free_list {
            w.write_all(&idx.to_le_bytes())?;
        }

        Ok(())
    }

    /// Reconstruct a registry from the snapshot section written by
    /// `write_snapshot`. Mirror of that byte layout.
    pub fn read_snapshot(r: &mut dyn Read) -> io::Result<UserRegistry> {
        let user_bump = read_u16(r)?;
        let user_count = read_u32(r)?;
        let mut user_states = Vec::with_capacity(user_bump as usize);
        user_states.resize_with(user_bump as usize, UserState::default);
        let mut user_map: FxHashMap<u32, u16> = FxHashMap::default();

        for _ in 0..user_count {
            let uid = read_u32(r)?;
            let idx = read_u16(r)?;
            let net_qty = read_i64(r)?;
            let order_count = read_u16(r)?;
            if idx as usize >= user_bump as usize {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "user index out of bounds",
                ));
            }
            user_map.insert(uid, idx);
            let us = &mut user_states[idx as usize];
            us.user_id = uid;
            us.net_qty = net_qty;
            us.order_count = order_count;
        }

        let free_count = read_u32(r)?;
        let mut user_free_list = Vec::with_capacity(free_count as usize);
        for _ in 0..free_count {
            let fidx = read_u16(r)?;
            if (fidx as usize) >= user_bump as usize {
                tracing::warn!(
                    fidx,
                    user_bump,
                    "snapshot user free-list index out of range, skipping",
                );
                continue;
            }
            user_free_list.push(fidx);
        }

        Ok(UserRegistry {
            user_states,
            user_map,
            user_free_list,
            user_bump,
        })
    }
}

fn read_u16(r: &mut dyn Read) -> io::Result<u16> {
    let mut buf = [0u8; 2];
    r.read_exact(&mut buf)?;
    Ok(u16::from_le_bytes(buf))
}

fn read_u32(r: &mut dyn Read) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_i64(r: &mut dyn Read) -> io::Result<i64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(i64::from_le_bytes(buf))
}

#[cfg(test)]
#[path = "user_test.rs"]
mod user_test;
