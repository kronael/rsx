use rsx_types::NONE;

pub trait SlabItem {
    fn next(&self) -> u32;
    fn set_next(&mut self, next: u32);
}

/// Fixed-capacity slab arena.
///
/// Invariant #8 (slab no-leak): `bump_next` counts ever-allocated
/// slots; `free_head` is the head of the freelist. Live slots =
/// `bump_next - |freelist|`. Every `alloc()` either pops `free_head`
/// (reuse) or bumps `bump_next` (fresh); every `free()` pushes onto
/// `free_head`. Callers must pair each `alloc` with at most one
/// `free` for the same handle.
pub struct Slab<T: SlabItem> {
    slots: Vec<T>,
    free_head: u32,
    bump_next: u32,
}

impl<T: SlabItem + Default> Slab<T> {
    pub fn new(capacity: u32) -> Self {
        let mut slots = Vec::with_capacity(capacity as usize);
        slots.resize_with(capacity as usize, T::default);
        Self {
            slots,
            free_head: NONE,
            bump_next: 0,
        }
    }

    pub fn alloc(&mut self) -> u32 {
        if self.free_head != NONE {
            let idx = self.free_head;
            self.free_head = self.slots[idx as usize].next();
            idx
        } else {
            assert!(
                (self.bump_next as usize) < self.slots.len(),
                "slab exhausted"
            );
            let idx = self.bump_next;
            self.bump_next += 1;
            idx
        }
    }

    pub fn free(&mut self, idx: u32) {
        assert!(
            (idx as usize) < self.slots.len(),
            "slab free: idx {} out of bounds",
            idx
        );
        // Guard against corrupting the freelist. A never-allocated slot
        // (>= bump_next) or an already-free slot would splice a cycle /
        // alias into the freelist and hand the same slot out twice. The
        // is-free walk is O(free) so it stays behind debug_assert (off in
        // release; the ME bounds open orders upstream — see the crate's
        // trust boundary). Callers must pair each `alloc` with at most one
        // `free` for the same handle (invariant #8, slab no-leak).
        debug_assert!(
            idx < self.bump_next,
            "slab free: idx {} never allocated (bump_next {})",
            idx,
            self.bump_next,
        );
        debug_assert!(
            !self.is_free(idx),
            "slab double-free: idx {} already on freelist",
            idx,
        );
        self.slots[idx as usize].set_next(self.free_head);
        self.free_head = idx;
    }

    /// True iff `idx` is already on the freelist. O(free); debug-only
    /// double-free detection (see `free`), never called on the hot path.
    fn is_free(&self, idx: u32) -> bool {
        let mut cur = self.free_head;
        while cur != NONE {
            if cur == idx {
                return true;
            }
            cur = self.slots[cur as usize].next();
        }
        false
    }

    #[inline]
    pub fn get(&self, idx: u32) -> &T {
        &self.slots[idx as usize]
    }

    #[inline]
    pub fn get_mut(&mut self, idx: u32) -> &mut T {
        &mut self.slots[idx as usize]
    }

    pub fn len(&self) -> u32 {
        self.bump_next
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn capacity(&self) -> u32 {
        self.slots.len() as u32
    }

    /// Number of slots currently on the freelist. Walks the freelist
    /// (O(free)), so it is for introspection/tests, not the hot path.
    /// Invariant #8 (no-leak) cross-check: `len() == free_count() +
    /// active`, where `active` = live orders across the book.
    pub fn free_count(&self) -> u32 {
        let mut n = 0;
        let mut cur = self.free_head;
        while cur != NONE {
            n += 1;
            cur = self.slots[cur as usize].next();
        }
        n
    }

    /// Set bump_next for snapshot restore.
    pub fn set_bump_next(&mut self, val: u32) {
        self.bump_next = val;
    }
}
