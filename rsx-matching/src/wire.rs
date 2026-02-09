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
    },
    OrderInserted {
        handle: u32,
        user_id: u32,
        side: u8,
        price: i64,
        qty: i64,
    },
    OrderCancelled {
        handle: u32,
        user_id: u32,
        remaining_qty: i64,
    },
    OrderDone {
        handle: u32,
        user_id: u32,
        reason: u8,
    },
    OrderFailed {
        user_id: u32,
        reason: u8,
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
            } => EventMessage::Fill {
                maker_handle,
                taker_user_id,
                price: price.0,
                qty: qty.0,
                side,
            },
            rsx_book::event::Event::OrderInserted {
                handle,
                user_id,
                side,
                price,
                qty,
            } => EventMessage::OrderInserted {
                handle,
                user_id,
                side,
                price: price.0,
                qty: qty.0,
            },
            rsx_book::event::Event::OrderCancelled {
                handle,
                user_id,
                remaining_qty,
            } => EventMessage::OrderCancelled {
                handle,
                user_id,
                remaining_qty: remaining_qty.0,
            },
            rsx_book::event::Event::OrderDone {
                handle,
                user_id,
                reason,
            } => EventMessage::OrderDone {
                handle,
                user_id,
                reason,
            },
            rsx_book::event::Event::OrderFailed {
                user_id,
                reason,
            } => EventMessage::OrderFailed {
                user_id,
                reason,
            },
        }
    }
}
