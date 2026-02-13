use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct NewOrder {
    pub symbol_id: u32,
    pub side: u8,
    pub price: i64,
    pub qty: i64,
    pub client_order_id: String,
    pub tif: u8,
    pub reduce_only: bool,
    pub post_only: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub enum OrderResponse {
    Update(OrderUpdate),
    Fill(Fill),
    Error(ErrorMessage),
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrderUpdate {
    pub oid: String,
    pub cid: String,
    pub status: u8,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Fill {
    pub oid: String,
    pub qty: i64,
    pub px: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorMessage {
    pub reason: u8,
    pub message: String,
}
