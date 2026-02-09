use rsx_types::NONE;

#[derive(Clone, Copy, Debug)]
pub struct PriceLevel {
    pub head: u32,
    pub tail: u32,
    pub total_qty: i64,
    pub order_count: u32,
}

const _: () = assert!(
    std::mem::size_of::<PriceLevel>() == 24
);

impl Default for PriceLevel {
    fn default() -> Self {
        Self {
            head: NONE,
            tail: NONE,
            total_qty: 0,
            order_count: 0,
        }
    }
}
