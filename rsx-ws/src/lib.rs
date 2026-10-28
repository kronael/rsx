//! rsx-ws: the monoio WebSocket core of the gateway and marketdata. See ARCHITECTURE.md.

pub mod accept;
pub mod frame;
pub mod upgrade;

pub use accept::ws_accept_loop;
pub use frame::ws_read_frame_buf;
pub use frame::ws_write_binary;
pub use frame::ws_write_frame;
pub use frame::ws_write_text;
pub use upgrade::header_value;
pub use upgrade::is_ws_upgrade;
pub use upgrade::read_http_request;
pub use upgrade::write_upgrade_response;
pub use upgrade::ws_upgrade;
pub use upgrade::WS_UPGRADE_DEADLINE;
