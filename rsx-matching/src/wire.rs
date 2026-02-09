use rsx_types::Side;
use rsx_types::TimeInForce;

/// Inbound order from risk engine via SPSC ring.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OrderMessage {
    pub price: i64,
    pub qty: i64,
    pub side: u8,
    pub tif: u8,
    pub reduce_only: u8,
    pub _pad1: [u8; 5],
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
            timestamp_ns: self.timestamp_ns,
            order_id_hi: self.order_id_hi,
            order_id_lo: self.order_id_lo,
        }
    }
}

/// Outbound event from ME via SPSC rings.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub enum EventMessage {
    Fill {
        maker_handle: u32,
        taker_user_id: u32,
        price: i64,
        qty: i64,
        side: u8,
        maker_order_id_hi: u64,
        maker_order_id_lo: u64,
        taker_order_id_hi: u64,
        taker_order_id_lo: u64,
    },
    OrderInserted {
        handle: u32,
        user_id: u32,
        side: u8,
        price: i64,
        qty: i64,
        order_id_hi: u64,
        order_id_lo: u64,
    },
    OrderCancelled {
        handle: u32,
        user_id: u32,
        remaining_qty: i64,
        order_id_hi: u64,
        order_id_lo: u64,
    },
    OrderDone {
        handle: u32,
        user_id: u32,
        reason: u8,
        filled_qty: i64,
        remaining_qty: i64,
        order_id_hi: u64,
        order_id_lo: u64,
    },
    OrderFailed {
        user_id: u32,
        reason: u8,
    },
    BBO {
        bid_px: i64,
        bid_qty: i64,
        ask_px: i64,
        ask_qty: i64,
    },
}

impl EventMessage {
    pub fn from_book_event(
        event: &rsx_book::event::Event,
    ) -> Self {
        match *event {
            rsx_book::event::Event::Fill {
                maker_handle,
                taker_user_id,
                price,
                qty,
                side,
                maker_order_id_hi,
                maker_order_id_lo,
                taker_order_id_hi,
                taker_order_id_lo,
            } => EventMessage::Fill {
                maker_handle,
                taker_user_id,
                price: price.0,
                qty: qty.0,
                side,
                maker_order_id_hi,
                maker_order_id_lo,
                taker_order_id_hi,
                taker_order_id_lo,
            },
            rsx_book::event::Event::OrderInserted {
                handle,
                user_id,
                side,
                price,
                qty,
                order_id_hi,
                order_id_lo,
            } => EventMessage::OrderInserted {
                handle,
                user_id,
                side,
                price: price.0,
                qty: qty.0,
                order_id_hi,
                order_id_lo,
            },
            rsx_book::event::Event::OrderCancelled {
                handle,
                user_id,
                remaining_qty,
                order_id_hi,
                order_id_lo,
            } => EventMessage::OrderCancelled {
                handle,
                user_id,
                remaining_qty: remaining_qty.0,
                order_id_hi,
                order_id_lo,
            },
            rsx_book::event::Event::OrderDone {
                handle,
                user_id,
                reason,
                filled_qty,
                remaining_qty,
                order_id_hi,
                order_id_lo,
            } => EventMessage::OrderDone {
                handle,
                user_id,
                reason,
                filled_qty: filled_qty.0,
                remaining_qty: remaining_qty.0,
                order_id_hi,
                order_id_lo,
            },
            rsx_book::event::Event::OrderFailed {
                user_id,
                reason,
            } => EventMessage::OrderFailed {
                user_id,
                reason,
            },
            rsx_book::event::Event::BBO {
                bid_px,
                bid_qty,
                ask_px,
                ask_qty,
            } => EventMessage::BBO {
                bid_px: bid_px.0,
                bid_qty: bid_qty.0,
                ask_px: ask_px.0,
                ask_qty: ask_qty.0,
            },
        }
    }
}
