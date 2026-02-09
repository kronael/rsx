use rsx_types::NONE;

pub trait SlabItem {
    fn next(&self) -> u32;
    fn set_next(&mut self, next: u32);
}

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
        self.slots[idx as usize].set_next(self.free_head);
        self.free_head = idx;
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

    pub fn capacity(&self) -> u32 {
        self.slots.len() as u32
    }
}
