use crate::slab::SlabItem;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::NONE;

pub(crate) const ORDER_ACTIVE: u8 = 1 << 0;
pub(crate) const ORDER_REDUCE_ONLY: u8 = 1 << 1;

#[repr(C, align(64))]
pub struct OrderSlot {
    // Bytes 0..64: matching and lifecycle state.
    pub price: Price,
    pub remaining_qty: Qty,
    pub side: u8,
    pub flags: u8,
    pub tif: u8,
    pub _pad1: [u8; 5],
    pub next: u32,
    pub prev: u32,
    pub tick_index: u32,
    pub _pad2: u32,
    pub user_id: u32,
    pub sequence: u32,
    pub original_qty: Qty,
    pub timestamp_ns: u64,
    // Bytes 64..128: external identity and padding.
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub _pad4: [u8; 24],
}

const _: () = assert!(std::mem::size_of::<OrderSlot>() == 128);
const _: () = assert!(std::mem::align_of::<OrderSlot>() == 64);

impl OrderSlot {
    pub fn is_active(&self) -> bool {
        self.flags & ORDER_ACTIVE != 0
    }

    pub fn set_active(&mut self, active: bool) {
        if active {
            self.flags |= ORDER_ACTIVE;
        } else {
            self.flags &= !ORDER_ACTIVE;
        }
    }

    pub fn is_reduce_only(&self) -> bool {
        self.flags & ORDER_REDUCE_ONLY != 0
    }
}

impl Default for OrderSlot {
    fn default() -> Self {
        Self {
            price: Price(0),
            remaining_qty: Qty(0),
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
            original_qty: Qty(0),
            timestamp_ns: 0,
            order_id_hi: 0,
            order_id_lo: 0,
            _pad4: [0; 24],
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
