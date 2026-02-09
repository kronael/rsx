use rsx_types::NONE;
use crate::slab::SlabItem;

#[repr(C, align(64))]
pub struct OrderSlot {
    // Cache line 1: hot fields
    pub price: i64,
    pub remaining_qty: i64,
    pub side: u8,
    pub flags: u8,       // bit 0: active, bit 1: reduce_only
    pub tif: u8,
    pub _pad1: [u8; 5],
    pub next: u32,
    pub prev: u32,
    pub tick_index: u32,
    pub _pad2: u32,
    // Cache line 2: cold fields
    pub user_id: u32,
    pub sequence: u16,
    pub _pad3: [u8; 2],
    pub original_qty: i64,
    pub timestamp_ns: u64,
    pub _pad4: [u8; 40],
}

const _: () = assert!(
    std::mem::size_of::<OrderSlot>() == 128
);
const _: () = assert!(
    std::mem::align_of::<OrderSlot>() == 64
);

impl OrderSlot {
    pub fn is_active(&self) -> bool {
        self.flags & 1 != 0
    }

    pub fn set_active(&mut self, active: bool) {
        if active {
            self.flags |= 1;
        } else {
            self.flags &= !1;
        }
    }

    pub fn is_reduce_only(&self) -> bool {
        self.flags & 2 != 0
    }
}

impl Default for OrderSlot {
    fn default() -> Self {
        Self {
            price: 0,
            remaining_qty: 0,
            side: 0,
            flags: 0,
            tif: 0,
            _pad1: [0; 5],
            next: NONE,
            prev: NONE,
            tick_index: 0,
            _pad2: 0,
            user_id: 0,
            sequence: 0,
            _pad3: [0; 2],
            original_qty: 0,
            timestamp_ns: 0,
            _pad4: [0; 40],
        }
    }
}

impl SlabItem for OrderSlot {
    fn next(&self) -> u32 {
        self.next
    }

    fn set_next(&mut self, next: u32) {
        self.next = next;
    }
}
