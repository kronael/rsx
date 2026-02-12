use rsx_types::Side;
use rsx_types::TimeInForce;

/// Inbound order from risk engine via SPSC ring.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OrderMessage {
    pub seq: u64,
    pub price: i64,
    pub qty: i64,
    pub side: u8,
    pub tif: u8,
    pub reduce_only: u8,
    pub post_only: u8,
    pub _pad1: [u8; 4],
    pub user_id: u32,
    pub _pad2: u32,
    pub timestamp_ns: u64,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
}

impl OrderMessage {
    pub fn to_incoming(
        &self,
    ) -> rsx_book::matching::IncomingOrder {
        rsx_book::matching::IncomingOrder {
            price: self.price,
            qty: self.qty,
            remaining_qty: self.qty,
            side: if self.side == 0 {
                Side::Buy
            } else {
                Side::Sell
            },
            tif: match self.tif {
                1 => TimeInForce::IOC,
                2 => TimeInForce::FOK,
                _ => TimeInForce::GTC,
            },
            user_id: self.user_id,
            reduce_only: self.reduce_only != 0,
            post_only: self.post_only != 0,
            timestamp_ns: self.timestamp_ns,
            order_id_hi: self.order_id_hi,
            order_id_lo: self.order_id_lo,
        }
    }
}
